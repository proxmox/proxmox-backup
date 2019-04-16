//use failure::*;
//use std::collections::HashMap;

use crate::api_schema::*;
use crate::api_schema::router::*;
use serde_json::{json};

pub mod datastore;

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            || Ok(json!([
                {"subdir": "datastore"},
            ])),
            ObjectSchema::new("Directory index.")))
        .subdir("datastore", datastore::router());
   

    route
}
