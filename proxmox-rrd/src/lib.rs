//! # Round Robin Database files
//!
//! ## Features
//!
//! * One file stores a single data source
//! * Stores data for different time resolution
//! * Simple cache implementation with journal support

mod rrd_v1;

pub mod rrd;
#[doc(inline)]
pub use rrd::Entry;

mod cache;
pub use cache::*;
