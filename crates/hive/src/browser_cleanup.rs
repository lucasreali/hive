//! What the agent's browser, removed in 26.11, left in an install that had it. The service runs
//! this when it starts: once the installs in use have started a version with it, it can go. The
//! only code that still names these things.

use std::path::Path;

/// The MCP server's config, in the bin folder: a `claude` wrapper of that time passes it on.
const MCP_CONFIG: &str = "hive-browser.json";
/// The browsers' temporary profiles, in the private runtime folder.
const PROFILES: &str = "browsers";
/// In a browser's profile: its process group, as decimal digits.
#[cfg(unix)]
const GROUP_FILE: &str = "hive-browser-group";

/// Removes [`MCP_CONFIG`] from `bin`, ends the browsers an earlier service left running (killed
/// before it could stop them: their DevTools port has no password) and removes their profiles
/// from `runtime`. Nothing there is nothing to do.
pub fn clean(bin: &Path, runtime: &Path) {
    let config = bin.join(MCP_CONFIG);
    if let Err(err) = remove(&config) {
        eprintln!("hive: warning: cannot remove {}: {err}", config.display());
    }
    let profiles = runtime.join(PROFILES);
    #[cfg(unix)]
    reap(&profiles, crate::procs::Source::System, &mut kill_group);
    let _ = std::fs::remove_dir_all(&profiles);
}

/// Removes the file `path`; one already gone is no error.
fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Ends the browsers of the profiles in `dir`. A profile's [`GROUP_FILE`] names the group killed,
/// and only while its leader still runs with that profile: a number the system gave another
/// process since is left alone. `kill` ends a group ([`kill_group`]).
#[cfg(unix)]
fn reap(dir: &Path, source: crate::procs::Source, kill: &mut dyn FnMut(i32)) {
    use std::io::Read as _;
    for profile in std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
    {
        let mut text = String::new();
        let read = std::fs::File::open(profile.join(GROUP_FILE))
            .and_then(|f| f.take(32).read_to_string(&mut text));
        let group = read.ok().and_then(|_| text.trim().parse::<i32>().ok());
        let mut flag = std::ffi::OsString::from("--user-data-dir=");
        flag.push(&profile);
        let flag = flag.to_string_lossy().into_owned();
        // Never 0 (the service's own group), 1 (`killpg(1)` is `kill(-1)`: every process of
        // the user) or a negative number, whatever their arguments.
        let leader = group
            .filter(|&g| g > 1)
            .filter(|&g| crate::procs::args(source, g).is_some_and(|args| args.contains(&flag)));
        leader.into_iter().for_each(&mut *kill);
    }
}

/// Ends the process group `group` at once (SIGKILL).
#[cfg(unix)]
fn kill_group(group: i32) {
    use nix::sys::signal::{Signal, killpg};
    let _ = killpg(nix::unistd::Pid::from_raw(group), Signal::SIGKILL);
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    use super::*;

    /// A process leading its own group, with `args` for arguments and a child in that group.
    fn group_leader(args: &[String]) -> Child {
        Command::new("sh")
            .args(["-c", "sleep 30; :", "browser"])
            .args(args)
            .process_group(0)
            .spawn()
            .unwrap()
    }

    #[test]
    fn the_mcp_config_goes_and_is_not_missed() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let runtime = tmp.path().join("run");
        std::fs::create_dir(&bin).unwrap();
        std::fs::write(bin.join("claude"), "the wrapper").unwrap();
        std::fs::write(bin.join(MCP_CONFIG), "{}").unwrap();
        clean(&bin, &runtime);
        assert!(!bin.join(MCP_CONFIG).exists());
        assert!(bin.join("claude").exists());
        // Already gone, and nothing it can remove: no harm, what is there stays.
        clean(&bin, &runtime);
        std::fs::create_dir(bin.join(MCP_CONFIG)).unwrap();
        clean(&bin, &runtime);
        assert!(bin.join(MCP_CONFIG).is_dir());
    }

    #[test]
    fn a_file_already_gone_is_removed_and_one_that_cannot_go_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(MCP_CONFIG);
        assert!(remove(&file).is_ok());
        std::fs::create_dir(&file).unwrap();
        assert!(remove(&file).is_err());
        assert!(file.is_dir());
    }

    #[test]
    fn a_left_browser_ends_its_profile_goes_and_a_stranger_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let runtime = tmp.path().join("run");
        let dir = runtime.join(PROFILES);
        let profile = dir.join("1");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("Cookies"), "a page's").unwrap();
        // Beside the profiles, in the same runtime folder: the service's socket stays.
        std::fs::write(runtime.join("hive.sock"), "").unwrap();
        let flag = format!("--user-data-dir={}", profile.display());
        let mut left = group_leader(&[flag]);
        std::fs::write(profile.join(GROUP_FILE), left.id().to_string()).unwrap();
        // Another process now has the number another profile names: not ours, left alone.
        let mut stranger = group_leader(&[]);
        std::fs::create_dir_all(dir.join("2")).unwrap();
        std::fs::write(dir.join("2").join(GROUP_FILE), stranger.id().to_string()).unwrap();
        // Unreadable numbers and a profile without one: nothing to kill.
        for (name, text) in [("3", "x"), ("4", "")] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join(GROUP_FILE), text).unwrap();
        }
        std::fs::create_dir_all(dir.join("6")).unwrap();
        // Its arguments are readable only once `sh` runs in place of the fork.
        let source = crate::procs::Source::System;
        let args = |pid: u32| crate::procs::args(source, pid as i32);
        let start = Instant::now();
        let flagged = (0..1000).any(|_| {
            std::thread::sleep(Duration::from_millis(10));
            args(left.id()).is_some_and(|a| a.iter().any(|a| a.starts_with("--user")))
        });
        assert!(flagged);
        clean(tmp.path(), &runtime);
        // Killed at once: not left to its 30 s.
        let ended = left.wait().unwrap();
        assert_eq!(ended.signal(), Some(nix::libc::SIGKILL));
        assert!(start.elapsed() < Duration::from_secs(20));
        assert!(!dir.exists());
        assert!(runtime.join("hive.sock").exists());
        assert_eq!(
            stranger.try_wait().unwrap(),
            None,
            "the stranger was killed"
        );
        let _ = stranger.kill();
        let _ = stranger.wait();
        // No folder: nothing to do.
        clean(tmp.path(), &runtime);
    }

    #[test]
    fn groups_0_1_and_negative_are_never_killed_even_with_a_browsers_arguments() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("browsers");
        // A process table where each of them runs with its profile's arguments.
        let procs = tmp.path().join("proc");
        for (name, group) in [("a", 1), ("b", 0), ("c", -7), ("d", 4242)] {
            let profile = dir.join(name);
            std::fs::create_dir_all(&profile).unwrap();
            std::fs::write(profile.join(GROUP_FILE), group.to_string()).unwrap();
            let entry = procs.join(group.to_string());
            std::fs::create_dir_all(&entry).unwrap();
            let cmdline = format!("chrome\0--user-data-dir={}\0", profile.display());
            std::fs::write(entry.join("cmdline"), cmdline).unwrap();
        }
        let mut killed = Vec::new();
        reap(&dir, crate::procs::Source::Dir(&procs), &mut |g| {
            killed.push(g)
        });
        assert_eq!(killed, [4242]);
    }
}
