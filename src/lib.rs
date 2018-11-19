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

pub mod section_config;

pub mod storage {

    pub mod futures;
}

pub mod getopts;

pub mod api3;

