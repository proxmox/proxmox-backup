//! User Management

use anyhow::{bail, format_err, Error};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

use proxmox_router::{ApiMethod, Permission, Router, RpcEnvironment, SubdirMap};
use proxmox_schema::api;
use proxmox_tfa::api::TfaConfig;

use pbs_api_types::{
    ApiToken, Authid, Tokenname, User, UserUpdater, UserWithTokens, Userid, ENABLE_USER_SCHEMA,
    EXPIRE_USER_SCHEMA, PBS_PASSWORD_SCHEMA, PRIV_PERMISSIONS_MODIFY, PRIV_SYS_AUDIT,
    PROXMOX_CONFIG_DIGEST_SCHEMA, SINGLE_LINE_COMMENT_SCHEMA,
};
use pbs_config::token_shadow;

use pbs_config::CachedUserInfo;

fn new_user_with_tokens(user: User, tfa: &TfaConfig) -> UserWithTokens {
    UserWithTokens {
        totp_locked: tfa
            .users
            .get(user.userid.as_str())
            .map(|data| data.totp_locked)
            .unwrap_or(false),
        tfa_locked_until: tfa
            .users
            .get(user.userid.as_str())
            .and_then(|data| data.tfa_locked_until),
        userid: user.userid,
        comment: user.comment,
        enable: user.enable,
        expire: user.expire,
        firstname: user.firstname,
        lastname: user.lastname,
        email: user.email,
        tokens: Vec::new(),
    }
}

