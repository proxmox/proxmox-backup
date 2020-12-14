use anyhow::{bail, format_err, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox::api::{api, Permission, Router, RpcEnvironment};
use proxmox::tools::tfa::totp::Totp;
use proxmox::{http_bail, http_err};

use crate::api2::types::{Authid, Userid, PASSWORD_SCHEMA};
use crate::config::acl::{PRIV_PERMISSIONS_MODIFY, PRIV_SYS_AUDIT};
use crate::config::cached_user_info::CachedUserInfo;
use crate::config::tfa::{TfaInfo, TfaUserData};

/// Perform first-factor (password) authentication only. Ignore password for the root user.
/// Otherwise check the current user's password.
///
/// This means that user admins need to type in their own password while editing a user, and
/// regular users, which can only change their own TFA settings (checked at the API level), can
/// change their own settings using their own password.
fn tfa_update_auth(
    rpcenv: &mut dyn RpcEnvironment,
    userid: &Userid,
    password: Option<String>,
) -> Result<(), Error> {
    let authid: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    if authid.user() != Userid::root_userid() {
        let password = password.ok_or_else(|| format_err!("missing password"))?;
        let _: () = crate::auth::authenticate_user(authid.user(), &password)?;
    }

    // After authentication, verify that the to-be-modified user actually exists:
    if authid.user() != userid {
        let (config, _digest) = crate::config::user::config()?;

        if config.sections.get(userid.as_str()).is_none() {
            bail!("user '{}' does not exists.", userid);
        }
    }

    Ok(())
}

#[api]
/// A TFA entry type.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TfaType {
    /// A TOTP entry type.
    Totp,
    /// A U2F token entry.
    U2f,
    /// A Webauthn token entry.
    Webauthn,
    /// Recovery tokens.
    Recovery,
}

#[api(
    properties: {
        type: { type: TfaType },
        info: { type: TfaInfo },
    },
)]
/// A TFA entry for a user.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TypedTfaInfo {
    #[serde(rename = "type")]
    pub ty: TfaType,

    #[serde(flatten)]
    pub info: TfaInfo,
}

fn to_data(data: TfaUserData) -> Vec<TypedTfaInfo> {
    let mut out = Vec::with_capacity(
        data.totp.len()
            + data.u2f.len()
            + data.webauthn.len()
            + if data.has_recovery() { 1 } else { 0 },
    );
    if data.has_recovery() {
        out.push(TypedTfaInfo {
            ty: TfaType::Recovery,
            info: TfaInfo::recovery(),
        })
    }
    for entry in data.totp {
        out.push(TypedTfaInfo {
            ty: TfaType::Totp,
            info: entry.info,
        });
    }
    for entry in data.webauthn {
        out.push(TypedTfaInfo {
            ty: TfaType::Webauthn,
            info: entry.info,
        });
    }
    for entry in data.u2f {
        out.push(TypedTfaInfo {
            ty: TfaType::U2f,
            info: entry.info,
        });
    }
    out
}

#[api(
    protected: true,
    input: {
        properties: { userid: { type: Userid } },
    },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// Add a TOTP secret to the user.
