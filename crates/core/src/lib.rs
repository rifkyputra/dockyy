//! kuadrat-core — transport-agnostic engine for Podman Quadlet workloads.
//!
//! This crate never opens a socket and never takes a host parameter.

//! Two seams carry every side effect: [`exec::Executor`] for processes and
//! [`fs::FileSystem`] for storage. Nothing else in the crate touches the host.

pub mod deploy;
pub mod events;
pub mod exec;
pub mod fs;
pub mod secrets;
pub mod spec;
pub mod store;
pub mod workloads;
