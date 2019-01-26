use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json};

pub mod datastore;

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            |_,_,_| Ok(json!([
                {"subdir": "datastore"}
            ])),
            ObjectSchema::new("Directory index.")))
        .subdir("datastore", datastore::router());

    route
}