pub fn list_user_tfa(userid: Userid) -> Result<Vec<TypedTfaInfo>, Error> {
    let _lock = crate::config::tfa::read_lock()?;

    Ok(match crate::config::tfa::read()?.users.remove(&userid) {
        Some(data) => to_data(data),
        None => Vec::new(),
    })
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
pub fn get_tfa_entry(userid: Userid, id: String) -> Result<TypedTfaInfo, Error> {
    let _lock = crate::config::tfa::read_lock()?;

    if let Some(user_data) = crate::config::tfa::read()?.users.remove(&userid) {
        if id == "recovery" {
            if user_data.has_recovery() {
                return Ok(TypedTfaInfo {
                    ty: TfaType::Recovery,
                    info: TfaInfo::recovery(),
                });
            }
        } else {
            for tfa in user_data.totp {
                if tfa.info.id == id {
                    return Ok(TypedTfaInfo {
                        ty: TfaType::Totp,
                        info: tfa.info,
                    });
                }
            }

            for tfa in user_data.webauthn {
                if tfa.info.id == id {
                    return Ok(TypedTfaInfo {
                        ty: TfaType::Webauthn,
                        info: tfa.info,
                    });
                }
            }

            for tfa in user_data.u2f {
                if tfa.info.id == id {
                    return Ok(TypedTfaInfo {
                        ty: TfaType::U2f,
                        info: tfa.info,
                    });
                }
            }
        }
    }

    http_bail!(NOT_FOUND, "no such tfa entry: {}/{}", userid, id);
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
/// Get a single TFA entry.
pub fn delete_tfa(
    userid: Userid,
    id: String,
    password: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    tfa_update_auth(rpcenv, &userid, password)?;

    let _lock = crate::config::tfa::write_lock()?;

    let mut data = crate::config::tfa::read()?;

    let user_data = data
        .users
        .get_mut(&userid)
        .ok_or_else(|| http_err!(NOT_FOUND, "no such entry: {}/{}", userid, id))?;

    let found = if id == "recovery" {
        let found = user_data.has_recovery();
        user_data.recovery = None;
        found
    } else if let Some(i) = user_data.totp.iter().position(|entry| entry.info.id == id) {
        user_data.totp.remove(i);
        true
    } else if let Some(i) = user_data
        .webauthn
        .iter()
        .position(|entry| entry.info.id == id)
    {
        user_data.webauthn.remove(i);
        true
    } else if let Some(i) = user_data.u2f.iter().position(|entry| entry.info.id == id) {
        user_data.u2f.remove(i);
        true
    } else {
        false
    };

    if !found {
        http_bail!(NOT_FOUND, "no such tfa entry: {}/{}", userid, id);
    }

    if user_data.is_empty() {
        data.users.remove(&userid);
    }

    crate::config::tfa::write(&data)?;

    Ok(())
}

#[api(
    properties: {
        "userid": { type: Userid },
        "entries": {
            type: Array,
            items: { type: TypedTfaInfo },
        },
    },
)]
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// Over the API we only provide the descriptions for TFA data.
pub struct TfaUser {
    /// The user this entry belongs to.
    userid: Userid,

    /// TFA entries.
    entries: Vec<TypedTfaInfo>,
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
)]
/// List user TFA configuration.
pub fn list_tfa(rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let authid: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let top_level_privs = user_info.lookup_privs(&authid, &["access", "users"]);
    let top_level_allowed = (top_level_privs & PRIV_SYS_AUDIT) != 0;

    let _lock = crate::config::tfa::read_lock()?;
    let tfa_data = crate::config::tfa::read()?.users;

    let mut out = Vec::<TfaUser>::new();
    if top_level_allowed {
        for (user, data) in tfa_data {
            out.push(TfaUser {
                userid: user,
                entries: to_data(data),
            });
        }
    } else {
        if let Some(data) = { tfa_data }.remove(authid.user()) {
            out.push(TfaUser {
                userid: authid.into(),
                entries: to_data(data),
            });
        }
    }

    Ok(serde_json::to_value(out)?)
}

#[api(
    properties: {
        recovery: {
            description: "A list of recovery codes as integers.",
            type: Array,
            items: {
                type: Integer,
                description: "A one-time usable recovery code entry.",
            },
        },
    },
)]
/// The result returned when adding TFA entries to a user.
#[derive(Default, Serialize)]
struct TfaUpdateInfo {
    /// The id if a newly added TFA entry.
    id: Option<String>,

    /// When adding u2f entries, this contains a challenge the user must respond to in order to
    /// finish the registration.
    #[serde(skip_serializing_if = "Option::is_none")]
    challenge: Option<String>,

    /// When adding recovery codes, this contains the list of codes to be displayed to the user
    /// this one time.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    recovery: Vec<String>,
}

