//! Browsing folders for "Add project": the subfolders of a typed folder, given as a Linux path
//! or, for the Windows side of WSL, as a Windows path converted with `wslpath -u`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use hive_protocol::{Control, Dir};

/// Longest path accepted, in bytes.
pub const PATH_LIMIT: usize = 4096;
/// Most subfolders sent.
pub const DIR_LIMIT: usize = 1000;

/// The programs run for the Windows side; tests pass stand-ins.
pub struct Windows<'a> {
    /// Prints the Windows user folder: `<cmd> /c echo %USERPROFILE%`.
    pub cmd: &'a str,
    /// Converts a Windows path: `<wslpath> -u <path>`.
    pub wslpath: &'a str,
}

pub const WINDOWS: Windows<'static> = Windows {
    cmd: "cmd.exe",
    wslpath: "wslpath",
};

/// Answers `ListDirs`: `path` (Windows when `windows`; the home folder when empty) as a Linux
/// path, and the subfolders of the folder it ends in, or of the one holding its last name.
pub fn answer(path: String, windows: bool, home: Option<&Path>, programs: &Windows) -> Control {
    match list(&path, windows, home, programs) {
        Ok((path, linux, parent, dirs)) => Control::Dirs {
            path,
            windows,
            linux_path: Some(linux),
            parent,
            dirs,
            error: None,
        },
        Err(err) => Control::Dirs {
            path,
            windows,
            linux_path: None,
            parent: None,
            dirs: Vec::new(),
            error: Some(err.to_string()),
        },
    }
}

/// The path as typed (or the home folder), as a Linux path, the folder above the listed one,
/// and the listed subfolders.
type Listing = (String, String, Option<String>, Vec<Dir>);

fn list(path: &str, windows: bool, home: Option<&Path>, programs: &Windows) -> io::Result<Listing> {
    if path.len() > PATH_LIMIT {
        return Err(io::Error::other(format!(
            "paths are at most {PATH_LIMIT} bytes"
        )));
    }
    let separators: &[char] = if windows { &['\\', '/'] } else { &['/'] };
    let path = match (path.is_empty(), windows) {
        (false, _) => path.to_owned(),
        (true, true) => with_separator(run(programs.cmd, &["/c", "echo", "%USERPROFILE%"])?, '\\'),
        (true, false) => {
            let home = home.ok_or_else(|| io::Error::other("HOME is not set"))?;
            with_separator(home.to_string_lossy().into_owned(), '/')
        }
    };
    let linux = if windows {
        run(programs.wslpath, &["-u", &path])?
    } else {
        path.clone()
    };
    if !linux.starts_with('/') {
        return Err(io::Error::other("type a full path"));
    }
    let typed = Path::new(&linux);
    let folder = if path.ends_with(separators) {
        typed
    } else {
        typed.parent().unwrap_or(typed)
    };
    let dirs = subfolders(folder)?;
    // The folder part of what was typed, and the one above it, in the same form.
    let above = path.rfind(separators).map_or("", |i| &path[..i]);
    let above = above.trim_end_matches(separators);
    let parent = above.rfind(separators).map(|i| path[..=i].to_owned());
    Ok((path, linux, parent, dirs))
}

fn with_separator(mut path: String, separator: char) -> String {
    if !path.ends_with(separator) {
        path.push(separator);
    }
    path
}

/// The visible subfolders of `folder` (links to folders included), sorted ignoring case, at
/// most [`DIR_LIMIT`].
fn subfolders(folder: &Path) -> io::Result<Vec<Dir>> {
    let entries = fs::read_dir(folder)
        .map_err(|err| io::Error::other(format!("cannot open {}: {err}", folder.display())))?;
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') && is_dir(&entry) {
            found.push((name, entry.path()));
        }
    }
    found.sort_by_cached_key(|(name, _)| (name.to_lowercase(), name.clone()));
    found.truncate(DIR_LIMIT);
    Ok(found
        .into_iter()
        .map(|(name, path)| Dir {
            git: path.join(".git").symlink_metadata().is_ok(),
            name,
        })
        .collect())
}

/// Reads the entry's type first: a stat per file is slow on the Windows side (drvfs).
fn is_dir(entry: &fs::DirEntry) -> bool {
    entry
        .file_type()
        .is_ok_and(|kind| kind.is_dir() || kind.is_symlink() && entry.path().is_dir())
}

