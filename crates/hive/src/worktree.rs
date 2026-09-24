//! Git worktrees following Claude Code's convention: `<repo>/.claude/worktrees/<name>`
//! on branch `worktree-<name>`. Git always runs as the `git` executable with separate arguments.

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// Largest hook payload accepted on stdin.
pub const HOOK_INPUT_LIMIT: u64 = 64 * 1024;
/// Largest project settings file inspected for a competing `WorktreeCreate` hook.
const SETTINGS_LIMIT: u64 = 1024 * 1024;
/// Where Claude Code (and Hive) put worktrees, relative to the main worktree.
pub const WORKTREES_DIR: &str = ".claude/worktrees";
const INCLUDE_FILE: &str = ".worktreeinclude";
/// Most bytes of branch names listed for the app; a repository with more is cut short.
const BRANCHES_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    /// Short branch name; `None` when detached or bare.
    pub branch: Option<String>,
    pub bare: bool,
    /// Its directory is gone; git keeps it until `git worktree prune`.
    pub prunable: bool,
}

impl fmt::Display for Worktree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let branch = match (&self.branch, self.bare) {
            (Some(branch), _) => branch,
            (None, true) => "(bare)",
            (None, false) => "(detached)",
        };
        write!(f, "{}\t{branch}", self.path.display())
    }
}

/// Same rule as the app's dialog: `^[a-z0-9][a-z0-9._-]*$`.
pub fn validate_name(name: &str) -> io::Result<()> {
    let allowed = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let valid = name.as_bytes().first().is_some_and(allowed)
        && name.bytes().all(|b| allowed(&b) || b"._-".contains(&b));
    if valid {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "invalid worktree name {name:?}: use lowercase letters, digits, '.', '_' and '-', starting with a letter or digit"
    )))
}

/// A new worktree and what the CLI reports about it on stderr.
#[derive(Debug, PartialEq, Eq)]
pub struct Created {
    pub path: PathBuf,
    /// e.g. a competing `WorktreeCreate` hook or the `.worktreeinclude` files copied.
    pub notes: Vec<String>,
}

/// The folder (relative to the main worktree) and branch the worktree `name` gets.
/// An empty name shows as `<name>`.
pub fn planned(name: &str) -> (String, String) {
    let name = if name.is_empty() { "<name>" } else { name };
    (
        format!("{WORKTREES_DIR}/{name}/"),
        format!("worktree-{name}"),
    )
}

/// Refuses an invalid name, or one whose folder already exists under `root` (the main
/// worktree). Returns the worktree's path. Git refuses an existing branch later.
pub fn check_name(root: &Path, name: &str) -> io::Result<PathBuf> {
    validate_name(name)?;
    let path = root.join(WORKTREES_DIR).join(name);
    if path.symlink_metadata().is_ok() {
        return Err(io::Error::other(format!(
            "worktree {name:?} already exists at {}",
            path.display()
        )));
    }
    Ok(path)
}

/// Local and remote branches, and the branch checked out where git runs.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Branches {
    pub local: Vec<String>,
    pub remote: Vec<String>,
    pub current: Option<String>,
}

/// Parses `git for-each-ref --format=%(HEAD)%(refname) refs/heads refs/remotes`: one ref
/// per line, prefixed by `*` for the checked-out branch. Remote `HEAD` symrefs are skipped
/// and at most [`BRANCHES_LIMIT`] bytes of names are kept.
pub fn parse_branches(out: &[u8]) -> Branches {
    let mut branches = Branches::default();
    let mut budget = BRANCHES_LIMIT;
    for line in String::from_utf8_lossy(out).lines() {
        let (current, refname) = match line.strip_prefix('*') {
            Some(refname) => (true, refname),
            None => (false, line.trim_start()),
        };
        let (list, name) = if let Some(name) = refname.strip_prefix("refs/heads/") {
            (&mut branches.local, name)
        } else if let Some(name) = refname.strip_prefix("refs/remotes/")
            && !name.ends_with("/HEAD")
        {
            (&mut branches.remote, name)
        } else {
            continue;
        };
        let Some(left) = budget.checked_sub(name.len()) else {
            break;
        };
        budget = left;
        if current {
            branches.current = Some(name.to_owned());
        }
        list.push(name.to_owned());
    }
    branches
}

