//! Server/client-specific parts for what's otherwise in pbs-datastore.

// Note: .pcat1 => Proxmox Catalog Format version 1
pub const CATALOG_NAME: &str = "catalog.pcat1.didx";

// Split
mod read_chunk;
pub use read_chunk::*;

mod datastore;
pub use datastore::*;

mod verify;
pub use verify::*;
