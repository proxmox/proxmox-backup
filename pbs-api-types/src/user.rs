use serde::{Deserialize, Serialize};

use proxmox_schema::{api, BooleanSchema, IntegerSchema, Schema, StringSchema, Updater};

use super::userid::{Authid, Userid, PROXMOX_TOKEN_ID_SCHEMA};
use super::{SINGLE_LINE_COMMENT_FORMAT, SINGLE_LINE_COMMENT_SCHEMA};

pub const ENABLE_USER_SCHEMA: Schema = BooleanSchema::new(
    "Enable the account (default). You can set this to '0' to disable the account.",
)
.default(true)
.schema();

pub const EXPIRE_USER_SCHEMA: Schema = IntegerSchema::new(
    "Account expiration date (seconds since epoch). '0' means no expiration date.",
)
.default(0)
.minimum(0)
.schema();

pub const FIRST_NAME_SCHEMA: Schema = StringSchema::new("First name.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .min_length(2)
    .max_length(64)
    .schema();

pub const LAST_NAME_SCHEMA: Schema = StringSchema::new("Last name.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .min_length(2)
    .max_length(64)
    .schema();

pub const EMAIL_SCHEMA: Schema = StringSchema::new("E-Mail Address.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .min_length(2)
    .max_length(64)
    .schema();

#[api(
    properties: {
        userid: {
            type: Userid,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        enable: {
            optional: true,
            schema: ENABLE_USER_SCHEMA,
        },
        expire: {
            optional: true,
            schema: EXPIRE_USER_SCHEMA,
        },
        firstname: {
            optional: true,
            schema: FIRST_NAME_SCHEMA,
        },
        lastname: {
            schema: LAST_NAME_SCHEMA,
            optional: true,
         },
        email: {
            schema: EMAIL_SCHEMA,
            optional: true,
        },
        tokens: {
            type: Array,
            optional: true,
            description: "List of user's API tokens.",
            items: {
                type: ApiToken
            },
        },
        "totp-locked": {
            type: bool,
            optional: true,
            default: false,
            description: "True if the user is currently locked out of TOTP factors",
        },
        "tfa-locked-until": {
            optional: true,
            description: "Contains a timestamp until when a user is locked out of 2nd factors",
        },
    }
)]
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// User properties with added list of ApiTokens
pub struct UserWithTokens {
    pub userid: Userid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firstname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lastname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tokens: Vec<ApiToken>,
    #[serde(skip_serializing_if = "bool_is_false", default)]
    pub totp_locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tfa_locked_until: Option<i64>,
}

fn bool_is_false(b: &bool) -> bool {
    !b
}

#[api(
    properties: {
        tokenid: {
            schema: PROXMOX_TOKEN_ID_SCHEMA,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        enable: {
            optional: true,
            schema: ENABLE_USER_SCHEMA,
        },
        expire: {
            optional: true,
            schema: EXPIRE_USER_SCHEMA,
        },
    }
)]
#[derive(Serialize, Deserialize, Clone, PartialEq)]
/// ApiToken properties.
pub struct ApiToken {
    pub tokenid: Authid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire: Option<i64>,
}

impl ApiToken {
    pub fn is_active(&self) -> bool {
        if !self.enable.unwrap_or(true) {
            return false;
        }
        if let Some(expire) = self.expire {
            let now = proxmox_time::epoch_i64();
            if expire > 0 && expire <= now {
                return false;
            }
        }
        true
    }
}

#[api(
    properties: {
        userid: {
            type: Userid,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        enable: {
            optional: true,
            schema: ENABLE_USER_SCHEMA,
        },
        expire: {
            optional: true,
            schema: EXPIRE_USER_SCHEMA,
        },
        firstname: {
            optional: true,
            schema: FIRST_NAME_SCHEMA,
        },
        lastname: {
            schema: LAST_NAME_SCHEMA,
            optional: true,
         },
        email: {
            schema: EMAIL_SCHEMA,
            optional: true,
        },
    }
)]
#[derive(Serialize, Deserialize, Updater, PartialEq, Eq)]
/// User properties.
pub struct User {
    #[updater(skip)]
    pub userid: Userid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firstname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lastname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

impl User {
    pub fn is_active(&self) -> bool {
        if !self.enable.unwrap_or(true) {
            return false;
        }
        if let Some(expire) = self.expire {
            let now = proxmox_time::epoch_i64();
            if expire > 0 && expire <= now {
                return false;
            }
        }
        true
    }
}
