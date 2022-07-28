use std::fmt;

use anyhow::{bail, Error};
use serde::{Deserialize, Serialize};

use pbs_api_types::Userid;

use crate::config::tfa;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialTicket {
    #[serde(rename = "u")]
    userid: Userid,

    #[serde(rename = "c")]
    challenge: tfa::TfaChallenge,
}

/// A new ticket struct used in rest.rs's `check_auth` - mostly for better errors than failing to
/// parse the userid ticket content.
pub enum ApiTicket {
    Full(Userid),
    Partial(Box<tfa::TfaChallenge>),
}

impl ApiTicket {
    /// Require the ticket to be a full ticket, otherwise error with a meaningful error message.
    pub fn require_full(self) -> Result<Userid, Error> {
        match self {
            ApiTicket::Full(userid) => Ok(userid),
            ApiTicket::Partial(_) => bail!("access denied - second login factor required"),
        }
    }

    /// Expect the ticket to contain a tfa challenge, otherwise error with a meaningful error
    /// message.
    pub fn require_partial(self) -> Result<Box<tfa::TfaChallenge>, Error> {
        match self {
            ApiTicket::Full(_) => bail!("invalid tfa challenge"),
            ApiTicket::Partial(challenge) => Ok(challenge),
        }
    }
}

impl fmt::Display for ApiTicket {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ApiTicket::Full(userid) => fmt::Display::fmt(userid, f),
            ApiTicket::Partial(partial) => {
                let data = serde_json::to_string(partial).map_err(|_| fmt::Error)?;
                write!(f, "!tfa!{}", data)
            }
        }
    }
}

impl std::str::FromStr for ApiTicket {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        if let Some(tfa_ticket) = s.strip_prefix("!tfa!") {
            Ok(ApiTicket::Partial(serde_json::from_str(tfa_ticket)?))
        } else {
            Ok(ApiTicket::Full(s.parse()?))
        }
    }
}
