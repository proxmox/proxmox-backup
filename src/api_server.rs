use std::collections::HashMap;
use std::path::{PathBuf};
use crate::api_info::*;
//use crate::json_schema::*;

use hyper::{Method};

pub struct ApiServer {
    basedir: PathBuf,
    router: &'static MethodInfo,
    aliases: HashMap<String, PathBuf>,
}

impl ApiServer {

    pub fn new<B: Into<PathBuf>>(basedir: B, router: &'static MethodInfo) -> Self {
        ApiServer {
            basedir: basedir.into(),
            router: router,
            aliases: HashMap::new(),
        }
    }

    pub fn find_method(&self, components: &[&str], method: Method) -> Option<&'static ApiMethod> {

        if let Some(info) = self.router.find_route(components) {
            println!("FOUND INFO");
            let opt_api_method = match method {
                Method::GET => &info.get,
                Method::PUT => &info.put,
                Method::POST => &info.post,
                Method::DELETE => &info.delete,
                _ => &None,
            };
            if let Some(api_method) = opt_api_method {
                return Some(&api_method);
            }
        }
        None
    }

    pub fn find_alias(&self, components: &[&str]) -> PathBuf {

        let mut prefix = String::new();
        let mut filename = self.basedir.clone();
        let comp_len = components.len();
        if comp_len >= 1 {
            prefix.push_str(components[0]);
            if let Some(subdir) = self.aliases.get(&prefix) {
                filename.push(subdir);
                for i in 1..comp_len { filename.push(components[i]) }
            }
        }
        filename
    }

    pub fn add_alias<S, P>(&mut self, alias: S, path: P)
        where S: Into<String>,
              P: Into<PathBuf>,
    {
        self.aliases.insert(alias.into(), path.into());
    }
}
