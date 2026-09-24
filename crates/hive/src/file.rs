//! One file of a worktree for the viewer and diff (#31): its text on disk and at `HEAD`.
//! Git runs as the executable with separate arguments.

use std::fs::File;
use std::hash::{DefaultHasher, Hasher};
use std::io::{self, Read};
use std::path::{Component, Path};

use hive_protocol::{Control, FileStatus, MAX_PAYLOAD};

use crate::changes::{BINARY_PROBE, git_ok, parse_status};

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
    let rel = Path::new(path);
    let normal = rel.components().all(|c| matches!(c, Component::Normal(_)));
    if path.is_empty() || path.len() > PATH_LIMIT || !normal {
        return Err(io::Error::other("not a relative path inside the worktree"));
    }
    Ok((on_disk(dir, rel)?, at_head(dir, path)?))
}

fn on_disk(dir: &Path, rel: &Path) -> io::Result<Side> {
    let real = match dir.join(rel).canonicalize() {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Side::Missing),
        real => real?,
    };
    if !real.starts_with(dir.canonicalize()?) {
        return Err(io::Error::other("the file resolves outside the worktree"));
    }
    // Never open a FIFO or a device: reading it could block forever.
    if !real.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    limited(&mut File::open(real)?)
}

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
    if git_ok(dir, &["rev-parse", "--verify", "--quiet", "HEAD"], &[0, 1])?.is_empty() {
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
    git_ok(dir, &["cat-file", "blob", &oid], &[0]).map(Side::Bytes)
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
    Ok(parse_ls_tree(&git_ok(dir, &args, &[0])?))
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
    let status = git_ok(dir, &args, &[0])?;
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
        let Control::File { error, .. } = answer(Ok((Side::Missing, Side::Missing))) else {
            panic!("expected a file")
        };
        assert_eq!(error.as_deref(), Some("a does not exist"));
        let Control::File { error, version, .. } = answer(Err(io::Error::other("nope"))) else {
            panic!("expected a file")
        };
        assert_eq!((error.as_deref(), version), (Some("nope"), None));

        let flags = |read| match answer(Ok(read)) {
            Control::File {
                content,
                base,
                version,
                binary,
                too_large,
                ..
            } => {
                assert_eq!((content, base), (None, None));
                (version, binary, too_large)
            }
            other => panic!("{other:?}"),
        };
        let nul = Side::Bytes(b"a\0b".to_vec());
        let v = Some(super::version(b"a\0b"));
        assert_eq!(flags((nul, bytes("x"))), (v, true, false));
        assert_eq!(flags((bytes("x"), Side::Bytes(b"\xff".to_vec()))).1, true);
        let mut late_nul = vec![b'a'; BINARY_PROBE];
        late_nul.push(0);
        let (_, binary, _) = flags((Side::Bytes(late_nul), Side::TooLarge));
        assert!(!binary, "a NUL after the probe is text");
        assert_eq!(flags((Side::TooLarge, bytes("x"))), (None, false, true));
        assert_eq!(
            flags((Side::TooLarge, Side::Bytes(vec![0]))),
            (None, true, true)
        );
    }

    #[test]
    fn texts_that_do_not_fit_in_a_frame_are_too_large() {
        // Every quote doubles when escaped: two sides of 1 MiB are over 4 MiB of JSON.
        let quotes = "\"".repeat(TEXT_LIMIT as usize);
        let Control::File {
            content,
            base,
            version,
            too_large,
            ..
        } = answer(Ok((bytes(&quotes), bytes(&quotes))))
        else {
            panic!("expected a file")
        };
        assert_eq!((content, base, too_large), (None, None, true));
        assert_eq!(version, Some(super::version(quotes.as_bytes())));
        // One side alone fits.
        let Control::File { too_large, .. } = answer(Ok((bytes(&quotes), Side::Missing))) else {
            panic!("expected a file")
        };
        assert!(!too_large);

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
