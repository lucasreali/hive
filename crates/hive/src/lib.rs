//! The `hive` binary: service, bridge and CLI in one executable.

pub mod adapter;
pub mod cli;
pub mod daemon;
pub mod paths;
pub mod procs;
pub mod terminal;

/// Binary version, compared with the app's in the handshake.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
