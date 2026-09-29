//! Integration tests: run the real `hive` binary. On Windows only `windows` runs until the
//! service has terminals there (12.5.3).

#[cfg(unix)]
#[cfg(test)]
mod agents;
#[cfg(unix)]
#[cfg(test)]
mod bridge;
#[cfg(unix)]
#[cfg(test)]
mod changes;
#[cfg(unix)]
#[cfg(test)]
mod common;
#[cfg(unix)]
#[cfg(test)]
mod daemon;
#[cfg(unix)]
#[cfg(test)]
mod file;
#[cfg(unix)]
#[cfg(test)]
mod files;
#[cfg(unix)]
#[cfg(test)]
mod hook;
#[cfg(unix)]
#[cfg(test)]
mod projects;
#[cfg(unix)]
#[cfg(test)]
mod pulls;
#[cfg(unix)]
#[cfg(test)]
mod registry;
#[cfg(unix)]
#[cfg(test)]
mod scripts;
#[cfg(unix)]
#[cfg(test)]
mod spaces;
#[cfg(unix)]
#[cfg(test)]
mod statusline;
#[cfg(unix)]
#[cfg(test)]
mod terminal;
#[cfg(unix)]
#[cfg(test)]
mod watch;
#[cfg(windows)]
#[cfg(test)]
mod windows;
#[cfg(unix)]
#[cfg(test)]
mod worktree;