#[api(
    protected: true,
    input: {
        properties: {
            include_tokens: {
                type: bool,
                description: "Include user's API tokens in returned list.",
                optional: true,
                default: false,
            },
        },
    },
    returns: {
        description: "List users (with config digest).",
        type: Array,
        items: { type: UserWithTokens },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Returns all or just the logged-in user (/API token owner), depending on privileges.",
    },
)]
/// List users
pub fn list_users(
    include_tokens: bool,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<UserWithTokens>, Error> {
    let (config, digest) = pbs_config::user::config()?;

    let auth_id: Authid = rpcenv
        .get_auth_id()
        .ok_or_else(|| format_err!("no authid available"))?
        .parse()?;

    let userid = auth_id.user();

    let user_info = CachedUserInfo::new()?;

    let top_level_privs = user_info.lookup_privs(&auth_id, &["access", "users"]);
    let top_level_allowed = (top_level_privs & PRIV_SYS_AUDIT) != 0;

    let filter_by_privs = |user: &User| top_level_allowed || user.userid == *userid;

    let list: Vec<User> = config.convert_to_typed_array("user")?;

    rpcenv["digest"] = hex::encode(digest).into();

    let tfa_data = crate::config::tfa::read()?;

    let iter = list.into_iter().filter(filter_by_privs);
    let list = if include_tokens {
        let tokens: Vec<ApiToken> = config.convert_to_typed_array("token")?;
        let mut user_to_tokens = tokens.into_iter().fold(
            HashMap::new(),
            |mut map: HashMap<Userid, Vec<ApiToken>>, token: ApiToken| {
                if token.tokenid.is_token() {
                    map.entry(token.tokenid.user().clone())
                        .or_default()
                        .push(token);
                }
                map
            },
        );
        iter.map(|user: User| {
            let mut user = new_user_with_tokens(user, &tfa_data);
            user.tokens = user_to_tokens.remove(&user.userid).unwrap_or_default();
            user
        })
        .collect()
    } else {
        iter.map(|user: User| new_user_with_tokens(user, &tfa_data))
            .collect()
    };

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: User,
                flatten: true,
            },
            password: {
                schema: PBS_PASSWORD_SCHEMA,
                optional: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
    },
)]
/// Create new user.
pub fn create_user(
    password: Option<String>,
    config: User,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = pbs_config::user::lock_config()?;

    let (mut section_config, _digest) = pbs_config::user::config()?;

    if section_config
        .sections
        .get(config.userid.as_str())
        .is_some()
    {
        bail!("user '{}' already exists.", config.userid);
    }

    section_config.set_data(config.userid.as_str(), "user", &config)?;

    let realm = config.userid.realm();

    // Fails if realm does not exist!
    let authenticator = crate::auth::lookup_authenticator(realm)?;

    pbs_config::user::save_config(&section_config)?;

    if let Some(password) = password {
        let user_info = CachedUserInfo::new()?;
        let current_auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        if realm == "pam" && !user_info.is_superuser(&current_auth_id) {
            bail!("only superuser can edit pam credentials!");
        }
        let client_ip = rpcenv.get_client_ip().map(|sa| sa.ip());
        authenticator.store_password(config.userid.name(), &password, client_ip.as_ref())?;
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
    returns: { type: User },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_SYS_AUDIT, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// Read user configuration data.
pub fn read_user(userid: Userid, rpcenv: &mut dyn RpcEnvironment) -> Result<User, Error> {
    let (config, digest) = pbs_config::user::config()?;
    let user = config.lookup("user", userid.as_str())?;
    rpcenv["digest"] = hex::encode(digest).into();
    Ok(user)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeletableProperty {
    /// Delete the comment property.
    Comment,
    /// Delete the firstname property.
    Firstname,
    /// Delete the lastname property.
    Lastname,
    /// Delete the email property.
    Email,
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: {
                type: Userid,
            },
            update: {
                type: UserUpdater,
                flatten: true,
            },
            password: {
                schema: PBS_PASSWORD_SCHEMA,
                optional: true,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeletableProperty,
                }
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
#[allow(clippy::too_many_arguments)]
pub fn update_user(
    userid: Userid,
    update: UserUpdater,
    password: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = pbs_config::user::lock_config()?;

    let (mut config, expected_digest) = pbs_config::user::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: User = config.lookup("user", userid.as_str())?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::Comment => data.comment = None,
                DeletableProperty::Firstname => data.firstname = None,
                DeletableProperty::Lastname => data.lastname = None,
                DeletableProperty::Email => data.email = None,
            }
        }
    }

    if let Some(comment) = update.comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }

    if let Some(enable) = update.enable {
        data.enable = if enable { None } else { Some(false) };
    }

    if let Some(expire) = update.expire {
        data.expire = if expire > 0 { Some(expire) } else { None };
    }

    if let Some(password) = password {
        let user_info = CachedUserInfo::new()?;
        let current_auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        let self_service = current_auth_id.user() == &userid;
        let target_realm = userid.realm();
        if !self_service && target_realm == "pam" && !user_info.is_superuser(&current_auth_id) {
            bail!("only superuser can edit pam credentials!");
        }
        let authenticator = crate::auth::lookup_authenticator(userid.realm())?;
        let client_ip = rpcenv.get_client_ip().map(|sa| sa.ip());
        authenticator.store_password(userid.name(), &password, client_ip.as_ref())?;
    }

    if let Some(firstname) = update.firstname {
        data.firstname = if firstname.is_empty() {
            None
        } else {
            Some(firstname)
        };
    }

    if let Some(lastname) = update.lastname {
        data.lastname = if lastname.is_empty() {
            None
        } else {
            Some(lastname)
        };
    }
    if let Some(email) = update.email {
        data.email = if email.is_empty() { None } else { Some(email) };
    }

    config.set_data(userid.as_str(), "user", &data)?;

    pbs_config::user::save_config(&config)?;

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
    let _lock = pbs_config::user::lock_config()?;
    let _tfa_lock = crate::config::tfa::write_lock()?;

    let (mut config, expected_digest) = pbs_config::user::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(userid.as_str()) {
        Some(_) => {
            config.sections.remove(userid.as_str());
        }
        None => bail!("user '{}' does not exist.", userid),
    }

    pbs_config::user::save_config(&config)?;

    let authenticator = crate::auth::lookup_authenticator(userid.realm())?;
    match authenticator.remove_password(userid.name()) {
        Ok(()) => {}
        Err(err) => {
            eprintln!(
                "error removing password after deleting user {:?}: {}",
                userid, err
            );
        }
    }

    match crate::config::tfa::read().and_then(|mut cfg| {
        let _: proxmox_tfa::api::NeedsSaving =
            cfg.remove_user(&crate::config::tfa::UserAccess, userid.as_str())?;
        crate::config::tfa::write(&cfg)
    }) {
        Ok(()) => (),
        Err(err) => {
            eprintln!(
                "error updating TFA config after deleting user {:?}: {}",
                userid, err
            );
        }
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            userid: {
                type: Userid,
            },
            "token-name": {
                type: Tokenname,
            },
        },
    },
    returns: { type: ApiToken },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_SYS_AUDIT, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// Read user's API token metadata