/// `program`'s first line of output.
fn run(program: &str, args: &[&str]) -> io::Result<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| io::Error::other(format!("cannot run {program}: {err}")))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next().unwrap_or_default().trim_end();
    if !out.status.success() || line.is_empty() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(io::Error::other(format!(
            "{program} failed: {}",
            stderr.trim()
        )));
    }
    Ok(line.to_owned())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    fn dir(name: &str, git: bool) -> Dir {
        Dir {
            name: name.into(),
            git,
        }
    }

    /// What `answer` says, as (path, linux path, parent, dirs, error).
    type Answer = (
        String,
        Option<String>,
        Option<String>,
        Vec<Dir>,
        Option<String>,
    );

    fn ask(path: &str, windows: bool, home: Option<&Path>, programs: &Windows) -> Answer {
        // Read as JSON: a `let … else` would leave a never-run line.
        let got = serde_json::to_value(answer(path.into(), windows, home, programs)).unwrap();
        assert_eq!(
            (&got["type"], &got["windows"]),
            (&"dirs".into(), &windows.into())
        );
        let field = |key: &str| got[key].clone();
        (
            serde_json::from_value(field("path")).unwrap(),
            serde_json::from_value(field("linux_path")).unwrap(),
            serde_json::from_value(field("parent")).unwrap(),
            serde_json::from_value(field("dirs")).unwrap(),
            serde_json::from_value(field("error")).unwrap(),
        )
    }

    fn script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[test]
    fn subfolders_are_listed_sorted_without_hidden_ones_and_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for name in ["b", "A", "a", "c d", ".hidden", "repo/.git", "work", "Z"] {
            fs::create_dir_all(root.join(name)).unwrap();
        }
        // A worktree's `.git` is a file.
        fs::write(root.join("work/.git"), "gitdir: /x\n").unwrap();
        fs::write(root.join("file"), "").unwrap();
        symlink(root.join("b"), root.join("link")).unwrap();
        symlink(root.join("file"), root.join("file-link")).unwrap();
        let top = format!("{}/", root.display());
        let (path, linux, parent, dirs, error) = ask(&top, false, None, &WINDOWS);
        assert_eq!((path, linux, error), (top.clone(), Some(top), None));
        assert_eq!(
            dirs,
            [
                dir("A", false),
                dir("a", false),
                dir("b", false),
                dir("c d", false),
                dir("link", false),
                dir("repo", true),
                dir("work", true),
                dir("Z", false),
            ]
        );
        let above = root.parent().unwrap().display().to_string();
        assert_eq!(parent, Some(format!("{above}/")));

        // Without a trailing separator, the last name is a filter: its folder is listed.
        let typed = format!("{}/re", root.display());
        let (path, linux, parent, dirs, _) = ask(&typed, false, None, &WINDOWS);
        assert_eq!((path, linux), (typed.clone(), Some(typed)));
        assert_eq!(dirs.len(), 8);
        assert_eq!(parent, Some(format!("{above}/")));
    }

    #[test]
    fn at_most_the_cap_is_listed() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..=DIR_LIMIT {
            fs::create_dir(tmp.path().join(format!("d{i:04}"))).unwrap();
        }
        let top = format!("{}/", tmp.path().display());
        let (.., dirs, _) = ask(&top, false, None, &WINDOWS);
        assert_eq!(dirs.len(), DIR_LIMIT);
        assert_eq!(dirs[DIR_LIMIT - 1].name, format!("d{:04}", DIR_LIMIT - 1));
    }

    #[test]
    fn empty_is_the_home_folder_and_the_root_has_no_parent() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("p")).unwrap();
        let home = format!("{}/", tmp.path().display());
        let (path, linux, _, dirs, _) = ask("", false, Some(tmp.path()), &WINDOWS);
        assert_eq!((path, linux), (home.clone(), Some(home.clone())));
        assert_eq!(dirs, [dir("p", false)]);
        // A home ending with a separator gets no second one.
        let (path, ..) = ask("", false, Some(Path::new(&home)), &WINDOWS);
        assert_eq!(path, home);
        let (.., error) = ask("", false, None, &WINDOWS);
        assert_eq!(error.as_deref(), Some("HOME is not set"));

        let (path, _, parent, dirs, error) = ask("/", false, None, &WINDOWS);
        assert_eq!((path.as_str(), parent, error), ("/", None, None));
        assert!(dirs.iter().any(|d| d.name == "tmp"), "{dirs:?}");
        let (.., parent, _, _) = ask("/tm", false, None, &WINDOWS);
        assert_eq!(parent, None);
        let (.., parent, _, _) = ask("/tmp/", false, None, &WINDOWS);
        assert_eq!(parent.as_deref(), Some("/"));
    }

    #[test]
    fn bad_paths_are_explained() {
        let (path, linux, parent, dirs, error) = ask("rel/x", false, None, &WINDOWS);
        assert_eq!(
            (path.as_str(), linux, parent, dirs, error.as_deref()),
            ("rel/x", None, None, vec![], Some("type a full path"))
        );
        let (.., error) = ask("/nonexistent-hive/x/", false, None, &WINDOWS);
        let error = error.unwrap();
        assert!(
            error.starts_with("cannot open /nonexistent-hive/x/: "),
            "{error}"
        );
        let longest = format!("/{}/", "a".repeat(PATH_LIMIT - 2));
        let (.., error) = ask(&longest, false, None, &WINDOWS);
        assert!(error.unwrap().starts_with("cannot open /"));
        let (.., error) = ask(&format!("{longest}a"), false, None, &WINDOWS);
        assert_eq!(error.as_deref(), Some("paths are at most 4096 bytes"));
    }

    #[test]
    fn windows_paths_are_converted_and_home_is_the_user_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let c = tmp.path().join("c");
        fs::create_dir_all(c.join("Users/me/src/.git")).unwrap();
        fs::create_dir_all(c.join("Users/me/Documents")).unwrap();
        let bin = tempfile::tempdir().unwrap();
        // cmd.exe prints CRLF lines; the stand-in `wslpath -u` swaps `C:` for the folder `c`.
        let cmd = script(bin.path(), "cmd", r"printf 'C:\\Users\\me\r\n'");
        let wslpath = script(
            bin.path(),
            "wslpath",
            &format!(
                r#"[ "$1" = -u ] && printf '%s\n' "$2" | sed 's|^C:|{}|; s|\\|/|g'"#,
                c.display()
            ),
        );
        let programs = Windows {
            cmd: &cmd,
            wslpath: &wslpath,
        };
        let me = format!("{}/Users/me/", c.display());
        let (path, linux, parent, dirs, error) = ask("", true, None, &programs);
        assert_eq!(
            (path.as_str(), linux, parent.as_deref(), error),
            (r"C:\Users\me\", Some(me.clone()), Some(r"C:\Users\"), None)
        );
        assert_eq!(dirs, [dir("Documents", false), dir("src", true)]);
        // Either separator works, as on Windows.
        let (path, linux, parent, dirs, _) = ask(r"C:\Users/me\Doc", true, None, &programs);
        assert_eq!(path, r"C:\Users/me\Doc");
        assert_eq!(linux, Some(format!("{me}Doc")));
        assert_eq!(parent.as_deref(), Some(r"C:\Users/"));
        assert_eq!(dirs.len(), 2);
        let (.., parent, _, _) = ask(r"C:\", true, None, &programs);
        assert_eq!(parent, None);
        // What wslpath cannot make absolute is refused.
        let (.., error) = ask("x", true, None, &programs);
        assert_eq!(error.as_deref(), Some("type a full path"));
    }

    #[test]
    fn failing_windows_programs_are_explained() {
        let bin = tempfile::tempdir().unwrap();
        let fails = script(bin.path(), "fails", "echo nope >&2; exit 1");
        let silent = script(bin.path(), "silent", "exit 0");
        let missing = bin.path().join("missing").display().to_string();
        for (program, error) in [
            (&fails, format!("{fails} failed: nope")),
            (&silent, format!("{silent} failed: ")),
        ] {
            let programs = Windows {
                cmd: program,
                wslpath: program,
            };
            let (.., got) = ask("", true, None, &programs);
            assert_eq!(got, Some(error.clone()));
            let (.., got) = ask(r"C:\", true, None, &programs);
            assert_eq!(got, Some(error));
        }
        let programs = Windows {
            cmd: &missing,
            wslpath: &missing,
        };
        let (.., error) = ask("", true, None, &programs);
        assert!(
            error
                .unwrap()
                .starts_with(&format!("cannot run {missing}: ")),
        );
    }
}
