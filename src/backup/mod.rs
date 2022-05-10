//! Server/client-specific parts for what's otherwise in pbs-datastore.

// Note: .pcat1 => Proxmox Catalog Format version 1
pub const CATALOG_NAME: &str = "catalog.pcat1.didx";

mod verify;
pub use verify::*;

mod hierarchy;
pub use hierarchy::*;
