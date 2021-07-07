//! See the different modules for documentation on their usage.
//!
//! The [backup](backup/index.html) module contains some detailed information
//! on the inner workings of the backup server regarding data storage.

#[macro_use]
pub mod tools;

#[macro_use]
pub mod server;

pub mod pxar;

#[macro_use]
pub mod backup;

pub mod config;

pub mod api2;

pub mod client;

pub mod auth_helpers;

pub mod auth;

pub mod rrd;

pub mod tape;

pub mod acme;