/// The branches of the repository at `root`; `current` is the one checked out there.
pub fn branches(root: &Path) -> io::Result<Branches> {
    let args = [
        "for-each-ref",
        "--format=%(HEAD)%(refname)",
        "refs/heads",
        "refs/remotes",
    ];
    git(root, &args).map(|out| parse_branches(&out))
}

/// Parses `git worktree list --porcelain -z`.
pub fn parse_porcelain(out: &[u8]) -> Vec<Worktree> {
    let mut list: Vec<Worktree> = Vec::new();
    for field in out.split(|&b| b == 0) {
        if let Some(path) = field.strip_prefix(b"worktree ") {
            list.push(Worktree {
                path: PathBuf::from(OsStr::from_bytes(path)),
                branch: None,
                bare: false,
                prunable: false,
            });
        } else if let Some(current) = list.last_mut() {
            if let Some(branch) = field.strip_prefix(b"branch ") {
                let branch = branch.strip_prefix(b"refs/heads/").unwrap_or(branch);
                current.branch = Some(String::from_utf8_lossy(branch).into_owned());
            } else if field == b"bare" {
                current.bare = true;
            } else if field.starts_with(b"prunable") {
                current.prunable = true;
            }
        }
    }
    list
}

/// Every worktree of the repository containing `dir`, the main one first.
pub fn list(dir: &Path) -> io::Result<Vec<Worktree>> {
    git(dir, &["worktree", "list", "--porcelain", "-z"]).map(|out| parse_porcelain(&out))
}

/// Creates the worktree `name` for the repository containing `dir` (even from inside a
/// linked worktree). The CLI and the app's dialog both come here (#33).
pub fn create(dir: &Path, name: &str, base: Option<&str>) -> io::Result<Created> {
    validate_name(name)?;
    if let Some(base) = base.filter(|base| base.starts_with('-')) {
        return Err(io::Error::other(format!("invalid base branch {base:?}")));
    }
    let root = main_root(dir)?;
    let path = check_name(&root, name)?;
    let mut notes: Vec<String> = own_create_hooks(&root)
        .into_iter()
        .map(|file| {
            format!(
                "warning: {file} defines its own WorktreeCreate hook; it will compete with Hive's"
            )
        })
        .collect();
    let included = included_files(&root)?;
    let branch = format!("worktree-{name}");
    // `-b` refuses an existing branch.
    let mut args = vec![
        OsStr::new("worktree"),
        OsStr::new("add"),
        OsStr::new("-b"),
        OsStr::new(&branch),
        path.as_os_str(),
    ];
    args.extend(base.map(OsStr::new));
    run_git(&root, &args, &[], &[0])?;
    match copy_included(&root, &path, &included) {
        Ok(0) => {}
        Ok(copied) => notes.push(copied_note(copied)),
        Err(err) => {
            // Undo the brand-new worktree and branch: a failed create leaves nothing behind.
            let remove = [
                OsStr::new("worktree"),
                OsStr::new("remove"),
                OsStr::new("--force"),
                path.as_os_str(),
            ];
            let _ = run_git(&root, &remove, &[], &[0]);
            let _ = git(&root, &["branch", "-D", &branch]);
            return Err(err);
        }
    }
    Ok(Created { path, notes })
}

fn copied_note(copied: usize) -> String {
    let files = if copied == 1 { "file" } else { "files" };
    format!("copied {copied} {files} listed in {INCLUDE_FILE}")
}

/// Removes the worktree `name` with `git worktree remove` (refused if it has changes).
/// Its branch is kept.
pub fn remove(dir: &Path, name: &str) -> io::Result<()> {
    validate_name(name)?;
    let root = main_root(dir)?;
    remove_path(&root, &root.join(WORKTREES_DIR).join(name))
}

/// `WorktreeCreate` hook: creates the worktree `name` in the repository of `cwd`, or
/// reuses it when it is already a Hive worktree (`claude -w <existing>` reopens it, as
/// Claude Code does without the hook).
pub fn hook_create(payload: &Value) -> io::Result<Created> {
    let name = field(payload, "name")?;
    let cwd = Path::new(field(payload, "cwd")?);
    match existing(cwd, name)? {
        Some(path) => Ok(Created {
            path,
            notes: Vec::new(),
        }),
        None => create(cwd, name, None),
    }
}

