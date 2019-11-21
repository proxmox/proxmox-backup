use crate::api_schema::router::*;

pub mod datastore;

const SUBDIRS: SubdirMap = &[
    ("datastore", &datastore::ROUTER)
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
