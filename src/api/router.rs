use failure::*;

use crate::api::schema::*;
use serde_json::{Value};
use std::collections::HashMap;

pub struct ApiMethod {
    pub description: &'static str,
    pub parameters: Schema,
    pub returns: Schema,
    pub handler: fn(Value, &ApiMethod) -> Result<Value, Error>,
}

pub enum SubRoute {
    None,
    Hash(HashMap<String, Router>),
    MatchAll { router: Box<Router>, param_name: String },
}

pub struct Router {
    pub get: Option<ApiMethod>,
    pub put: Option<ApiMethod>,
    pub post: Option<ApiMethod>,
    pub delete: Option<ApiMethod>,
    pub subroute: SubRoute,
}

impl Router {

    pub fn new() -> Self {
        Self {
            get: None,
            put: None,
            post: None,
            delete: None,
            subroute: SubRoute::None
        }
    }

    pub fn subdirs(mut self, map: HashMap<String, Router>) -> Self {
        self.subroute = SubRoute::Hash(map);
        self
    }

    pub fn match_all(mut self, router: Router) -> Self {
        self.subroute = SubRoute::MatchAll { router: Box::new(router), param_name: "test".into() };
        self
    }

    pub fn get(mut self, m: ApiMethod) -> Self {
        self.get = Some(m);
        self
    }

    pub fn find_route(&self, components: &[&str]) -> Option<&Router> {

        if components.len() == 0 { return Some(self); };

        let (dir, rest) = (components[0], &components[1..]);

        match self.subroute {
            SubRoute::None => {},
            SubRoute::Hash(ref dirmap) => {
                if let Some(ref router) = dirmap.get(dir) {
                    println!("FOUND SUBDIR {}", dir);
                    return router.find_route(rest);
                }
            }
            SubRoute::MatchAll { ref router, ref param_name } => {
                println!("URI PARAM {} = {}", param_name, dir); // fixme: store somewhere
                return router.find_route(rest);
            },
        }

        None
    }
}

// fixme: remove - not required?
#[macro_export]
macro_rules! methodinfo {
    ($($option:ident => $e:expr),*) => {{
        let info = Router::new();

        $(
            info.$option = Some($e);
        )*

        info
    }}
}
