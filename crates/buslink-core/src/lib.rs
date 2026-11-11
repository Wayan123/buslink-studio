//! BusLink Studio core: Modbus RTU/TCP framing and codec, device profiles,
//! quality model, per-link scheduler, the command service (the only write
//! path), historian and export. Pure Rust, no Tauri dependency.
//!
//! Safety rules: `.kiro/steering/safety.md`. Protocol rules: `.kiro/steering/modbus.md`.

#![forbid(unsafe_code)]

pub mod bus;
pub mod clock;
pub mod codec;
pub mod command;
pub mod device;
pub mod export;
pub mod historian;
pub mod poll;
pub mod profile;
pub mod protocol;
pub mod quality;
pub mod rng;
pub mod testkit;
pub mod time;
pub mod transport;
