#[macro_use]
pub mod tools;

/// API definition helper
///
/// This module contains helper classes to define REST APIs. Method
/// parameters and return types are described using a
/// [Schema](schema/enum.Schema.html).
///
/// The [Router](router/struct.Router.html) is used to define a
/// hierarchy of API entries, and provides ways to find an API
/// definition by path.

#[macro_use]
pub mod api {

    #[macro_use]
    pub mod schema;
    pub mod registry;
    #[macro_use]
    pub mod router;
    pub mod config;
}

#[macro_use]
pub mod server {

    pub mod environment;
    pub mod formatter;
    #[macro_use]
    pub mod rest;

}

pub mod catar;

pub mod section_config;

pub mod backup;

pub mod config {

    pub mod datastore;
}

pub mod storage {

    pub mod config;
    pub mod futures;
}

pub mod cli {

    pub mod environment;
    pub mod command;
}


pub mod api2;

pub mod client {

    pub mod http_client;
    pub mod catar_backup_stream;
}

pub mod getopts;
pub mod auth_helpers;
