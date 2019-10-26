//use failure::*;
//use std::collections::HashMap;

//use crate::api_schema;
use crate::api_schema::router::*;

pub mod datastore;

pub fn router() -> Router {
    Router::new()
        .subdir("datastore", datastore::router())
        .list_subdirs()
}
