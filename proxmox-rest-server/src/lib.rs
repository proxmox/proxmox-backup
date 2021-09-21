use anyhow::{bail, Error};

mod state;
pub use state::*;

mod command_socket;
pub use command_socket::*;

mod file_logger;
pub use file_logger::{FileLogger, FileLogOptions};

mod api_config;
pub use api_config::ApiConfig;

pub enum AuthError {
    Generic(Error),
    NoData,
}

impl From<Error> for AuthError {
    fn from(err: Error) -> Self {
        AuthError::Generic(err)
    }
}

pub trait ApiAuth {
    fn check_auth(
        &self,
        headers: &http::HeaderMap,
        method: &hyper::Method,
    ) -> Result<String, AuthError>;
}

static mut SHUTDOWN_REQUESTED: bool = false;

pub fn request_shutdown() {
    unsafe {
        SHUTDOWN_REQUESTED = true;
    }
    crate::server_shutdown();
}

#[inline(always)]
pub fn shutdown_requested() -> bool {
    unsafe { SHUTDOWN_REQUESTED }
}

pub fn fail_on_shutdown() -> Result<(), Error> {
    if shutdown_requested() {
        bail!("Server shutdown requested - aborting task");
    }
    Ok(())
}

