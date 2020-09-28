use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment, Permission};
use proxmox::api::schema::{Schema, StringSchema};
use proxmox::tools::fs::open_file_locked;

use crate::api2::types::*;
use crate::config::user;
use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_PERMISSIONS_MODIFY};
use crate::config::cached_user_info::CachedUserInfo;

pub const PBS_PASSWORD_SCHEMA: Schema = StringSchema::new("User Password.")
    .format(&PASSWORD_FORMAT)
    .min_length(5)
    .max_length(64)
    .schema();

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List users (with config digest).",
        type: Array,
        items: { type: user::User },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Returns all or just the logged-in user, depending on privileges.",
    },
)]
/// List users
pub fn list_users(
    _param: Value,
    _info: &ApiMethod,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<user::User>, Error> {

    let (config, digest) = user::config()?;

    let userid: Userid = rpcenv.get_user().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let top_level_privs = user_info.lookup_privs(&userid, &["access", "users"]);
    let top_level_allowed = (top_level_privs & PRIV_SYS_AUDIT) != 0;

    let filter_by_privs = |user: &user::User| {
        top_level_allowed || user.userid == userid
    };

    let list:Vec<user::User> = config.convert_to_typed_array("user")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list.into_iter().filter(filter_by_privs).collect())
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: {
                type: Userid,
            },
            comment: {
                schema: SINGLE_LINE_COMMENT_SCHEMA,
                optional: true,
            },
            password: {
                schema: PBS_PASSWORD_SCHEMA,
                optional: true,
            },
            enable: {
                schema: user::ENABLE_USER_SCHEMA,
                optional: true,
            },
            expire: {
                schema: user::EXPIRE_USER_SCHEMA,
                optional: true,
            },
            firstname: {
                schema: user::FIRST_NAME_SCHEMA,
                optional: true,
            },
            lastname: {
                schema: user::LAST_NAME_SCHEMA,
                optional: true,
            },
            email: {
                schema: user::EMAIL_SCHEMA,
                optional: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
    },
)]
/// Create new user.
pub fn create_user(password: Option<String>, param: Value) -> Result<(), Error> {

    let _lock = open_file_locked(user::USER_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let user: user::User = serde_json::from_value(param)?;

    let (mut config, _digest) = user::config()?;

    if let Some(_) = config.sections.get(user.userid.as_str()) {
        bail!("user '{}' already exists.", user.userid);
    }

    let authenticator = crate::auth::lookup_authenticator(&user.userid.realm())?;

    config.set_data(user.userid.as_str(), "user", &user)?;

    user::save_config(&config)?;

    if let Some(password) = password {
        authenticator.store_password(user.userid.name(), &password)?;
    }

    Ok(())
}

#[api(
   input: {
        properties: {
            userid: {
                type: Userid,
            },
         },
    },
    returns: {
        description: "The user configuration (with config digest).",
        type: user::User,
    },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_SYS_AUDIT, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// Read user configuration data.
pub fn read_user(userid: Userid, mut rpcenv: &mut dyn RpcEnvironment) -> Result<user::User, Error> {
    let (config, digest) = user::config()?;
    let user = config.lookup("user", userid.as_str())?;
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();
    Ok(user)
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: {
                type: Userid,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            password: {
                schema: PBS_PASSWORD_SCHEMA,
                optional: true,
            },
            enable: {
                schema: user::ENABLE_USER_SCHEMA,
                optional: true,
            },
            expire: {
                schema: user::EXPIRE_USER_SCHEMA,
                optional: true,
            },
            firstname: {
                schema: user::FIRST_NAME_SCHEMA,
                optional: true,
            },
            lastname: {
                schema: user::LAST_NAME_SCHEMA,
                optional: true,
            },
            email: {
                schema: user::EMAIL_SCHEMA,
                optional: true,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
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
/// Update user configuration.
pub fn update_user(
    userid: Userid,
    comment: Option<String>,
    enable: Option<bool>,
    expire: Option<i64>,
    password: Option<String>,
    firstname: Option<String>,
    lastname: Option<String>,
    email: Option<String>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = open_file_locked(user::USER_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, expected_digest) = user::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: user::User = config.lookup("user", userid.as_str())?;

    if let Some(comment) = comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }

    if let Some(enable) = enable {
        data.enable = if enable { None } else { Some(false) };
    }

    if let Some(expire) = expire {
        data.expire = if expire > 0 { Some(expire) } else { None };
    }

    if let Some(password) = password {
        let authenticator = crate::auth::lookup_authenticator(userid.realm())?;
        authenticator.store_password(userid.name(), &password)?;
    }

    if let Some(firstname) = firstname {
        data.firstname = if firstname.is_empty() { None } else { Some(firstname) };
    }

    if let Some(lastname) = lastname {
        data.lastname = if lastname.is_empty() { None } else { Some(lastname) };
    }
    if let Some(email) = email {
        data.email = if email.is_empty() { None } else { Some(email) };
    }

    config.set_data(userid.as_str(), "user", &data)?;

    user::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: {
                type: Userid,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
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
/// Remove a user from the configuration file.
pub fn delete_user(userid: Userid, digest: Option<String>) -> Result<(), Error> {

    let _lock = open_file_locked(user::USER_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, expected_digest) = user::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(userid.as_str()) {
        Some(_) => { config.sections.remove(userid.as_str()); },
        None => bail!("user '{}' does not exist.", userid),
    }

    user::save_config(&config)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_USER)
    .put(&API_METHOD_UPDATE_USER)
    .delete(&API_METHOD_DELETE_USER);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_USERS)
    .post(&API_METHOD_CREATE_USER)
    .match_all("userid", &ITEM_ROUTER);
