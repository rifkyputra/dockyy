//! kuadrat-core — transport-agnostic engine for Podman Quadlet workloads.
//!
//! This crate never opens a socket and never takes a host parameter.

pub mod exec;
pub mod spec;
pub mod workloads;
