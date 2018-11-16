pub mod static_map;

#[macro_use]
pub mod api {

    #[macro_use]
    pub mod schema;
    #[macro_use]
    pub mod router;
    pub mod config;
    pub mod server;

}

pub mod getopts;

pub mod api3;

