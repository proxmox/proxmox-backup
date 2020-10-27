//! Proxmox Server/Service framework
//!
//! This code provides basic primitives to build our REST API
//! services. We want async IO, so this is built on top of
//! tokio/hyper.

mod environment;
pub use environment::*;

mod upid;
pub use upid::*;

mod state;
pub use state::*;

mod command_socket;
pub use command_socket::*;

mod worker_task;
pub use worker_task::*;

mod h2service;
pub use h2service::*;

pub mod config;
pub use config::*;

pub mod formatter;

#[macro_use]
pub mod rest;

pub mod jobstate;

mod verify_job;
pub use verify_job::*;

mod email_notifications;
pub use email_notifications::*;
