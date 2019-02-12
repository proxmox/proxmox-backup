//! This module implements the proxmox backup chunked data storage
//!
//! A chunk is simply defined as binary blob. We store them inside a
//! `ChunkStore`, addressed by the SHA256 digest of the binary
//! blob. This technology is also known as content-addressable
//! storage.
//!
//! We store larger files by splitting them into chunks. The resulting
//! SHA256 digest list is stored as separate index file. The
//! `DynamicIndex*` format is able to deal with dynamic chunk sizes,
//! whereas the `FixedIndex*` format is an optimization to store a
//! list of equal sized chunks.

pub mod chunker;
pub mod chunk_store;
pub mod fixed_index;
pub mod dynamic_index;
pub mod datastore;
