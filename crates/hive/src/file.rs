//! One file of a worktree for the viewer and diff (#31): its text on disk and at `HEAD`.
//! Git runs as the executable with separate arguments.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::hash::{DefaultHasher, Hasher};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use hive_protocol::{Control, FileStatus, MAX_PAYLOAD, SaveError};

use crate::changes::{BINARY_PROBE, parse_status};
use crate::git;

/// Most bytes of each side (on disk, at `HEAD`) sent to the app.
pub const TEXT_LIMIT: u64 = 1_048_576; // 1 MiB
/// Longest `path` accepted, Linux's `PATH_MAX`.
const PATH_LIMIT: usize = 4096;

/// One side of a file.
#[derive(Debug, PartialEq, Eq)]
pub enum Side {
    Missing,
    TooLarge,
    Bytes(Vec<u8>),
}

/// The file `path` of the worktree at `dir`: on disk and at `HEAD`. `path` comes from the app:
/// it must be relative, without `..`, and must not resolve (through symlinks) outside `dir`.
pub fn read(dir: &Path, path: &str) -> io::Result<(Side, Side)> {
    Ok((on_disk(dir, relative(path)?)?, at_head(dir, path)?))
}

/// `path` from the app: relative, without `.` or `..`, at most [`PATH_LIMIT`] bytes.
fn relative(path: &str) -> io::Result<&Path> {
    let rel = Path::new(path);
    let normal = rel.components().all(|c| matches!(c, Component::Normal(_)));
    if path.is_empty() || path.len() > PATH_LIMIT || !normal {
        return Err(io::Error::other("not a relative path inside the worktree"));
    }
    Ok(rel)
}

/// `path` resolved (symlinks followed), when it stays inside `dir`.
fn inside(dir: &Path, path: &Path) -> io::Result<PathBuf> {
    let real = path.canonicalize()?;
    if !real.starts_with(dir.canonicalize()?) {
        return Err(io::Error::other("the file resolves outside the worktree"));
    }
    Ok(real)
}

/// The regular file `rel` of `dir`, resolved; `None` when it does not exist.
fn resolve(dir: &Path, rel: &Path) -> io::Result<Option<PathBuf>> {
    let real = match inside(dir, &dir.join(rel)) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        real => real?,
    };
    // Never open a FIFO or a device: reading it could block forever.
    if !real.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    Ok(Some(real))
}

fn on_disk(dir: &Path, rel: &Path) -> io::Result<Side> {
    match resolve(dir, rel)? {
        Some(real) => limited(&mut File::open(real)?),
        None => Ok(Side::Missing),
    }
}

/// Where a save writes `rel`: the file itself (through symlinks), or a new file in a folder
/// that resolves inside `dir`; with the file's permission bits when it exists.
fn target(dir: &Path, rel: &Path) -> io::Result<(PathBuf, Option<u32>)> {
    if let Some(real) = resolve(dir, rel)? {
        let mode = real.metadata()?.permissions().mode();
        return Ok((real, Some(mode)));
    }
    // `rel` has only normal components, so it has a parent (maybe `dir`) and a name.
    let joined = dir.join(rel);
    let folder = inside(dir, joined.parent().unwrap_or(dir))?;
    Ok((folder.join(joined.file_name().unwrap_or_default()), None))
}

/// Temporary files of saves in progress, so two never share a name.
static SAVES: AtomicU64 = AtomicU64::new(0);

/// Writes `content` over the file `path` of the worktree at `dir`, only if its bytes on disk
/// still have `version` (`None`: the file must not exist). The text goes to a temporary file
/// in the same folder (0600, then the file's own mode), synced, and renamed over the file;
/// the version is checked right before the rename. Returns the new version.
pub fn save(
    dir: &Path,
    path: &str,
    content: &str,
    version: Option<&str>,
) -> Result<String, (SaveError, String)> {
    save_with(dir, path, content, version, |from, to| fs::rename(from, to))
}