/// The worktree `name` if it exists at `.claude/worktrees/<name>` on branch `worktree-<name>`.
fn existing(dir: &Path, name: &str) -> io::Result<Option<PathBuf>> {
    validate_name(name)?;
    let path = main_root(dir)?.join(WORKTREES_DIR).join(name);
    let branch = format!("worktree-{name}");
    let ours = |wt: &Worktree| wt.path == path && wt.branch.as_deref() == Some(branch.as_str());
    Ok(list(dir)?.into_iter().find(ours).map(|wt| wt.path))
}

/// `WorktreeRemove` hook: removes `worktree_path`, which must be a worktree directly under
/// its repository's `.claude/worktrees/`.
pub fn hook_remove(payload: &Value) -> io::Result<()> {
    let path = Path::new(field(payload, "worktree_path")?).canonicalize()?;
    let root = main_root(&path)?;
    if path.parent() != Some(root.join(WORKTREES_DIR).canonicalize()?.as_path()) {
        return Err(io::Error::other(format!(
            "refusing to remove {}: not under {}",
            path.display(),
            root.join(WORKTREES_DIR).display()
        )));
    }
    remove_path(&root, &path)
}

/// `git worktree remove` without `--force`: a worktree with changes is kept.
fn remove_path(root: &Path, path: &Path) -> io::Result<()> {
    let args = [
        OsStr::new("worktree"),
        OsStr::new("remove"),
        path.as_os_str(),
    ];
    run_git(root, &args, &[], &[0]).map(drop)
}

/// Reads at most `limit` bytes; more is an error.
pub fn read_limited(input: &mut dyn Read, limit: u64) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    input.take(limit + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > limit {
        return Err(io::Error::other(format!("input larger than {limit} bytes")));
    }
    Ok(buf)
}

/// A worktree hook's JSON input, at most [`HOOK_INPUT_LIMIT`] bytes.
pub fn read_payload(input: &mut dyn Read) -> io::Result<Value> {
    serde_json::from_slice(&read_limited(input, HOOK_INPUT_LIMIT)?)
        .map_err(|err| io::Error::other(format!("invalid hook input: {err}")))
}

fn field<'a>(payload: &'a Value, key: &str) -> io::Result<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| io::Error::other(format!("hook input has no string field {key:?}")))
}

/// Whether a Claude Code settings file configures a `WorktreeCreate` hook.
pub fn has_create_hook(settings: &[u8]) -> bool {
    serde_json::from_slice::<Value>(settings)
        .ok()
        .and_then(|value| value.pointer("/hooks/WorktreeCreate")?.as_array().cloned())
        .is_some_and(|hooks| !hooks.is_empty())
}

/// Project settings files under `root` that configure their own `WorktreeCreate` hook.
fn own_create_hooks(root: &Path) -> Vec<&'static str> {
    [".claude/settings.json", ".claude/settings.local.json"]
        .into_iter()
        .filter(|file| {
            std::fs::File::open(root.join(file))
                .and_then(|mut f| read_limited(&mut f, SETTINGS_LIMIT))
                .is_ok_and(|settings| has_create_hook(&settings))
        })
        .collect()
}

/// The main worktree's root, from anywhere inside the repository or one of its worktrees.
pub fn main_root(dir: &Path) -> io::Result<PathBuf> {
    list(dir)?
        .into_iter()
        .next()
        .filter(|main| !main.bare)
        .map(|main| main.path)
        .ok_or_else(|| io::Error::other("a bare repository has no main worktree"))
}

/// Untracked files matched by `.worktreeinclude` (gitignore syntax) that are also
/// gitignored, as Claude Code copies them: NUL-separated paths relative to `root`.
/// Git evaluates both pattern sets.
fn included_files(root: &Path) -> io::Result<Vec<u8>> {
    if !root.join(INCLUDE_FILE).is_file() {
        return Ok(Vec::new());
    }
    let exclude_from = format!("--exclude-from={INCLUDE_FILE}");
    let args = ["ls-files", "-z", "--others", "--ignored", &exclude_from];
    let matching = git(root, &args)?;
    // Exit code 1 means none of them is ignored.
    let args = ["check-ignore", "-z", "--stdin"].map(OsStr::new);
    run_git(root, &args, &matching, &[0, 1])
}

