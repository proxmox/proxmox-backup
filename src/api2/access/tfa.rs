//! Two Factor Authentication

use anyhow::Error;

use proxmox_router::{http_bail, http_err, Permission, Router, RpcEnvironment};
use proxmox_schema::api;
use proxmox_tfa::api::methods;

use pbs_api_types::{
    Authid, User, Userid, PASSWORD_SCHEMA, PRIV_PERMISSIONS_MODIFY, PRIV_SYS_AUDIT,
};
use pbs_config::CachedUserInfo;

use crate::config::tfa::UserAccess;

/// Perform first-factor (password) authentication only. Ignore password for the root user.
/// Otherwise check the current user's password.
///
/// This means that user admins need to type in their own password while editing a user, and
/// regular users, which can only change their own TFA settings (checked at the API level), can
/// change their own settings using their own password.
async fn tfa_update_auth(
    rpcenv: &mut dyn RpcEnvironment,
    userid: &Userid,
    password: Option<String>,
    must_exist: bool,
) -> Result<(), Error> {
    let authid: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    if authid.user() != Userid::root_userid() {
        let client_ip = rpcenv.get_client_ip().map(|sa| sa.ip());
        let password = password.ok_or_else(|| http_err!(UNAUTHORIZED, "missing password"))?;
        #[allow(clippy::let_unit_value)]
        {
            let _: () =
                crate::auth::authenticate_user(authid.user(), &password, client_ip.as_ref())
                    .await
                    .map_err(|err| http_err!(UNAUTHORIZED, "{}", err))?;
        }
    }

    // After authentication, verify that the to-be-modified user actually exists:
    if must_exist && authid.user() != userid {
        let (config, _digest) = pbs_config::user::config()?;

        if config.lookup::<User>("user", userid.as_str()).is_err() {
            http_bail!(UNAUTHORIZED, "user '{}' does not exists.", userid);
        }
    }

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: { userid: { type: Userid } },
    },
    returns: {
        description: "The list of TFA entries.",
        type: Array,
        items: { type: methods::TypedTfaInfo }
    },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// Add a TOTP secret to the user.
pub fn list_user_tfa(userid: Userid) -> Result<Vec<methods::TypedTfaInfo>, Error> {
    let _lock = crate::config::tfa::read_lock()?;

    methods::list_user_tfa(&crate::config::tfa::read()?, userid.as_str())
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: { type: Userid },
            id: { description: "the tfa entry id" }
        },
    },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// Get a single TFA entry.
fn get_tfa_entry(userid: Userid, id: String) -> Result<methods::TypedTfaInfo, Error> {
    let _lock = crate::config::tfa::read_lock()?;

    match methods::get_tfa_entry(&crate::config::tfa::read()?, userid.as_str(), &id) {
        Some(entry) => Ok(entry),
        None => http_bail!(NOT_FOUND, "no such tfa entry: {}/{}", userid, id),
    }
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: { type: Userid },
            id: {
                description: "the tfa entry id",
            },
            password: {
                schema: PASSWORD_SCHEMA,
                optional: true,
            },
        },
    },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// Delete a single TFA entry.
pub async fn delete_tfa(
    userid: Userid,
    id: String,
    password: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    tfa_update_auth(rpcenv, &userid, password, false).await?;

    let _lock = crate::config::tfa::write_lock()?;

    let mut data = crate::config::tfa::read()?;

    match methods::delete_tfa(&mut data, userid.as_str(), &id) {
        Ok(_) => (),
        Err(methods::EntryNotFound) => {
            http_bail!(NOT_FOUND, "no such tfa entry: {}/{}", userid, id)
        }
    }

    crate::config::tfa::write(&data)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {},
    },
    access: {
        permission: &Permission::Anybody,
        description: "Returns all or just the logged-in user, depending on privileges.",
    },
    returns: {
        description: "The list tuples of user and TFA entries.",
        type: Array,
        items: { type: methods::TfaUser }
    },
)]
/// List user TFA configuration.
fn list_tfa(rpcenv: &mut dyn RpcEnvironment) -> Result<Vec<methods::TfaUser>, Error> {
    let authid: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let top_level_privs = user_info.lookup_privs(&authid, &["access", "users"]);
    let top_level_allowed = (top_level_privs & PRIV_SYS_AUDIT) != 0;

    let _lock = crate::config::tfa::read_lock()?;
    let tfa_data = crate::config::tfa::read()?;
    methods::list_tfa(&tfa_data, authid.user().as_str(), top_level_allowed)
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: { type: Userid },
            description: {
                description: "A description to distinguish multiple entries from one another",
                type: String,
                max_length: 255,
                optional: true,
            },
            "type": { type: methods::TfaType },
            totp: {
                description: "A totp URI.",
                optional: true,
            },
            value: {
                description:
            "The current value for the provided totp URI, or a Webauthn/U2F challenge response",
                optional: true,
            },
            challenge: {
                description: "When responding to a u2f challenge: the original challenge string",
                optional: true,
            },
            password: {
                schema: PASSWORD_SCHEMA,
                optional: true,
            },
        },
    },
    returns: { type: methods::TfaUpdateInfo },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// Add a TFA entry to the user.
#[allow(clippy::too_many_arguments)]
async fn add_tfa_entry(
    userid: Userid,
    description: Option<String>,
    totp: Option<String>,
    value: Option<String>,
    challenge: Option<String>,
    password: Option<String>,
    r#type: methods::TfaType,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<methods::TfaUpdateInfo, Error> {
    tfa_update_auth(rpcenv, &userid, password, true).await?;

    let _lock = crate::config::tfa::write_lock()?;

    let mut data = crate::config::tfa::read()?;
    let out = methods::add_tfa_entry(
        &mut data,
        &UserAccess,
        userid.as_str(),
        description,
        totp,
        value,
        challenge,
        r#type,
        None,
    )?;
    crate::config::tfa::write(&data)?;
    Ok(out)
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: { type: Userid },
            id: {
                description: "the tfa entry id",
            },
            description: {
                description: "A description to distinguish multiple entries from one another",
                type: String,
                max_length: 255,
                optional: true,
            },
            enable: {
                description: "Whether this entry should currently be enabled or disabled",
                optional: true,
            },
            password: {
                schema: PASSWORD_SCHEMA,
                optional: true,
            },
        },
    },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// Update user's TFA entry description.
async fn update_tfa_entry(
    userid: Userid,
    id: String,
    description: Option<String>,
    enable: Option<bool>,
    password: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    tfa_update_auth(rpcenv, &userid, password, true).await?;

    let _lock = crate::config::tfa::write_lock()?;

    let mut data = crate::config::tfa::read()?;
    match methods::update_tfa_entry(&mut data, userid.as_str(), &id, description, enable) {
        Ok(()) => (),
        Err(methods::EntryNotFound) => http_bail!(NOT_FOUND, "no such entry: {}/{}", userid, id),
    }
    crate::config::tfa::write(&data)?;
    Ok(())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_TFA)
    .match_all("userid", &USER_ROUTER);

const USER_ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_USER_TFA)
    .post(&API_METHOD_ADD_TFA_ENTRY)
    .match_all("id", &ITEM_ROUTER);

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_TFA_ENTRY)
    .put(&API_METHOD_UPDATE_TFA_ENTRY)
    .delete(&API_METHOD_DELETE_TFA);
