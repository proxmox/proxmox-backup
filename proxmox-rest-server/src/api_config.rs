use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;
use std::fs::metadata;
use std::sync::{Arc, Mutex, RwLock};
use std::pin::Pin;

use anyhow::{bail, Error, format_err};
use hyper::{Method, Body, Response};
use hyper::http::request::Parts;

use handlebars::Handlebars;
use serde::Serialize;

use proxmox_sys::fs::{create_path, CreateOptions};
use proxmox_router::{ApiMethod, Router, RpcEnvironmentType, UserInformation};

use crate::{ServerAdapter, AuthError, FileLogger, FileLogOptions, CommandSocket, RestEnvironment};


/// REST server configuration
pub struct ApiConfig {
    basedir: PathBuf,
    router: &'static Router,
    aliases: HashMap<String, PathBuf>,
    env_type: RpcEnvironmentType,
    templates: RwLock<Handlebars<'static>>,
    template_files: RwLock<HashMap<String, (SystemTime, PathBuf)>>,
    request_log: Option<Arc<Mutex<FileLogger>>>,
    auth_log: Option<Arc<Mutex<FileLogger>>>,
    adapter: Pin<Box<dyn ServerAdapter + Send + Sync>>,
}

impl ApiConfig {
    /// Creates a new instance
    ///
    /// `basedir` - File lookups are relative to this directory.
    ///
    /// `router` - The REST API definition.
    ///
    /// `env_type` - The environment type.
    ///
    /// `api_auth` - The Authentication handler
    ///
    /// `get_index_fn` - callback to generate the root page
    /// (index). Please note that this fuctions gets a reference to
    /// the [ApiConfig], so it can use [Handlebars] templates
    /// ([render_template](Self::render_template) to generate pages.
    pub fn new<B: Into<PathBuf>>(
        basedir: B,
        router: &'static Router,
        env_type: RpcEnvironmentType,
        adapter: impl ServerAdapter + 'static,
    ) -> Result<Self, Error> {
        Ok(Self {
            basedir: basedir.into(),
            router,
            aliases: HashMap::new(),
            env_type,
            templates: RwLock::new(Handlebars::new()),
            template_files: RwLock::new(HashMap::new()),
            request_log: None,
            auth_log: None,
            adapter: Box::pin(adapter),
        })
    }

    pub(crate) async fn get_index(
        &self,
        rest_env: RestEnvironment,
        parts: Parts,
    ) -> Response<Body> {
        self.adapter.get_index(rest_env, parts).await
    }

    pub(crate) async fn check_auth(
        &self,
        headers: &http::HeaderMap,
        method: &hyper::Method,
    ) -> Result<(String, Box<dyn UserInformation + Sync + Send>), AuthError> {
        self.adapter.check_auth(headers, method).await
    }

    pub(crate) fn find_method(
        &self,
        components: &[&str],
        method: Method,
        uri_param: &mut HashMap<String, String>,
    ) -> Option<&'static ApiMethod> {

        self.router.find_method(components, method, uri_param)
    }

    pub(crate) fn find_alias(&self, components: &[&str]) -> PathBuf {

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

    /// Register a path alias
    ///
    /// This can be used to redirect file lookups to a specific
    /// directory, e.g.:
    ///
    /// ```
    /// use proxmox_rest_server::ApiConfig;
    /// // let mut config = ApiConfig::new(...);
    /// # fn fake(config: &mut ApiConfig) {
    /// config.add_alias("extjs", "/usr/share/javascript/extjs");
    /// # }
    /// ```
    pub fn add_alias<S, P>(&mut self, alias: S, path: P)
        where S: Into<String>,
              P: Into<PathBuf>,
    {
        self.aliases.insert(alias.into(), path.into());
    }

    pub(crate) fn env_type(&self) -> RpcEnvironmentType {
        self.env_type
    }

    /// Register a [Handlebars] template file
    ///
    /// Those templates cane be use with [render_template](Self::render_template) to generate pages.
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

    /// Enable the access log feature
    ///
    /// When enabled, all requests are logged to the specified file.
    /// This function also registers a `api-access-log-reopen`
    /// command one the [CommandSocket].
    pub fn enable_access_log<P>(
        &mut self,
        path: P,
        dir_opts: Option<CreateOptions>,
        file_opts: Option<CreateOptions>,
        commando_sock: &mut CommandSocket,
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
            println!("re-opening access-log file");
            request_log.lock().unwrap().reopen()?;
            Ok(serde_json::Value::Null)
        })?;

        Ok(())
    }

    /// Enable the authentication log feature
    ///
    /// When enabled, all authentication requests are logged to the
    /// specified file. This function also registers a
    /// `api-auth-log-reopen` command one the [CommandSocket].
    pub fn enable_auth_log<P>(
        &mut self,
        path: P,
        dir_opts: Option<CreateOptions>,
        file_opts: Option<CreateOptions>,
        commando_sock: &mut CommandSocket,
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
            prefix_time: true,
            file_opts: file_opts.unwrap_or(CreateOptions::default()),
            ..Default::default()
        };
        let auth_log = Arc::new(Mutex::new(FileLogger::new(&path, logger_options)?));
        self.auth_log = Some(Arc::clone(&auth_log));

        commando_sock.register_command("api-auth-log-reopen".into(), move |_args| {
            println!("re-opening auth-log file");
            auth_log.lock().unwrap().reopen()?;
            Ok(serde_json::Value::Null)
        })?;

        Ok(())
    }

    pub(crate) fn get_access_log(&self) -> Option<&Arc<Mutex<FileLogger>>> {
        self.request_log.as_ref()
    }

    pub(crate) fn get_auth_log(&self) -> Option<&Arc<Mutex<FileLogger>>> {
        self.auth_log.as_ref()
    }
}