/// Copies `included` (from [`included_files`]) from `root` into the new worktree. Only
/// regular files are copied, never over an existing path nor through a symlink. Returns how
/// many were copied.
fn copy_included(root: &Path, worktree: &Path, included: &[u8]) -> io::Result<usize> {
    let mut copied = 0;
    for rel in included
        .split(|&b| b == 0)
        .map(|p| Path::new(OsStr::from_bytes(p)))
    {
        let src = root.join(rel);
        let is_file = src.symlink_metadata().is_ok_and(|m| m.is_file());
        if !is_file || unsafe_target(worktree, rel) {
            continue;
        }
        let dst = worktree.join(rel);
        std::fs::create_dir_all(dst.parent().unwrap_or(worktree))?;
        std::fs::copy(src, dst)?;
        copied += 1;
    }
    Ok(copied)
}

/// Whether writing `rel` under `base` would overwrite something (e.g. a tracked file or
/// symlink of the base branch) or pass through a symlink, possibly out of the worktree.
fn unsafe_target(base: &Path, rel: &Path) -> bool {
    base.join(rel).symlink_metadata().is_ok()
        || rel.ancestors().skip(1).any(|dir| {
            base.join(dir)
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
        })
}

fn git(dir: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    run_git(dir, &args, &[], &[0])
}

