//! Client side interface to the proxmox backup server
//!
//! This library implements the client side to access the backups
//! server using https.

pub mod pipe_to_stream;
mod merge_known_chunks;

mod http_client;
pub use  http_client::*;

mod pxar_backup_stream;
pub use pxar_backup_stream::*;

mod pxar_decode_writer;
pub use pxar_decode_writer::*;

mod backup_repo;
pub use backup_repo::*;
