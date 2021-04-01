//! Provides authentication primitives for the HTTP server
use anyhow::{bail, format_err, Error};

use crate::tools::ticket::Ticket;
use crate::auth_helpers::*;
use crate::tools;
use crate::config::cached_user_info::CachedUserInfo;
use crate::api2::types::{Authid, Userid};

use hyper::header;
use percent_encoding::percent_decode_str;

pub struct UserAuthData {
    ticket: String,
    csrf_token: Option<String>,
}

pub enum AuthData {
    User(UserAuthData),
    ApiToken(String),
}

pub fn extract_auth_data(headers: &http::HeaderMap) -> Option<AuthData> {
    if let Some(raw_cookie) = headers.get(header::COOKIE) {
        if let Ok(cookie) = raw_cookie.to_str() {
            if let Some(ticket) = tools::extract_cookie(cookie, "PBSAuthCookie") {
                let csrf_token = match headers.get("CSRFPreventionToken").map(|v| v.to_str()) {
                    Some(Ok(v)) => Some(v.to_owned()),
                    _ => None,
                };
                return Some(AuthData::User(UserAuthData {
                    ticket,
                    csrf_token,
                }));
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
        },
        _ => None,
    }
}

pub fn check_auth(
    method: &hyper::Method,
    auth_data: &AuthData,
    user_info: &CachedUserInfo,
) -> Result<Authid, Error> {
    match auth_data {
        AuthData::User(user_auth_data) => {
            let ticket = user_auth_data.ticket.clone();
            let ticket_lifetime = tools::ticket::TICKET_LIFETIME;

            let userid: Userid = Ticket::<super::ticket::ApiTicket>::parse(&ticket)?
                .verify_with_time_frame(public_auth_key(), "PBS", None, -300..ticket_lifetime)?
                .require_full()?;

            let auth_id = Authid::from(userid.clone());
            if !user_info.is_active_auth_id(&auth_id) {
                bail!("user account disabled or expired.");
            }

            if method != hyper::Method::GET {
                if let Some(csrf_token) = &user_auth_data.csrf_token {
                    verify_csrf_prevention_token(csrf_secret(), &userid, &csrf_token, -300, ticket_lifetime)?;
                } else {
                    bail!("missing CSRF prevention token");
                }
            }

            Ok(auth_id)
        },
        AuthData::ApiToken(api_token) => {
            let mut parts = api_token.splitn(2, ':');
            let tokenid = parts.next()
                .ok_or_else(|| format_err!("failed to split API token header"))?;
            let tokenid: Authid = tokenid.parse()?;

            if !user_info.is_active_auth_id(&tokenid) {
                bail!("user account or token disabled or expired.");
            }

            let tokensecret = parts.next()
                .ok_or_else(|| format_err!("failed to split API token header"))?;
            let tokensecret = percent_decode_str(tokensecret)
                .decode_utf8()
                .map_err(|_| format_err!("failed to decode API token header"))?;

            crate::config::token_shadow::verify_secret(&tokenid, &tokensecret)?;

            Ok(tokenid)
        }
    }
}