/// [`save`] with the rename passed in, so its failure can be tested.
pub fn save_with(
    dir: &Path,
    path: &str,
    content: &str,
    version: Option<&str>,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<String, (SaveError, String)> {
    let invalid = |err: io::Error| (SaveError::InvalidPath, err.to_string());
    let rel = relative(path).map_err(invalid)?;
    if content.len() as u64 > TEXT_LIMIT {
        return Err((SaveError::TooLarge, format!("{path} is over 1 MiB")));
    }
    let (target, mode) = target(dir, rel).map_err(invalid)?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let n = SAVES.fetch_add(1, Ordering::Relaxed);
    let temp = target.with_file_name(format!(".{name}.hive-{}-{n}.tmp", std::process::id()));
    let io = |err: io::Error| (SaveError::Io, err.to_string());
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(io)?;
    let saved = (|| {
        write_temp(file, content, mode.unwrap_or(0o644)).map_err(io)?;
        let conflict = || (SaveError::Conflict, format!("{path} changed on disk"));
        let now = match File::open(&target) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            // ponytail: a writer between this check and the rename is lost; a lock would not
            // help, since agents do not take one.
            opened => match limited(&mut opened.map_err(io)?).map_err(io)? {
                Side::Bytes(bytes) => Some(self::version(&bytes)),
                _ => return Err(conflict()),
            },
        };
        if now.as_deref() != version {
            return Err(conflict());
        }
        rename(&temp, &target).map_err(io)
    })();
    if saved.is_err() {
        let _ = fs::remove_file(&temp);
    }
    saved?;
    // The rename itself is durable once the folder is synced; the text already is.
    let folder = target.parent().unwrap_or(dir);
    let _ = File::open(folder).and_then(|folder| folder.sync_all());
    Ok(self::version(content.as_bytes()))
}

fn write_temp(mut file: File, content: &str, mode: u32) -> io::Result<()> {
    file.write_all(content.as_bytes())?;
    file.set_permissions(fs::Permissions::from_mode(mode & 0o7777))?;
    file.sync_all()
}

/// The system that opens files for the app: Windows through WSL, or macOS itself.
#[cfg(target_os = "linux")]
const SYSTEM: &str = "Windows";
#[cfg(target_os = "macos")]
const SYSTEM: &str = "macOS";

/// Extensions that the system runs, installs or follows instead of opening them in an editor.
// ponytail: a fixed list, not the user's file associations; extend it when one is missing.
#[cfg(target_os = "linux")]
const RUNS_AS_PROGRAM: &[&str] = &[
    "appref-ms",
    "application",
    "bat",
    "cmd",
    "com",
    "cpl",
    "exe",
    "hta",
    "inf",
    "jar",
    "js",
    "jse",
    "lnk",
    "msc",
    "msi",
    "msp",
    "pif",
    "ps1",
    "reg",
    "scf",
    "scr",
    "url",
    "vbe",
    "vbs",
    "ws",
    "wsf",
    "wsh",
];
#[cfg(target_os = "macos")]
const RUNS_AS_PROGRAM: &[&str] = &[
    "app", "command", "jar", "pkg", "scpt", "terminal", "tool", "workflow",
];

/// The path the app opens (see [`windows`]) of the file `path` of the worktree at `dir` (the
/// folder itself when `path` is empty), with the system's default app. Refused for a file
/// the system would run.
pub fn windows_path(dir: &Path, path: &str, wslpath: &OsStr) -> io::Result<String> {
    let real = if path.is_empty() {
        // The worktree's own folder, for the Windows Explorer.
        dir.to_path_buf()
    } else {
        let Some(real) = resolve(dir, relative(path)?)? else {
            return Err(io::Error::other(format!("{path} does not exist")));
        };
        real
    };
    let extension = real.extension().unwrap_or_default().to_string_lossy();
    let extension = extension.to_ascii_lowercase();
    if RUNS_AS_PROGRAM.contains(&extension.as_str()) {
        return Err(io::Error::other(format!(
            "{SYSTEM} would run a .{extension} file instead of opening it in an editor"
        )));
    }
    windows(&real, wslpath)
}

