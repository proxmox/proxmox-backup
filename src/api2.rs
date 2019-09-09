pub mod types;
pub mod config;
pub mod admin;
pub mod backup;
pub mod reader;
pub mod node;
pub mod version;
mod subscription;
mod access;

use crate::api_schema::router::*;

pub fn router() -> Router {

    let nodes = Router::new()
        .match_all("node", node::router());

    let route = Router::new()
        .subdir("access", access::router())
        .subdir("admin", admin::router())
        .subdir("backup", backup::router())
        .subdir("reader", reader::router())
        .subdir("config", config::router())
        .subdir("nodes", nodes)
        .subdir("subscription", subscription::router())
        .subdir("version", version::router())
        .list_subdirs();

    route
}
