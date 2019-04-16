//use failure::*;

use crate::api_schema::*;
use crate::api_schema::router::*;
use serde_json::{json};
use std::sync::Arc;

pub mod config;
pub mod admin;
pub mod node;
mod version;
mod subscription;
mod access;

use lazy_static::lazy_static;
use crate::tools::common_regex;

// common schema definitions

lazy_static! {
    pub static ref IP_FORMAT: Arc<ApiStringFormat> = ApiStringFormat::Pattern(&common_regex::IP_REGEX).into();

    pub static ref PVE_CONFIG_DIGEST_FORMAT: Arc<ApiStringFormat> =
        ApiStringFormat::Pattern(&common_regex::SHA256_HEX_REGEX).into();

    pub static ref PVE_CONFIG_DIGEST_SCHEMA: Arc<Schema> =
        StringSchema::new("Prevent changes if current configuration file has different SHA256 digest. This can be used to prevent concurrent modifications.")
        .format(PVE_CONFIG_DIGEST_FORMAT.clone()).into();
}

pub fn router() -> Router {

    let nodes = Router::new()
        .match_all("node", node::router());

    let route = Router::new()
        .get(ApiMethod::new(
            || Ok(json!([
                {"subdir": "access"},
                {"subdir": "admin"},
                {"subdir": "config"},
                {"subdir": "nodes"},
                {"subdir": "subscription"},
                {"subdir": "version"},
            ])),
            ObjectSchema::new("Directory index.")))
        .subdir("access", access::router())
        .subdir("admin", admin::router())
        .subdir("config", config::router())
        .subdir("nodes", nodes)
        .subdir("subscription", subscription::router())
        .subdir("version", version::router());

    route
}
