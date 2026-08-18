//! The seam to the kuadrat daemon. Task 3 gives it a real surface; the trait
//! exists from Task 1 so `serve`'s signature never changes.

pub trait Daemon: Send + Sync {}
