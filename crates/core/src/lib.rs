//! kuadrat-core — transport-agnostic engine for Podman Quadlet workloads.
//!
//! This crate never opens a socket and never takes a host parameter.

//! Three seams carry every side effect: [`exec::Executor`] for processes,
//! [`fs::FileSystem`] for storage, and [`events::EventSink`] for publishing
//! events to subscribers. Nothing else in the crate touches the host.

pub mod deploy;
pub mod events;
pub mod exec;
pub mod fs;
pub mod gateway;
pub mod logs;
pub mod managed;
pub mod secrets;
pub mod spec;
pub mod store;
pub mod workloads;
