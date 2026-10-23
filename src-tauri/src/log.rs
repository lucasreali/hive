//! The app's log (D18.6): on macOS its stderr goes to `~/Library/Logs/Hive/app.log`, the
//! previous launch's kept as `app.log.1`, and a panic is written there with a backtrace. A GUI
//! app's stderr goes nowhere otherwise. The service's `daemon.log` is the bridge's
//! (`hive::bridge`): the app does not link the service, so the few lines are not shared.

use std::backtrace::Backtrace;
use std::ffi::OsString;
use std::fs::{DirBuilder, File};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

unsafe extern "C" {
    fn dup2(from: i32, to: i32) -> i32;
}

/// `~/Library/Logs/Hive/app.log`; `app.dev.log` for a development build, which runs beside the
/// installed app (17.18) and so must not rotate its log. None without a `HOME`.
pub fn path(home: Option<OsString>, dev: bool) -> Option<PathBuf> {
    let home = PathBuf::from(home.filter(|home| !home.is_empty())?);
    let name = if dev { "app.dev.log" } else { "app.log" };
    Some(home.join("Library/Logs/Hive").join(name))
}

/// Opens `path` for this launch (its folder made `0700` if missing), the previous launch's log
/// kept as `<path>.1` and any older one dropped. Appends: should the rename fail, the previous
/// log is kept in place.
pub fn open(path: &Path) -> io::Result<File> {
    if let Some(folder) = path.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)?;
    }
    let mut previous = path.as_os_str().to_owned();
    previous.push(".1");
    // A missing log (first launch) or a failed rename only means nothing is rotated.
    let _ = std::fs::rename(path, previous);
    File::options()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

/// Sends what is written to descriptor `to` (2, stderr, in the app) to the log of `home`
/// ([`path`], [`open`]). Nothing without a `HOME`. Called once the single-instance check
/// passed, so a second launch, which hands over and ends, never rotates the running app's log.
pub fn start(home: Option<OsString>, dev: bool, to: i32) -> io::Result<()> {
    let Some(path) = path(home, dev) else {
        return Ok(());
    };
    let file = open(&path)?;
    // SAFETY: both are open descriptors; `to` then refers to the log, `file` closes its own.
    if unsafe { dup2(file.as_raw_fd(), to) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// A panic hook that hands `write` the time, the thread, the panic and a backtrace, in one
/// piece (a release build aborts right after). `main` installs it writing to stderr, which
/// [`start`] sends to the log.
pub fn hook(
    write: impl Fn(&str) + Send + Sync + 'static,
) -> Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static> {
    Box::new(move |info| {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        write(&format!(
            "hive-app: panic at {time} (Unix time) in thread '{name}': {info}\n{}\n",
            Backtrace::force_capture()
        ));
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};

    #[test]
    fn the_log_is_in_the_users_logs_folder() {
        assert_eq!(
            path(Some("/Users/me".into()), false),
            Some(PathBuf::from("/Users/me/Library/Logs/Hive/app.log"))
        );
        assert_eq!(
            path(Some("/Users/me".into()), true),
            Some(PathBuf::from("/Users/me/Library/Logs/Hive/app.dev.log"))
        );
        assert_eq!(path(Some("".into()), false), None);
        assert_eq!(path(None, false), None);
    }

    #[test]
    fn each_launch_keeps_the_previous_log() {
        let tmp = tempfile_dir();
        let log = tmp.join("Logs/Hive/app.log");
        let previous = tmp.join("Logs/Hive/app.log.1");
        let read = |path: &Path| std::fs::read_to_string(path).unwrap();
        for run in ["first", "second", "third"] {
            write!(open(&log).unwrap(), "{run}").unwrap();
        }
        assert_eq!(read(&log), "third");
        assert_eq!(read(&previous), "second");
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&log), 0o600);
        assert_eq!(mode(log.parent().unwrap()), 0o700);
        // A rename that fails (`app.log.1` is a folder with something in it) loses nothing.
        std::fs::remove_file(&previous).unwrap();
        std::fs::create_dir_all(previous.join("x")).unwrap();
        write!(open(&log).unwrap(), "+fourth").unwrap();
        assert_eq!(read(&log), "third+fourth");
        // A folder that cannot be made fails.
        std::fs::write(tmp.join("file"), "").unwrap();
        assert!(open(&tmp.join("file/app.log")).is_err());
        std::fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn start_sends_a_descriptor_to_the_log() {
        let tmp = tempfile_dir();
        let home = tmp.join("home");
        // A descriptor of this test's own stands for stderr.
        let mut stand_in = File::create(tmp.join("stand-in")).unwrap();
        start(Some(home.clone().into()), false, stand_in.as_raw_fd()).unwrap();
        stand_in.write_all(b"to the log").unwrap();
        let log = home.join("Library/Logs/Hive/app.log");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "to the log");
        assert_eq!(std::fs::read_to_string(tmp.join("stand-in")).unwrap(), "");
        // Without a `HOME`, nothing; a log that cannot be opened, or a bad descriptor, fails.
        start(None, false, -1).unwrap();
        std::fs::write(tmp.join("file"), "").unwrap();
        assert!(start(Some(tmp.join("file").into()), false, stand_in.as_raw_fd()).is_err());
        let err = start(Some(home.into()), false, -1).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(9), "EBADF: {err}");
        std::fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn a_panic_is_written_with_its_backtrace() {
        let out = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&out);
        let previous = std::panic::take_hook();
        std::panic::set_hook(hook(move |text| sink.lock().unwrap().push_str(text)));
        let panicked = std::thread::Builder::new()
            .name("logged".into())
            .spawn(|| panic!("boom"))
            .unwrap()
            .join();
        std::panic::set_hook(previous);
        assert!(panicked.is_err());
        let text = out.lock().unwrap().clone();
        // Another test's panic in the meantime would be written too: look for this one's.
        let start = text.find("in thread 'logged'").unwrap();
        let entry = &text[start..];
        assert!(entry.contains("panicked at"), "{entry}");
        assert!(entry.contains("boom\n"), "{entry}");
        assert!(text.starts_with("hive-app: panic at "), "{text}");
        let time: u64 = text["hive-app: panic at ".len()..]
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(time > 1_700_000_000, "{time}");
        // The backtrace follows, on lines of its own.
        assert!(entry.lines().count() > 3, "{entry}");
    }

    /// A fresh folder under the system's temporary folder (the app has no `tempfile`).
    fn tempfile_dir() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("hive-app-log-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