impl TfaUpdateInfo {
    fn id(id: String) -> Self {
        Self {
            id: Some(id),
            ..Default::default()
        }
    }
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
            "type": { type: TfaType },
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
    returns: { type: TfaUpdateInfo },
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
            &Permission::UserParam("userid"),
        ]),
    },
)]
/// Add a TFA entry to the user.
fn add_tfa_entry(
    userid: Userid,
    description: Option<String>,
    totp: Option<String>,
    value: Option<String>,
    challenge: Option<String>,
    password: Option<String>,
    mut params: Value, // FIXME: once api macro supports raw parameters names, use `r#type`
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<TfaUpdateInfo, Error> {
    tfa_update_auth(rpcenv, &userid, password)?;

    let tfa_type: TfaType = serde_json::from_value(params["type"].take())?;

    let need_description =
        move || description.ok_or_else(|| format_err!("'description' is required for new entries"));

    match tfa_type {
        TfaType::Totp => match (totp, value) {
            (Some(totp), Some(value)) => {
                if challenge.is_some() {
                    bail!("'challenge' parameter is invalid for 'totp' entries");
                }
                let description = need_description()?;

                let totp: Totp = totp.parse()?;
                if totp
                    .verify(&value, std::time::SystemTime::now(), -1..=1)?
                    .is_none()
                {
                    bail!("failed to verify TOTP challenge");
                }
                crate::config::tfa::add_totp(&userid, description, totp).map(TfaUpdateInfo::id)
            }
            _ => bail!("'totp' type requires both 'totp' and 'value' parameters"),
        },
        TfaType::Webauthn => {
            if totp.is_some() {
                bail!("'totp' parameter is invalid for 'totp' entries");
            }

            match challenge {
                None => crate::config::tfa::add_webauthn_registration(&userid, need_description()?)
                    .map(|c| TfaUpdateInfo {
                        challenge: Some(c),
                        ..Default::default()
                    }),
                Some(challenge) => {
                    let value = value.ok_or_else(|| {
                        format_err!(
                            "missing 'value' parameter (webauthn challenge response missing)"
                        )
                    })?;
                    crate::config::tfa::finish_webauthn_registration(&userid, &challenge, &value)
                        .map(TfaUpdateInfo::id)
                }
            }
        }
        TfaType::U2f => {
            if totp.is_some() {
                bail!("'totp' parameter is invalid for 'totp' entries");
            }

            match challenge {
                None => crate::config::tfa::add_u2f_registration(&userid, need_description()?).map(
                    |c| TfaUpdateInfo {
                        challenge: Some(c),
                        ..Default::default()
                    },
                ),
                Some(challenge) => {
                    let value = value.ok_or_else(|| {
                        format_err!("missing 'value' parameter (u2f challenge response missing)")
                    })?;
                    crate::config::tfa::finish_u2f_registration(&userid, &challenge, &value)
                        .map(TfaUpdateInfo::id)
                }
            }
        }
        TfaType::Recovery => {
            if totp.or(value).or(challenge).is_some() {
                bail!("generating recovery tokens does not allow additional parameters");
            }

            let recovery = crate::config::tfa::add_recovery(&userid)?;

            Ok(TfaUpdateInfo {
                id: Some("recovery".to_string()),
                recovery,
                ..Default::default()
            })
        }
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
pub fn update_tfa_entry(
    userid: Userid,
    id: String,
    description: Option<String>,
    enable: Option<bool>,
    password: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    tfa_update_auth(rpcenv, &userid, password)?;

    let _lock = crate::config::tfa::write_lock()?;

    let mut data = crate::config::tfa::read()?;

    let mut entry = data
        .users
        .get_mut(&userid)
        .and_then(|user| user.find_entry_mut(&id))
        .ok_or_else(|| http_err!(NOT_FOUND, "no such entry: {}/{}", userid, id))?;

    if let Some(description) = description {
        entry.description = description;
    }

    if let Some(enable) = enable {
        entry.enable = enable;
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
