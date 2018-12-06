use failure::*;

use crate::api::schema::*;
use serde_json::{Value};
use std::collections::HashMap;
use std::sync::Arc;

type ApiHandlerFn = fn(Value, &ApiMethod) -> Result<Value, Error>;

pub struct ApiMethod {
    pub parameters: ObjectSchema,
    pub returns: Arc<Schema>,
    pub handler: ApiHandlerFn,
}

impl ApiMethod {

    pub fn new(handler: ApiHandlerFn, parameters: ObjectSchema) -> Self {
        Self {
            parameters,
            handler,
            returns: Arc::new(Schema::Null),
        }
    }

    pub fn returns<S: Into<Arc<Schema>>>(mut self, schema: S) -> Self {

        self.returns = schema.into();

        self
    }

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

    pub fn subdir<S: Into<String>>(mut self, subdir: S, router: Router) -> Self {
        if let SubRoute::None = self.subroute {
            self.subroute = SubRoute::Hash(HashMap::new());
        }
        match self.subroute {
            SubRoute::Hash(ref mut map) => {
                map.insert(subdir.into(), router);
            }
            _ => panic!("unexpected subroute type"),
        }
        self
    }

    pub fn subdirs(mut self, map: HashMap<String, Router>) -> Self {
        self.subroute = SubRoute::Hash(map);
        self
    }

    pub fn match_all<S: Into<String>>(mut self, param_name: S, router: Router) -> Self {
        if let SubRoute::None = self.subroute {
            self.subroute = SubRoute::MatchAll { router: Box::new(router), param_name: param_name.into() };
        } else {
            panic!("unexpected subroute type");
        }
        self
    }

    pub fn get(mut self, m: ApiMethod) -> Self {
        self.get = Some(m);
        self
    }

    pub fn put(mut self, m: ApiMethod) -> Self {
        self.put = Some(m);
        self
    }

    pub fn post(mut self, m: ApiMethod) -> Self {
        self.post = Some(m);
        self
    }

    pub fn delete(mut self, m: ApiMethod) -> Self {
        self.delete = Some(m);
        self
    }

    pub fn find_route(&self, components: &[&str], uri_param: &mut HashMap<String, String>) -> Option<&Router> {

        if components.len() == 0 { return Some(self); };

        let (dir, rest) = (components[0], &components[1..]);

        match self.subroute {
            SubRoute::None => {},
            SubRoute::Hash(ref dirmap) => {
                if let Some(ref router) = dirmap.get(dir) {
                    println!("FOUND SUBDIR {}", dir);
                    return router.find_route(rest, uri_param);
                }
            }
            SubRoute::MatchAll { ref router, ref param_name } => {
                println!("URI PARAM {} = {}", param_name, dir); // fixme: store somewhere
                uri_param.insert(param_name.clone(), dir.into());
                return router.find_route(rest, uri_param);
            },
        }

        None
    }
}