/// Runs `git -C <dir> <args>` with `input` on stdin; any exit code outside `ok` is an error
/// carrying git's stderr.
fn run_git(dir: &Path, args: &[&OsStr], input: &[u8], ok: &[i32]) -> io::Result<Vec<u8>> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        // Hive always names the repository with `-C`.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| io::Error::new(err.kind(), format!("cannot run git: {err}")))?;
    let stdin = child.stdin.take();
    // Feed stdin from another thread so a large input cannot deadlock against stdout.
    let out = std::thread::scope(|scope| {
        scope.spawn(move || stdin.map(|mut stdin| stdin.write_all(input)));
        child.wait_with_output()
    });
    let out = out?;
    if out.status.code().is_some_and(|code| ok.contains(&code)) {
        return Ok(out.stdout);
    }
    let command: Vec<_> = args.iter().map(|arg| arg.to_string_lossy()).collect();
    Err(io::Error::other(format!(
        "git {} failed: {}",
        command.join(" "),
        String::from_utf8_lossy(&out.stderr).trim()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_dialog_rule() {
        for name in ["a", "0", "feat-1", "a.b_c-d", "9lives"] {
            assert!(validate_name(name).is_ok(), "{name}");
        }
        for name in ["", "A", "-a", ".a", "_a", "a/b", "aB", "a b", "a..\n", "é"] {
            let err = validate_name(name).unwrap_err();
            assert!(err.to_string().contains("invalid worktree name"), "{name}");
        }
    }

    #[test]
    fn porcelain_is_parsed() {
        let out = b"worktree /repo\0HEAD 1111\0branch refs/heads/main\0\0\
worktree /repo/.claude/worktrees/a b\0HEAD 2222\0detached\0locked\0\0\
worktree /repo/.claude/worktrees/c\0HEAD 3333\0branch refs/heads/worktree-c\0prunable gone\0\0";
        let list = parse_porcelain(out);
        assert_eq!(
            list,
            vec![
                Worktree {
                    path: "/repo".into(),
                    branch: Some("main".into()),
                    bare: false,
                    prunable: false
                },
                Worktree {
                    path: "/repo/.claude/worktrees/a b".into(),
                    branch: None,
                    bare: false,
                    prunable: false
                },
                Worktree {
                    path: "/repo/.claude/worktrees/c".into(),
                    branch: Some("worktree-c".into()),
                    bare: false,
                    prunable: true
                },
            ]
        );
        assert_eq!(list[0].to_string(), "/repo\tmain");
        assert_eq!(
            list[1].to_string(),
            "/repo/.claude/worktrees/a b\t(detached)"
        );
    }

    #[test]
    fn bare_porcelain_and_stray_fields() {
        let list = parse_porcelain(b"branch refs/heads/x\0worktree /r.git\0bare\0\0");
        let bare = Worktree {
            path: "/r.git".into(),
            branch: None,
            bare: true,
            prunable: false,
        };
        assert_eq!(list, vec![bare.clone()]);
        assert_eq!(bare.to_string(), "/r.git\t(bare)");
        assert_eq!(
            parse_porcelain(b"worktree /r\0branch other/x\0")[0].branch,
            Some("other/x".into())
        );
        assert_eq!(parse_porcelain(b""), vec![]);
    }

    #[test]
    fn create_hook_is_detected_in_settings() {
        assert!(has_create_hook(
            br#"{"hooks":{"WorktreeCreate":[{"hooks":[{"type":"command","command":"x"}]}]}}"#
        ));
        assert!(!has_create_hook(br#"{"hooks":{"WorktreeCreate":[]}}"#));
        assert!(!has_create_hook(br#"{"hooks":{"WorktreeRemove":[{}]}}"#));
        assert!(!has_create_hook(br#"{"hooks":{"WorktreeCreate":"x"}}"#));
        assert!(!has_create_hook(b"not json"));
    }

    #[test]
    fn branches_are_parsed() {
        let out = b"*refs/heads/main\n refs/heads/feat/x\n refs/remotes/origin/HEAD\n\
 refs/remotes/origin/main\n refs/tags/v1\n";
        let branches = parse_branches(out);
        assert_eq!(
            branches,
            Branches {
                local: vec!["main".into(), "feat/x".into()],
                remote: vec!["origin/main".into()],
                current: Some("main".into()),
            }
        );
        assert_eq!(parse_branches(b""), Branches::default());
    }

    #[test]
    fn branch_names_are_size_limited() {
        let name = "b".repeat(1000);
        let out: String = (0..BRANCHES_LIMIT / 1000 + 5)
            .map(|_| format!(" refs/heads/{name}\n"))
            .collect();
        let branches = parse_branches(out.as_bytes());
        assert_eq!(branches.local.len(), BRANCHES_LIMIT / 1000);
        // A big repository (5000 remote branches) is listed whole.
        let out: String = (0..5000)
            .map(|i| format!(" refs/remotes/origin/renovate/dependency-{i:05}\n"))
            .collect();
        assert_eq!(parse_branches(out.as_bytes()).remote.len(), 5000);
        // Exactly at the limit is kept.
        let out = format!(" refs/heads/{}\n", "b".repeat(BRANCHES_LIMIT));
        assert_eq!(parse_branches(out.as_bytes()).local.len(), 1);
    }

    #[test]
    fn planned_folder_and_branch() {
        assert_eq!(
            planned("fix-a"),
            (".claude/worktrees/fix-a/".into(), "worktree-fix-a".into())
        );
        assert_eq!(
            planned(""),
            (".claude/worktrees/<name>/".into(), "worktree-<name>".into())
        );
    }

    #[test]
    fn existing_folders_are_refused_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude/worktrees/taken")).unwrap();
        let err = check_name(tmp.path(), "taken").unwrap_err().to_string();
        assert!(
            err.starts_with("worktree \"taken\" already exists at "),
            "{err}"
        );
        assert_eq!(
            check_name(tmp.path(), "free").unwrap(),
            tmp.path().join(".claude/worktrees/free")
        );
        assert!(check_name(tmp.path(), "Bad").is_err());
    }

    #[test]
    fn copied_files_are_counted() {
        assert_eq!(copied_note(1), "copied 1 file listed in .worktreeinclude");
        assert_eq!(copied_note(3), "copied 3 files listed in .worktreeinclude");
    }

    #[test]
    fn input_is_size_limited() {
        assert_eq!(read_limited(&mut &b"abcd"[..], 4).unwrap(), b"abcd");
        let err = read_limited(&mut &b"abcde"[..], 4).unwrap_err();
        assert_eq!(err.to_string(), "input larger than 4 bytes");
    }
}
