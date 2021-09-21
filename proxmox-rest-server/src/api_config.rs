use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;
use std::fs::metadata;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{bail, Error, format_err};
use hyper::Method;
use handlebars::Handlebars;
use serde::Serialize;

use proxmox::api::{ApiMethod, Router, RpcEnvironmentType};
use proxmox::tools::fs::{create_path, CreateOptions};

use crate::{ApiAuth, FileLogger, FileLogOptions, CommandoSocket};

pub struct ApiConfig {
    basedir: PathBuf,
    router: &'static Router,
    aliases: HashMap<String, PathBuf>,
    env_type: RpcEnvironmentType,
    templates: RwLock<Handlebars<'static>>,
    template_files: RwLock<HashMap<String, (SystemTime, PathBuf)>>,
    request_log: Option<Arc<Mutex<FileLogger>>>,
    pub api_auth: Arc<dyn ApiAuth + Send + Sync>,
}

impl ApiConfig {
    pub fn new<B: Into<PathBuf>>(
        basedir: B,
        router: &'static Router,
        env_type: RpcEnvironmentType,
        api_auth: Arc<dyn ApiAuth + Send + Sync>,
    ) -> Result<Self, Error> {
        Ok(Self {
            basedir: basedir.into(),
            router,
            aliases: HashMap::new(),
            env_type,
            templates: RwLock::new(Handlebars::new()),
            template_files: RwLock::new(HashMap::new()),
            request_log: None,
            api_auth,
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
                components.iter().skip(1).for_each(|comp| filename.push(comp));
            } else {
                components.iter().for_each(|comp| filename.push(comp));
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

    pub fn register_template<P>(&self, name: &str, path: P) -> Result<(), Error>
    where
        P: Into<PathBuf>
    {
        if self.template_files.read().unwrap().contains_key(name) {
            bail!("template already registered");
        }

        let path: PathBuf = path.into();
        let metadata = metadata(&path)?;
        let mtime = metadata.modified()?;

        self.templates.write().unwrap().register_template_file(name, &path)?;
        self.template_files.write().unwrap().insert(name.to_string(), (mtime, path));

        Ok(())
    }

    /// Checks if the template was modified since the last rendering
    /// if yes, it loads a the new version of the template
    pub fn render_template<T>(&self, name: &str, data: &T) -> Result<String, Error>
    where
        T: Serialize,
    {
        let path;
        let mtime;
        {
            let template_files = self.template_files.read().unwrap();
            let (old_mtime, old_path) = template_files.get(name).ok_or_else(|| format_err!("template not found"))?;

            mtime = metadata(old_path)?.modified()?;
            if mtime <= *old_mtime {
                return self.templates.read().unwrap().render(name, data).map_err(|err| format_err!("{}", err));
            }
            path = old_path.to_path_buf();
        }

        {
            let mut template_files = self.template_files.write().unwrap();
            let mut templates = self.templates.write().unwrap();

            templates.register_template_file(name, &path)?;
            template_files.insert(name.to_string(), (mtime, path));

            templates.render(name, data).map_err(|err| format_err!("{}", err))
        }
    }

    pub fn enable_file_log<P>(
        &mut self,
        path: P,
        dir_opts: Option<CreateOptions>,
        file_opts: Option<CreateOptions>,
        commando_sock: &mut CommandoSocket,
    ) -> Result<(), Error>
    where
        P: Into<PathBuf>
    {
        let path: PathBuf = path.into();
        if let Some(base) = path.parent() {
            if !base.exists() {
                create_path(base, None, dir_opts).map_err(|err| format_err!("{}", err))?;
            }
        }

        let logger_options = FileLogOptions {
            append: true,
            file_opts: file_opts.unwrap_or(CreateOptions::default()),
            ..Default::default()
        };
        let request_log = Arc::new(Mutex::new(FileLogger::new(&path, logger_options)?));
        self.request_log = Some(Arc::clone(&request_log));

        commando_sock.register_command("api-access-log-reopen".into(), move |_args| {
            println!("re-opening log file");
            request_log.lock().unwrap().reopen()?;
            Ok(serde_json::Value::Null)
        })?;

        Ok(())
    }

    pub fn get_file_log(&self) -> Option<&Arc<Mutex<FileLogger>>> {
        self.request_log.as_ref()
    }
}
