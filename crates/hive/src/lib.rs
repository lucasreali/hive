//! The `hive` binary: service, bridge and CLI in one executable.

pub mod adapter;
pub mod bridge;
pub mod changes;
pub mod cli;
pub mod daemon;
pub mod dirs;
pub mod file;
pub mod files;
pub mod git;
pub mod health;
pub mod hook;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod paths;
pub mod procs;
pub mod projects;
pub mod scripts;
pub mod search;
pub mod sessions;
pub mod settings;
pub mod spaces;
pub mod states;
pub mod terminal;
pub mod transcript;
pub mod watch;
pub mod worktree;
pub mod wrapper;

/// Binary version, compared with the app's in the handshake.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
