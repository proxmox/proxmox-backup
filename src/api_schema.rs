//! API definition helper
//!
//! This module contains helper classes to define REST APIs. Method
//! parameters and return types are described using a
//! [Schema](schema/enum.Schema.html).
//!
//! The [Router](router/struct.Router.html) is used to define a
//! hierarchy of API entries, and provides ways to find an API
//! definition by path.

//pub mod registry;
pub mod config;
pub mod format;

/*
 * --------------------------------------------------------------------------------------------
 * Everything below is a compatibility layer to support building the current code until api2.rs
 * and the api2/ directory have been updated to the proxmox::api crate:
 * --------------------------------------------------------------------------------------------
 */

pub use proxmox::api::schema::*;
pub use proxmox::api::*;

pub use proxmox::api::ApiFuture as BoxFut;

pub mod api_handler {
    pub use super::{ApiAsyncHandlerFn, ApiHandler, ApiHandlerFn, BoxFut};
}

pub mod router {
    pub use super::{ApiHandler, ApiMethod, HttpError, RpcEnvironment, RpcEnvironmentType};
    pub use proxmox::api::router::*;
}

pub mod schema {
    pub use proxmox::api::schema::*;
}
