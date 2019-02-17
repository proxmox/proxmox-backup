//! API definition helper
//!
//! This module contains helper classes to define REST APIs. Method
//! parameters and return types are described using a
//! [Schema](schema/enum.Schema.html).
//!
//! The [Router](router/struct.Router.html) is used to define a
//! hierarchy of API entries, and provides ways to find an API
//! definition by path.

#[macro_use]
pub mod schema;
pub mod registry;
#[macro_use]
pub mod router;
pub mod config;
