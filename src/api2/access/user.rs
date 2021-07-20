//! User Management

use anyhow::{bail, format_err, Error};
use serde::{Serialize, Deserialize};
use serde_json::{json, Value};
use std::collections::HashMap;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment, Permission};
use proxmox::api::router::SubdirMap;
use proxmox::api::schema::{Schema, StringSchema};

use pbs_api_types::{
    PASSWORD_FORMAT, PROXMOX_CONFIG_DIGEST_SCHEMA, SINGLE_LINE_COMMENT_SCHEMA, Authid,
    Tokenname, UserWithTokens, Userid,
};

use crate::config::user;
use crate::config::token_shadow;
use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_PERMISSIONS_MODIFY};
use crate::config::cached_user_info::CachedUserInfo;
use crate::backup::open_backup_lockfile;

pub const PBS_PASSWORD_SCHEMA: Schema = StringSchema::new("User Password.")
    .format(&PASSWORD_FORMAT)
    .min_length(5)
    .max_length(64)
    .schema();

fn new_user_with_tokens(user: user::User) -> UserWithTokens {
    UserWithTokens {
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
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<UserWithTokens>, Error> {

    let (config, digest) = user::config()?;

    let auth_id: Authid = rpcenv
        .get_auth_id()
        .ok_or_else(|| format_err!("no authid available"))?
        .parse()?;

    let userid = auth_id.user();

    let user_info = CachedUserInfo::new()?;

    let top_level_privs = user_info.lookup_privs(&auth_id, &["access", "users"]);
    let top_level_allowed = (top_level_privs & PRIV_SYS_AUDIT) != 0;

    let filter_by_privs = |user: &user::User| {
        top_level_allowed || user.userid == *userid
    };


    let list:Vec<user::User> = config.convert_to_typed_array("user")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    let iter = list.into_iter().filter(filter_by_privs);
    let list = if include_tokens {
        let tokens: Vec<user::ApiToken> = config.convert_to_typed_array("token")?;
        let mut user_to_tokens = tokens
            .into_iter()
            .fold(
                HashMap::new(),
                |mut map: HashMap<Userid, Vec<user::ApiToken>>, token: user::ApiToken| {
                if token.tokenid.is_token() {
                    map
                        .entry(token.tokenid.user().clone())
                        .or_default()
                        .push(token);
                }
                map
            });
        iter
            .map(|user: user::User| {
                let mut user = new_user_with_tokens(user);
                user.tokens = user_to_tokens.remove(&user.userid).unwrap_or_default();
                user
            })
            .collect()
    } else {
        iter.map(new_user_with_tokens)
            .collect()
    };

    Ok(list)
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
pub fn create_user(
    password: Option<String>,
    param: Value,
    rpcenv: &mut dyn RpcEnvironment
) -> Result<(), Error> {

    let _lock = open_backup_lockfile(user::USER_CFG_LOCKFILE, None, true)?;

    let user: user::User = serde_json::from_value(param)?;

    let (mut config, _digest) = user::config()?;

    if config.sections.get(user.userid.as_str()).is_some() {
        bail!("user '{}' already exists.", user.userid);
    }

    config.set_data(user.userid.as_str(), "user", &user)?;

    let realm = user.userid.realm();

    // Fails if realm does not exist!
    let authenticator = crate::auth::lookup_authenticator(realm)?;

    user::save_config(&config)?;

    if let Some(password) = password {
        let user_info = CachedUserInfo::new()?;
        let current_auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        if realm == "pam" && !user_info.is_superuser(&current_auth_id) {
            bail!("only superuser can edit pam credentials!");
        }
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
    returns: { type: user::User },
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

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
#[allow(non_camel_case_types)]
pub enum DeletableProperty {
    /// Delete the comment property.
    comment,
    /// Delete the firstname property.
    firstname,
    /// Delete the lastname property.
    lastname,
    /// Delete the email property.
    email,
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
    comment: Option<String>,
    enable: Option<bool>,
    expire: Option<i64>,
    password: Option<String>,
    firstname: Option<String>,
    lastname: Option<String>,
    email: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let _lock = open_backup_lockfile(user::USER_CFG_LOCKFILE, None, true)?;

    let (mut config, expected_digest) = user::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: user::User = config.lookup("user", userid.as_str())?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::comment => data.comment = None,
                DeletableProperty::firstname => data.firstname = None,
                DeletableProperty::lastname => data.lastname = None,
                DeletableProperty::email => data.email = None,
            }
        }
    }

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
        let user_info = CachedUserInfo::new()?;
        let current_auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        let self_service = current_auth_id.user() == &userid;
        let target_realm = userid.realm();
        if !self_service && target_realm == "pam" && !user_info.is_superuser(&current_auth_id) {
            bail!("only superuser can edit pam credentials!");
        }
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

    let _tfa_lock = crate::config::tfa::write_lock()?;
    let _lock = open_backup_lockfile(user::USER_CFG_LOCKFILE, None, true)?;

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

    let authenticator = crate::auth::lookup_authenticator(userid.realm())?;
    match authenticator.remove_password(userid.name()) {
        Ok(()) => {},
        Err(err) => {
            eprintln!(
                "error removing password after deleting user {:?}: {}",
                userid, err
            );
        }
    }

    match crate::config::tfa::read().and_then(|mut cfg| {
        let _: bool = cfg.remove_user(&userid);
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
            tokenname: {
                type: Tokenname,
            },
        },
    },
    returns: { type: user::ApiToken },
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
    tokenname: Tokenname,
    _info: &ApiMethod,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<user::ApiToken, Error> {

    let (config, digest) = user::config()?;

    let tokenid = Authid::from((userid, Some(tokenname)));

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();
    config.lookup("token", &tokenid.to_string())
}

#[api(
    protected: true,
    input: {
        properties: {
            userid: {
                type: Userid,
            },
            tokenname: {
                type: Tokenname,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            enable: {
                schema: user::ENABLE_USER_SCHEMA,
                optional: true,
            },
            expire: {
                schema: user::EXPIRE_USER_SCHEMA,
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
    tokenname: Tokenname,
    comment: Option<String>,
    enable: Option<bool>,
    expire: Option<i64>,
    digest: Option<String>,
) -> Result<Value, Error> {

    let _lock = open_backup_lockfile(user::USER_CFG_LOCKFILE, None, true)?;

    let (mut config, expected_digest) = user::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let tokenid = Authid::from((userid.clone(), Some(tokenname.clone())));
    let tokenid_string = tokenid.to_string();

    if config.sections.get(&tokenid_string).is_some() {
        bail!("token '{}' for user '{}' already exists.", tokenname.as_str(), userid);
    }

    let secret = format!("{:x}", proxmox::tools::uuid::Uuid::generate());
    token_shadow::set_secret(&tokenid, &secret)?;

    let token = user::ApiToken {
        tokenid,
        comment,
        enable,
        expire,
    };

    config.set_data(&tokenid_string, "token", &token)?;

    user::save_config(&config)?;

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
            tokenname: {
                type: Tokenname,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            enable: {
                schema: user::ENABLE_USER_SCHEMA,
                optional: true,
            },
            expire: {
                schema: user::EXPIRE_USER_SCHEMA,
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
    tokenname: Tokenname,
    comment: Option<String>,
    enable: Option<bool>,
    expire: Option<i64>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = open_backup_lockfile(user::USER_CFG_LOCKFILE, None, true)?;

    let (mut config, expected_digest) = user::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let tokenid = Authid::from((userid, Some(tokenname)));
    let tokenid_string = tokenid.to_string();

    let mut data: user::ApiToken = config.lookup("token", &tokenid_string)?;

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
            tokenname: {
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
    tokenname: Tokenname,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = open_backup_lockfile(user::USER_CFG_LOCKFILE, None, true)?;

    let (mut config, expected_digest) = user::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let tokenid = Authid::from((userid.clone(), Some(tokenname.clone())));
    let tokenid_string = tokenid.to_string();

    match config.sections.get(&tokenid_string) {
        Some(_) => { config.sections.remove(&tokenid_string); },
        None => bail!("token '{}' of user '{}' does not exist.", tokenname.as_str(), userid),
    }

    token_shadow::delete_secret(&tokenid)?;

    user::save_config(&config)?;

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
        description: "List user's API tokens (with config digest).",
        type: Array,
        items: { type: user::ApiToken },
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
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<user::ApiToken>, Error> {

    let (config, digest) = user::config()?;

    let list:Vec<user::ApiToken> = config.convert_to_typed_array("token")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    let filter_by_owner = |token: &user::ApiToken| {
        if token.tokenid.is_token() {
           token.tokenid.user() == &userid
        } else {
            false
        }
    };

    Ok(list.into_iter().filter(filter_by_owner).collect())
}

const TOKEN_ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_TOKEN)
    .put(&API_METHOD_UPDATE_TOKEN)
    .post(&API_METHOD_GENERATE_TOKEN)
    .delete(&API_METHOD_DELETE_TOKEN);

const TOKEN_ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_TOKENS)
    .match_all("tokenname", &TOKEN_ITEM_ROUTER);

const USER_SUBDIRS: SubdirMap = &[
    ("token", &TOKEN_ROUTER),
];

const USER_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_USER)
    .put(&API_METHOD_UPDATE_USER)
    .delete(&API_METHOD_DELETE_USER)
    .subdirs(USER_SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_USERS)
    .post(&API_METHOD_CREATE_USER)
    .match_all("userid", &USER_ROUTER);
