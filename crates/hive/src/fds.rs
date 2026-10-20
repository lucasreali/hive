//! The service's descriptor limit (17.6). A process started from a GUI app on macOS gets a
//! soft limit of 256, which a large worktree or ~30 terminals reach: terminals and socket
//! accepts then fail with `EMFILE`. The service raises it once, at start.

use std::io;

use nix::sys::resource::{Resource, getrlimit, setrlimit};

/// The highest soft limit asked for: macOS's `OPEN_MAX` (`<sys/syslimits.h>`), above which its `setrlimit` refuses
/// `RLIMIT_NOFILE` (`RLIM_INFINITY` included). The same cap elsewhere: far above what the
/// service holds.
pub const MOST: u64 = 10_240;

/// `RLIMIT_NOFILE`: its soft and hard values.
pub trait Limit {
    fn get(&self) -> io::Result<(u64, u64)>;
    fn set(&self, soft: u64, hard: u64) -> io::Result<()>;
}

/// This process's.
pub struct Process;

impl Limit for Process {
    fn get(&self) -> io::Result<(u64, u64)> {
        Ok(getrlimit(Resource::RLIMIT_NOFILE)?)
    }

    fn set(&self, soft: u64, hard: u64) -> io::Result<()> {
        Ok(setrlimit(Resource::RLIMIT_NOFILE, soft, hard)?)
    }
}

/// Raises the soft limit to the hard one, at most [`MOST`], and never lowers it: the soft
/// limit now.
pub fn raise(limit: &impl Limit) -> io::Result<u64> {
    let (soft, hard) = limit.get()?;
    let want = hard.min(MOST);
    if want <= soft {
        return Ok(soft);
    }
    limit.set(want, hard)?;
    Ok(want)
}

/// [`raise`] at start, a failure written to `log`: the service still runs on what it has.
pub fn raise_at_start(limit: &impl Limit, log: &mut impl io::Write) {
    if let Err(err) = raise(limit) {
        let _ = writeln!(
            log,
            "hive: warning: cannot raise the descriptor limit: {err}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A limit held in memory; `fail` makes `get` or `set` fail.
    struct Fake {
        now: Cell<(u64, u64)>,
        fail: Option<&'static str>,
        sets: Cell<usize>,
    }

    impl Fake {
        fn new(soft: u64, hard: u64) -> Self {
            Self {
                now: Cell::new((soft, hard)),
                fail: None,
                sets: Cell::new(0),
            }
        }
    }

    impl Limit for Fake {
        fn get(&self) -> io::Result<(u64, u64)> {
            match self.fail {
                Some("get") => Err(io::Error::from_raw_os_error(nix::libc::EPERM)),
                _ => Ok(self.now.get()),
            }
        }

        fn set(&self, soft: u64, hard: u64) -> io::Result<()> {
            match self.fail {
                Some("set") => Err(io::Error::from_raw_os_error(nix::libc::EINVAL)),
                _ => {
                    self.now.set((soft, hard));
                    self.sets.set(self.sets.get() + 1);
                    Ok(())
                }
            }
        }
    }

    #[test]
    fn the_soft_limit_rises_to_the_hard_one_within_the_cap() {
        let low = Fake::new(256, u64::MAX);
        assert_eq!(raise(&low).unwrap(), MOST);
        assert_eq!(low.now.get(), (MOST, u64::MAX));
        let hard = Fake::new(256, 4096);
        assert_eq!(raise(&hard).unwrap(), 4096);
        assert_eq!(hard.now.get(), (4096, 4096));
        // Never lowered.
        let high = Fake::new(MOST + 1, u64::MAX);
        assert_eq!(raise(&high).unwrap(), MOST + 1);
        assert_eq!(high.now.get(), (MOST + 1, u64::MAX));
        assert_eq!(high.sets.get(), 0);
        let at = Fake::new(MOST, MOST);
        assert_eq!(raise(&at).unwrap(), MOST);
        assert_eq!(at.sets.get(), 0);
        assert_eq!(low.sets.get(), 1);
    }

    #[test]
    fn a_limit_that_cannot_be_raised_is_only_logged() {
        for fail in ["get", "set"] {
            let mut limit = Fake::new(256, u64::MAX);
            limit.fail = Some(fail);
            assert!(raise(&limit).is_err());
            let mut log = Vec::new();
            raise_at_start(&limit, &mut log);
            let log = String::from_utf8(log).unwrap();
            let want = "hive: warning: cannot raise the descriptor limit: ";
            assert!(log.starts_with(want), "{log}");
            assert_eq!(limit.now.get(), (256, u64::MAX));
        }
        let mut log = Vec::new();
        raise_at_start(&Fake::new(256, 512), &mut log);
        assert!(log.is_empty());
    }

    /// This process's soft limit, as a child shell sees it.
    fn shell_soft() -> String {
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", "ulimit -n"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn this_process_limit_is_raised() {
        let (soft, hard) = Process.get().unwrap();
        let most = hard.min(MOST);
        // One below what it can be, as a child sees.
        Process.set(most - 1, hard).unwrap();
        assert_eq!(Process.get().unwrap(), (most - 1, hard));
        assert_eq!(shell_soft(), (most - 1).to_string());
        raise_at_start(&Process, &mut io::stderr());
        assert_eq!(Process.get().unwrap(), (most, hard));
        assert_eq!(shell_soft(), most.to_string());
        // Never lower than it was.
        Process.set(soft.max(most), hard).unwrap();
    }
}
