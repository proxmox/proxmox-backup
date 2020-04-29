use std::collections::HashMap;
use std::path::{PathBuf};
use anyhow::Error;

use hyper::Method;
use handlebars::Handlebars;

use proxmox::api::{ApiMethod, Router, RpcEnvironmentType};

pub struct ApiConfig {
    basedir: PathBuf,
    router: &'static Router,
    aliases: HashMap<String, PathBuf>,
    env_type: RpcEnvironmentType,
    pub templates: Handlebars<'static>,
}

impl ApiConfig {

    pub fn new<B: Into<PathBuf>>(basedir: B, router: &'static Router, env_type: RpcEnvironmentType) -> Result<Self, Error> {
        let mut templates = Handlebars::new();
        let basedir = basedir.into();
        templates.register_template_file("index", basedir.join("index.hbs"))?;
        Ok(Self {
            basedir,
            router,
            aliases: HashMap::new(),
            env_type,
            templates
        })
    }

    pub fn find_method(
        &self,
        components: &[&str],
        method: Method,
        uri_param: &mut HashMap<String, String>,
    ) -> Option<&'static ApiMethod> {

        self.router.find_method(components, method, uri_param)
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
            } else {
                for i in 0..comp_len { filename.push(components[i]) }
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

    pub fn env_type(&self) -> RpcEnvironmentType {
        self.env_type
    }
}