pub fn read_token(
    userid: Userid,
    token_name: Tokenname,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<ApiToken, Error> {
    let (config, digest) = pbs_config::user::config()?;

    let tokenid = Authid::from((userid, Some(token_name)));

    rpcenv["digest"] = hex::encode(digest).into();
    config.lookup("token", &tokenid.to_string())
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: {
                type: Userid,
            },
            "token-name": {
                type: Tokenname,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            enable: {
                schema: ENABLE_USER_SCHEMA,
                optional: true,
            },
            expire: {
                schema: EXPIRE_USER_SCHEMA,
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
    returns: {
        description: "API token identifier + generated secret.",
        properties: {
            value: {
                type: String,
                description: "The API token secret",
            },
            tokenid: {
                type: String,
                description: "The API token identifier",
            },
        },
    },
)]
/// Generate a new API token with given metadata
pub fn generate_token(
    userid: Userid,
    token_name: Tokenname,
    comment: Option<String>,
    enable: Option<bool>,
    expire: Option<i64>,
    digest: Option<String>,
) -> Result<Value, Error> {
    let _lock = pbs_config::user::lock_config()?;

    let (mut config, expected_digest) = pbs_config::user::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let tokenid = Authid::from((userid.clone(), Some(token_name.clone())));
    let tokenid_string = tokenid.to_string();

    if config.sections.get(&tokenid_string).is_some() {
        bail!(
            "token '{}' for user '{}' already exists.",
            token_name.as_str(),
            userid
        );
    }

    let secret = format!("{:x}", proxmox_uuid::Uuid::generate());
    token_shadow::set_secret(&tokenid, &secret)?;

    let token = ApiToken {
        tokenid,
        comment,
        enable,
        expire,
    };

    config.set_data(&tokenid_string, "token", &token)?;

    pbs_config::user::save_config(&config)?;

    Ok(json!({
        "tokenid": tokenid_string,
        "value": secret
    }))
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: {
                type: Userid,
            },
            "token-name": {
                type: Tokenname,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            enable: {
                schema: ENABLE_USER_SCHEMA,
                optional: true,
            },
            expire: {
                schema: EXPIRE_USER_SCHEMA,
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
/// Update user's API token metadata
pub fn update_token(
    userid: Userid,
    token_name: Tokenname,
    comment: Option<String>,
    enable: Option<bool>,
    expire: Option<i64>,
    digest: Option<String>,
) -> Result<(), Error> {
    let _lock = pbs_config::user::lock_config()?;

    let (mut config, expected_digest) = pbs_config::user::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let tokenid = Authid::from((userid, Some(token_name)));
    let tokenid_string = tokenid.to_string();

    let mut data: ApiToken = config.lookup("token", &tokenid_string)?;

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

    config.set_data(&tokenid_string, "token", &data)?;

    pbs_config::user::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: {
                type: Userid,
            },
            "token-name": {
                type: Tokenname,
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
/// Delete a user's API token
pub fn delete_token(
    userid: Userid,
    token_name: Tokenname,
    digest: Option<String>,
) -> Result<(), Error> {
    let _lock = pbs_config::user::lock_config()?;

    let (mut config, expected_digest) = pbs_config::user::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let tokenid = Authid::from((userid.clone(), Some(token_name.clone())));
    let tokenid_string = tokenid.to_string();

    match config.sections.get(&tokenid_string) {
        Some(_) => {
            config.sections.remove(&tokenid_string);
        }
        None => bail!(
            "token '{}' of user '{}' does not exist.",
            token_name.as_str(),
            userid
        ),
    }

    token_shadow::delete_secret(&tokenid)?;

    pbs_config::user::save_config(&config)?;

    Ok(())
}

#[api(
    properties: {
        "token-name": { type: Tokenname },
        token: { type: ApiToken },
    }
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// A Token Entry that contains the token-name
pub struct TokenApiEntry {
    /// The Token name
    pub token_name: Tokenname,
    #[serde(flatten)]
    pub token: ApiToken,
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
        description: "List user's API tokens (with config digest).",
        type: Array,
        items: { type: TokenApiEntry },
    },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_SYS_AUDIT, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// List user's API tokens
pub fn list_tokens(
    userid: Userid,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TokenApiEntry>, Error> {
    let (config, digest) = pbs_config::user::config()?;

    let list: Vec<ApiToken> = config.convert_to_typed_array("token")?;

    rpcenv["digest"] = hex::encode(digest).into();

    let filter_by_owner = |token: ApiToken| {
        if token.tokenid.is_token() && token.tokenid.user() == &userid {
            let token_name = token.tokenid.tokenname().unwrap().to_owned();
            Some(TokenApiEntry { token_name, token })
        } else {
            None
        }
    };

    let res = list.into_iter().filter_map(filter_by_owner).collect();

    Ok(res)
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: {
                type: Userid,
            },
        },
    },
    returns: {
        type: bool,
        description: "Whether the user was previously locked out of any 2nd factor.",
    },
    access: {
        permission: &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
    },
)]
/// Unlock a user's TFA authentication.
pub fn unlock_tfa(userid: Userid) -> Result<bool, Error> {
    let _lock = crate::config::tfa::write_lock()?;

    let mut config = crate::config::tfa::read()?;
    if proxmox_tfa::api::methods::unlock_and_reset_tfa(
        &mut config,
        &crate::config::tfa::UserAccess,
        userid.as_str(),
    )? {
        crate::config::tfa::write(&config)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

const TOKEN_ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_TOKEN)
    .put(&API_METHOD_UPDATE_TOKEN)
    .post(&API_METHOD_GENERATE_TOKEN)
    .delete(&API_METHOD_DELETE_TOKEN);

const TOKEN_ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_TOKENS)
    .match_all("token-name", &TOKEN_ITEM_ROUTER);

const UNLOCK_TFA_ROUTER: Router = Router::new().put(&API_METHOD_UNLOCK_TFA);

const USER_SUBDIRS: SubdirMap = &[("token", &TOKEN_ROUTER), ("unlock-tfa", &UNLOCK_TFA_ROUTER)];

const USER_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_USER)
    .put(&API_METHOD_UPDATE_USER)
    .delete(&API_METHOD_DELETE_USER)
    .subdirs(USER_SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_USERS)
    .post(&API_METHOD_CREATE_USER)
    .match_all("userid", &USER_ROUTER);
