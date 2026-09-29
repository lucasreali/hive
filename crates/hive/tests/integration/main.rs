//! Integration tests: run the real `hive` binary. The service runs on Unix only until 12.5.2.
#![cfg(unix)]

#[cfg(test)]
mod agents;
#[cfg(test)]
mod bridge;
#[cfg(test)]
mod changes;
#[cfg(test)]
mod common;
#[cfg(test)]
mod daemon;
#[cfg(test)]
mod file;
#[cfg(test)]
mod files;
#[cfg(test)]
mod hook;
#[cfg(test)]
mod projects;
#[cfg(test)]
mod pulls;
#[cfg(test)]
mod registry;
#[cfg(test)]
mod scripts;
#[cfg(test)]
mod spaces;
#[cfg(test)]
mod terminal;
#[cfg(test)]
mod watch;
#[cfg(test)]
mod worktree;
