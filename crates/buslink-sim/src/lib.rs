//! BusLink Studio simulator: deterministic in-process Modbus slaves for
//! training and tests. Every device here is a **simulated device**; nothing
//! in this crate upgrades a profile's support level (safety rule 12).
//!
//! The simulator decodes write PDUs (it plays the slave); it never builds
//! write requests. Requests come from `buslink-core`.

#![forbid(unsafe_code)]

pub mod devices;
pub mod fault;
pub mod image;
pub mod server;
pub mod tcp;
pub mod training;
pub mod transport;
pub mod world;

pub use fault::{FaultKind, FaultRule, Trigger};
pub use transport::{Chunking, SimTransport};
pub use world::{SharedWorld, SimWorld};
