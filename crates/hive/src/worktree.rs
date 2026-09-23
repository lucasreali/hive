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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    /// Short branch name; `None` when detached or bare.
    pub branch: Option<String>,
    pub bare: bool,
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

/// Parses `git worktree list --porcelain -z`.
pub fn parse_porcelain(out: &[u8]) -> Vec<Worktree> {
    let mut list: Vec<Worktree> = Vec::new();
    for field in out.split(|&b| b == 0) {
        if let Some(path) = field.strip_prefix(b"worktree ") {
            list.push(Worktree {
                path: PathBuf::from(OsStr::from_bytes(path)),
                branch: None,
                bare: false,
            });
        } else if let Some(current) = list.last_mut() {
            if let Some(branch) = field.strip_prefix(b"branch ") {
                let branch = branch.strip_prefix(b"refs/heads/").unwrap_or(branch);
                current.branch = Some(String::from_utf8_lossy(branch).into_owned());
            } else if field == b"bare" {
                current.bare = true;
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
/// linked worktree) and returns its path.
pub fn create(dir: &Path, name: &str, base: Option<&str>) -> io::Result<PathBuf> {
    validate_name(name)?;
    if let Some(base) = base.filter(|base| base.starts_with('-')) {
        return Err(io::Error::other(format!("invalid base branch {base:?}")));
    }
    let root = main_root(dir)?;
    for file in own_create_hooks(&root) {
        eprintln!(
            "hive: warning: {file} defines its own WorktreeCreate hook; it will compete with Hive's"
        );
    }
    let path = root.join(WORKTREES_DIR).join(name);
    if path.symlink_metadata().is_ok() {
        return Err(io::Error::other(format!(
            "worktree {name:?} already exists at {}",
            path.display()
        )));
    }
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
    if let Err(err) = copy_included(&root, &path, &included) {
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
    Ok(path)
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
pub fn hook_create(input: &mut dyn Read) -> io::Result<PathBuf> {
    let payload = read_payload(input)?;
    let name = field(&payload, "name")?;
    let cwd = Path::new(field(&payload, "cwd")?);
    match existing(cwd, name)? {
        Some(path) => Ok(path),
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
pub fn hook_remove(input: &mut dyn Read) -> io::Result<()> {
    let payload = read_payload(input)?;
    let path = Path::new(field(&payload, "worktree_path")?).canonicalize()?;
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

fn read_payload(input: &mut dyn Read) -> io::Result<Value> {
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
/// regular files are copied, never over an existing path nor through a symlink.
fn copy_included(root: &Path, worktree: &Path, included: &[u8]) -> io::Result<()> {
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
    }
    Ok(())
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
                    bare: false
                },
                Worktree {
                    path: "/repo/.claude/worktrees/a b".into(),
                    branch: None,
                    bare: false
                },
                Worktree {
                    path: "/repo/.claude/worktrees/c".into(),
                    branch: Some("worktree-c".into()),
                    bare: false
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
    fn input_is_size_limited() {
        assert_eq!(read_limited(&mut &b"abcd"[..], 4).unwrap(), b"abcd");
        let err = read_limited(&mut &b"abcde"[..], 4).unwrap_err();
        assert_eq!(err.to_string(), "input larger than 4 bytes");
    }
}