/// Where Windows sees `path`: `<wslpath> -w <path>`.
#[cfg(target_os = "linux")]
pub fn windows(path: &Path, wslpath: &OsStr) -> io::Result<String> {
    let out = std::process::Command::new(wslpath)
        .arg("-w")
        .arg(path)
        .output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(io::Error::other(format!(
            "wslpath failed: {}",
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
}

#[cfg(target_os = "macos")]
pub use crate::macos::native_path as windows;

/// At most [`TEXT_LIMIT`] bytes, else [`Side::TooLarge`].
pub fn limited(input: &mut dyn Read) -> io::Result<Side> {
    let mut bytes = Vec::new();
    input.take(TEXT_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > TEXT_LIMIT {
        return Ok(Side::TooLarge);
    }
    Ok(Side::Bytes(bytes))
}

/// The blob at `HEAD`, looked up under the old path for a staged rename.
fn at_head(dir: &Path, path: &str) -> io::Result<Side> {
    if git::output(dir, &["rev-parse", "--verify", "--quiet", "HEAD"], &[0, 1])?.is_empty() {
        return Ok(Side::Missing);
    }
    let blob = match blob(dir, path)? {
        Some(blob) => Some(blob),
        None => match renamed_from(dir, path)? {
            Some(old) => blob(dir, &old)?,
            None => None,
        },
    };
    let Some((oid, size)) = blob else {
        return Ok(Side::Missing);
    };
    if size > TEXT_LIMIT {
        return Ok(Side::TooLarge);
    }
    git::output(dir, &["cat-file", "blob", &oid], &[0]).map(Side::Bytes)
}

/// The id and size of the blob at `path` in `HEAD`.
fn blob(dir: &Path, path: &str) -> io::Result<Option<(String, u64)>> {
    let args = [
        "--literal-pathspecs",
        "ls-tree",
        "-l",
        "-z",
        "HEAD",
        "--",
        path,
    ];
    Ok(parse_ls_tree(&git::output(dir, &args, &[0])?))
}

/// The first entry of `git ls-tree -l -z` (`<mode> <type> <id> <size>\t<path>`) if it is a
/// blob.
pub fn parse_ls_tree(out: &[u8]) -> Option<(String, u64)> {
    let meta = out.split(|&b| b == b'\t').next()?;
    let meta = String::from_utf8_lossy(meta);
    let fields: Vec<&str> = meta.split_whitespace().collect();
    match fields[..] {
        [_, "blob", oid, size] => Some((oid.to_owned(), size.parse().ok()?)),
        _ => None,
    }
}

/// Where a staged rename to `path` came from.
fn renamed_from(dir: &Path, path: &str) -> io::Result<Option<String>> {
    let args = [
        "status",
        "--porcelain=v2",
        "-z",
        "--untracked-files=no",
        "--find-renames",
    ];
    let status = git::output(dir, &args, &[0])?;
    Ok(parse_status(&status)
        .into_iter()
        .find(|(to, status, _)| to == path.as_bytes() && *status == FileStatus::Renamed)
        .and_then(|(_, _, from)| String::from_utf8(from?).ok()))
}

/// The token for the bytes on disk (3.5 compares it before saving): their length and 64-bit
/// SipHash, stable within one build of the service.
pub fn version(bytes: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    format!("{}-{:016x}", bytes.len(), hasher.finish())
}

/// Text, or `Err` for binary: a NUL in the first 8000 bytes (as git) or not UTF-8.
fn text(side: &Side) -> Result<Option<String>, ()> {
    match side {
        Side::Bytes(bytes) if bytes[..bytes.len().min(BINARY_PROBE)].contains(&0) => Err(()),
        Side::Bytes(bytes) => String::from_utf8(bytes.clone()).map(Some).map_err(drop),
        _ => Ok(None),
    }
}

/// The `file` answer for `path`. When the texts would not fit in one frame, they are left out
/// as too large.
pub fn message(worktree: String, path: String, read: io::Result<(Side, Side)>) -> Control {
    let (disk, head, error) = match read {
        Ok((Side::Missing, Side::Missing)) => {
            let error = format!("{path} does not exist");
            (Side::Missing, Side::Missing, Some(error))
        }
        Ok((disk, head)) => (disk, head, None),
        Err(err) => (Side::Missing, Side::Missing, Some(err.to_string())),
    };
    let version = match &disk {
        Side::Bytes(bytes) => Some(version(bytes)),
        _ => None,
    };
    let mut too_large = disk == Side::TooLarge || head == Side::TooLarge;
    let (content, base) = (text(&disk), text(&head));
    let binary = content.is_err() || base.is_err();
    let texts = |ok: bool| match (content.clone(), base.clone()) {
        (Ok(content), Ok(base)) if ok => (content, base),
        _ => (None, None),
    };
    let file = |(content, base), too_large| Control::File {
        worktree: worktree.clone(),
        path: path.clone(),
        content,
        base,
        version: version.clone(),
        binary,
        too_large,
        error: error.clone(),
    };
    let reply = file(texts(!too_large), too_large);
    if serde_json::to_vec(&reply).map_or(0, |json| json.len()) <= MAX_PAYLOAD {
        return reply;
    }
    too_large = true;
    file(texts(false), too_large)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn file(content: Option<&str>, base: Option<&str>) -> Control {
        Control::File {
            worktree: "/w".into(),
            path: "a".into(),
            content: content.map(Into::into),
            base: base.map(Into::into),
            version: content.map(|c| version(c.as_bytes())),
            binary: false,
            too_large: false,
            error: None,
        }
    }

    fn answer(read: io::Result<(Side, Side)>) -> Control {
        message("/w".into(), "a".into(), read)
    }

    /// The answer's fields by name.
    fn fields(reply: Control) -> serde_json::Value {
        serde_json::to_value(reply).unwrap()
    }

    fn bytes(text: &str) -> Side {
        Side::Bytes(text.into())
    }

    #[test]
    fn paths_must_stay_inside_the_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("out")).unwrap();
        std::os::unix::fs::symlink("gone", dir.path().join("dangling")).unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let refused = |path: &str| read(dir.path(), path).unwrap_err().to_string();
        let not_inside = "not a relative path inside the worktree";
        for path in ["", "/etc/passwd", "../x", "sub/../../x", "./a", "sub/.."] {
            assert_eq!(refused(path), not_inside, "{path:?}");
        }
        assert_eq!(refused(&"a".repeat(PATH_LIMIT + 1)), not_inside);
        assert_eq!(refused("out"), "the file resolves outside the worktree");
        assert_eq!(refused("sub"), "not a regular file");
        let fifo = dir.path().join("fifo");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        assert!(made.unwrap().success());
        assert_eq!(refused("fifo"), "not a regular file");
        // Neither exists nor is in a repository: what is on disk is read before git runs.
        assert_eq!(
            on_disk(dir.path(), Path::new("dangling")).unwrap(),
            Side::Missing
        );
        assert!(refused("missing").starts_with("git rev-parse"));
        std::fs::write(dir.path().join("sub/f"), "text").unwrap();
        assert_eq!(
            on_disk(dir.path(), Path::new("sub/f")).unwrap(),
            bytes("text")
        );
        assert!(on_disk(dir.path(), Path::new("sub/f/x")).is_err());
        assert_eq!(
            read(dir.path(), &"a".repeat(PATH_LIMIT))
                .unwrap_err()
                .kind(),
            {
                // A name longer than a path component allows.
                io::ErrorKind::InvalidFilename
            }
        );
    }

    /// The names in `dir`, sorted: a save leaves no temporary file behind.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn mode(path: &Path) -> u32 {
        path.metadata().unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn a_save_replaces_the_file_keeping_its_mode() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.sh");
        std::fs::write(&file, "old").unwrap();
        std::fs::set_permissions(&file, fs::Permissions::from_mode(0o750)).unwrap();
        let saved = save(dir.path(), "f.sh", "new", Some(&version(b"old")));
        assert_eq!(saved, Ok(version(b"new")));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(mode(&file), 0o750);

        // Through a symlink inside the worktree, the file it points to is written.
        std::os::unix::fs::symlink("f.sh", dir.path().join("link")).unwrap();
        let saved = save(dir.path(), "link", "newer", Some(&version(b"new")));
        assert_eq!(saved, Ok(version(b"newer")));
        assert!(dir.path().join("link").is_symlink());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "newer");
        assert_eq!(names(dir.path()), ["f.sh", "link"]);
    }

    #[test]
    fn a_save_needs_the_version_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "disk").unwrap();
        let conflict = Err((SaveError::Conflict, "f changed on disk".to_owned()));
        assert_eq!(save(dir.path(), "f", "x", Some(&version(b"old"))), conflict);
        assert_eq!(save(dir.path(), "f", "x", None), conflict);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "disk");

        // `None` creates a missing file (0644); a version for a missing file is a conflict.
        let gone = Err((SaveError::Conflict, "n changed on disk".to_owned()));
        assert_eq!(save(dir.path(), "n", "x", Some(&version(b"x"))), gone);
        assert_eq!(save(dir.path(), "n", "x", None), Ok(version(b"x")));
        assert_eq!(mode(&dir.path().join("n")), 0o644);

        // A file over the limit on disk has no version the app could hold.
        std::fs::write(&file, vec![b'a'; TEXT_LIMIT as usize + 1]).unwrap();
        assert_eq!(
            save(dir.path(), "f", "x", Some(&version(b"disk"))),
            conflict
        );
        assert_eq!(names(dir.path()), ["f", "n"]);
    }

    #[test]
    fn a_save_is_refused_outside_the_worktree_or_over_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
        let refused = |path: &str| save(dir.path(), path, "x", None).unwrap_err();
        let invalid = |message: &str| (SaveError::InvalidPath, message.to_owned());
        let not_inside = invalid("not a relative path inside the worktree");
        assert_eq!(refused("../x"), not_inside);
        assert_eq!(refused("/etc/x"), not_inside);
        assert_eq!(refused("sub"), invalid("not a regular file"));
        let outside_file = invalid("the file resolves outside the worktree");
        assert_eq!(refused("out/new"), outside_file);
        assert_eq!(refused("missing/new").0, SaveError::InvalidPath);
        assert_eq!(names(outside.path()), Vec::<String>::new());

        let at_limit = "a".repeat(TEXT_LIMIT as usize);
        assert!(save(dir.path(), "big", &at_limit, None).is_ok());
        let over = format!("{at_limit}a");
        let too_large = (SaveError::TooLarge, "big is over 1 MiB".to_owned());
        let saved = save(
            dir.path(),
            "big",
            &over,
            Some(&version(at_limit.as_bytes())),
        );
        assert_eq!(saved, Err(too_large));
    }

    #[test]
    fn a_failed_save_leaves_the_file_and_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "old").unwrap();
        let v = version(b"old");
        let boom = |_: &Path, _: &Path| Err(io::Error::other("boom"));
        let failed = save_with(dir.path(), "f", "new", Some(&v), boom);
        assert_eq!(failed, Err((SaveError::Io, "boom".to_owned())));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "old");
        assert_eq!(names(dir.path()), ["f"]);

        // An unreadable file cannot be checked.
        std::fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
        let (error, _) = save(dir.path(), "f", "new", Some(&v)).unwrap_err();
        assert_eq!(error, SaveError::Io);
        assert_eq!(names(dir.path()), ["f"]);

        // A folder that cannot be written to holds no temporary file.
        std::fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let (error, message) = save(dir.path(), "f", "new", Some(&v)).unwrap_err();
        std::fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(error, SaveError::Io, "{message}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "old");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_file_is_located_for_windows_unless_windows_would_run_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), "").unwrap();
        std::fs::write(dir.path().join("run.BAT"), "").unwrap();
        let real = dir.path().canonicalize().unwrap().join("a.ts");
        let at = |path: &str, program: &str| {
            windows_path(dir.path(), path, OsStr::new(program)).map_err(|e| e.to_string())
        };
        // `echo` stands in for `wslpath`: it prints its arguments.
        assert_eq!(at("a.ts", "echo"), Ok(format!("-w {}", real.display())));
        // An empty path is the worktree's folder.
        assert_eq!(at("", "echo"), Ok(format!("-w {}", dir.path().display())));
        let run = "Windows would run a .bat file instead of opening it in an editor";
        assert_eq!(at("run.BAT", "echo"), Err(run.to_owned()));
        assert_eq!(at("nope", "echo"), Err("nope does not exist".to_owned()));
        let not_inside = "not a relative path inside the worktree";
        assert_eq!(at("../a.ts", "echo"), Err(not_inside.to_owned()));
        assert_eq!(at("a.ts", "false"), Err("wslpath failed: ".to_owned()));
        assert!(at("a.ts", "/nonexistent/wslpath").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_file_is_located_on_macos_unless_macos_would_run_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), "").unwrap();
        std::fs::write(dir.path().join("Some.APP"), "").unwrap();
        let real = dir.path().canonicalize().unwrap().join("a.ts");
        let at = |path: &str| {
            windows_path(dir.path(), path, OsStr::new("wslpath")).map_err(|e| e.to_string())
        };
        assert_eq!(at("a.ts"), Ok(real.display().to_string()));
        assert_eq!(at(""), Ok(dir.path().display().to_string()));
        let run = "macOS would run a .app file instead of opening it in an editor";
        assert_eq!(at("Some.APP"), Err(run.to_owned()));
        assert_eq!(at("nope"), Err("nope does not exist".to_owned()));
    }

    #[test]
    fn a_side_is_read_up_to_the_limit() {
        let at_limit = vec![b'a'; TEXT_LIMIT as usize];
        assert_eq!(
            limited(&mut &at_limit[..]).unwrap(),
            Side::Bytes(at_limit.clone())
        );
        let over = vec![b'a'; TEXT_LIMIT as usize + 1];
        assert_eq!(limited(&mut &over[..]).unwrap(), Side::TooLarge);
    }

    #[test]
    fn ls_tree_entries_are_parsed() {
        let blob = b"100644 blob 3b18e512dba79e4c8300dd08aeb37f8e728b8dad      12\tst*r\0";
        let parsed = parse_ls_tree(blob);
        assert_eq!(
            parsed,
            Some(("3b18e512dba79e4c8300dd08aeb37f8e728b8dad".into(), 12))
        );
        assert_eq!(parse_ls_tree(b"040000 tree 3b18e5       -\tsub\0"), None);
        assert_eq!(parse_ls_tree(b"160000 commit 3b18e5       -\tmod\0"), None);
        assert_eq!(parse_ls_tree(b"100644 blob 3b18e5 x\ta\0"), None);
        assert_eq!(parse_ls_tree(b""), None);
    }

    #[test]
    fn versions_differ_with_the_bytes() {
        assert_eq!(version(b"abc"), version(b"abc"));
        assert_ne!(version(b"abc"), version(b"abd"));
        assert!(version(b"abc").starts_with("3-"), "{}", version(b"abc"));
        assert_eq!(version(b"abc").len(), 2 + 16);
    }

    #[test]
    fn the_answer_carries_both_texts() {
        assert_eq!(
            answer(Ok((bytes("new\n"), bytes("old\n")))),
            file(Some("new\n"), Some("old\n"))
        );
        assert_eq!(
            answer(Ok((bytes("new\n"), Side::Missing))),
            file(Some("new\n"), None)
        );
        assert_eq!(
            answer(Ok((Side::Missing, bytes("old\n")))),
            file(None, Some("old\n"))
        );
    }

    #[test]
    fn the_answer_says_why_there_is_no_text() {
        let missing = fields(answer(Ok((Side::Missing, Side::Missing))));
        assert_eq!(missing["error"], "a does not exist");
        let failed = fields(answer(Err(io::Error::other("nope"))));
        assert_eq!(
            (&failed["error"], &failed["version"]),
            (&json!("nope"), &json!(null))
        );

        let flags = |read| {
            let file = fields(answer(Ok(read)));
            assert_eq!(
                (&file["content"], &file["base"]),
                (&json!(null), &json!(null))
            );
            (
                file["version"].clone(),
                file["binary"].clone(),
                file["too_large"].clone(),
            )
        };
        let nul = Side::Bytes(b"a\0b".to_vec());
        let v = json!(super::version(b"a\0b"));
        assert_eq!(flags((nul, bytes("x"))), (v, json!(true), json!(false)));
        assert_eq!(
            flags((bytes("x"), Side::Bytes(b"\xff".to_vec()))).1,
            json!(true)
        );
        let mut late_nul = vec![b'a'; BINARY_PROBE];
        late_nul.push(0);
        let (_, binary, _) = flags((Side::Bytes(late_nul), Side::TooLarge));
        assert_eq!(binary, json!(false), "a NUL after the probe is text");
        let null = json!(null);
        assert_eq!(
            flags((Side::TooLarge, bytes("x"))),
            (null.clone(), json!(false), json!(true))
        );
        assert_eq!(
            flags((Side::TooLarge, Side::Bytes(vec![0]))),
            (null, json!(true), json!(true))
        );
    }

    #[test]
    fn texts_that_do_not_fit_in_a_frame_are_too_large() {
        // Every quote doubles when escaped: two sides of 1 MiB are over 4 MiB of JSON.
        let quotes = "\"".repeat(TEXT_LIMIT as usize);
        let both = fields(answer(Ok((bytes(&quotes), bytes(&quotes)))));
        let shown = (&both["content"], &both["base"], &both["too_large"]);
        assert_eq!(shown, (&json!(null), &json!(null), &json!(true)));
        assert_eq!(both["version"], super::version(quotes.as_bytes()));
        // One side alone fits.
        let one = fields(answer(Ok((bytes(&quotes), Side::Missing))));
        assert_eq!(one["too_large"], false);

        // Exactly one frame still fits; one more escaped byte does not.
        let json = |base: &str| {
            let reply = answer(Ok((bytes(&quotes), bytes(base))));
            (serde_json::to_vec(&reply).unwrap().len(), reply)
        };
        let letters = "a".repeat(TEXT_LIMIT as usize);
        let spare = MAX_PAYLOAD - json(&letters).0;
        let base = |n| format!("{}{}", "\"".repeat(n), &letters[n..]);
        let (len, reply) = json(&base(spare));
        assert_eq!(len, MAX_PAYLOAD);
        assert!(matches!(
            reply,
            Control::File {
                too_large: false,
                ..
            }
        ));
        let (_, reply) = json(&base(spare + 1));
        assert!(matches!(
            reply,
            Control::File {
                too_large: true,
                ..
            }
        ));
    }
}
