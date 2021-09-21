//! Provides authentication primitives for the HTTP server
use anyhow::format_err;

use std::sync::Arc;

use pbs_tools::ticket::{self, Ticket};
use pbs_config::{token_shadow, CachedUserInfo};
use pbs_api_types::{Authid, Userid};
use proxmox_rest_server::{ApiAuth, AuthError, extract_cookie};

use crate::auth_helpers::*;

use hyper::header;
use percent_encoding::percent_decode_str;

struct UserAuthData {
    ticket: String,
    csrf_token: Option<String>,
}

enum AuthData {
    User(UserAuthData),
    ApiToken(String),
}

pub struct UserApiAuth {}
pub fn default_api_auth() -> Arc<UserApiAuth> {
    Arc::new(UserApiAuth {})
}

impl UserApiAuth {
    fn extract_auth_data(headers: &http::HeaderMap) -> Option<AuthData> {
        if let Some(raw_cookie) = headers.get(header::COOKIE) {
            if let Ok(cookie) = raw_cookie.to_str() {
                if let Some(ticket) = extract_cookie(cookie, "PBSAuthCookie") {
                    let csrf_token = match headers.get("CSRFPreventionToken").map(|v| v.to_str()) {
                        Some(Ok(v)) => Some(v.to_owned()),
                        _ => None,
                    };
                    return Some(AuthData::User(UserAuthData { ticket, csrf_token }));
                }
            }
        }

        match headers.get(header::AUTHORIZATION).map(|v| v.to_str()) {
            Some(Ok(v)) => {
                if v.starts_with("PBSAPIToken ") || v.starts_with("PBSAPIToken=") {
                    Some(AuthData::ApiToken(v["PBSAPIToken ".len()..].to_owned()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl ApiAuth for UserApiAuth {
    fn check_auth(
        &self,
        headers: &http::HeaderMap,
        method: &hyper::Method,
    ) -> Result<String, AuthError> {

        let user_info = CachedUserInfo::new()?;

        let auth_data = Self::extract_auth_data(headers);
        match auth_data {
            Some(AuthData::User(user_auth_data)) => {
                let ticket = user_auth_data.ticket.clone();
                let ticket_lifetime = ticket::TICKET_LIFETIME;

                let userid: Userid = Ticket::<super::ticket::ApiTicket>::parse(&ticket)?
                    .verify_with_time_frame(public_auth_key(), "PBS", None, -300..ticket_lifetime)?
                    .require_full()?;

                let auth_id = Authid::from(userid.clone());
                if !user_info.is_active_auth_id(&auth_id) {
                    return Err(format_err!("user account disabled or expired.").into());
                }

                if method != hyper::Method::GET {
                    if let Some(csrf_token) = &user_auth_data.csrf_token {
                        verify_csrf_prevention_token(
                            csrf_secret(),
                            &userid,
                            &csrf_token,
                            -300,
                            ticket_lifetime,
                        )?;
                    } else {
                        return Err(format_err!("missing CSRF prevention token").into());
                    }
                }

                Ok(auth_id.to_string())
            }
            Some(AuthData::ApiToken(api_token)) => {
                let mut parts = api_token.splitn(2, ':');
                let tokenid = parts
                    .next()
                    .ok_or_else(|| format_err!("failed to split API token header"))?;
                let tokenid: Authid = tokenid.parse()?;

                if !user_info.is_active_auth_id(&tokenid) {
                    return Err(format_err!("user account or token disabled or expired.").into());
                }

                let tokensecret = parts
                    .next()
                    .ok_or_else(|| format_err!("failed to split API token header"))?;
                let tokensecret = percent_decode_str(tokensecret)
                    .decode_utf8()
                    .map_err(|_| format_err!("failed to decode API token header"))?;

                token_shadow::verify_secret(&tokenid, &tokensecret)?;

                Ok(tokenid.to_string())
            }
            None => Err(AuthError::NoData),
        }
    }
}
