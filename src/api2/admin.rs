use crate::api_schema::*;
use crate::api_schema::router::*;
use serde_json::{json};

pub mod datastore;

pub fn router() -> Router {

    let route = Router::new()
        .subdir("datastore", datastore::router())
        .list_subdirs();

    route
}
