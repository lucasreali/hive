//! Projects: git repositories inside WSL that the app follows (#4). The service owns the list
//! and keeps it in `<data>/hive/projects.json`, a JSON array of top-level paths.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use hive_protocol::{Project, ProjectError, Worktree};

use crate::worktree::{self, WORKTREES_DIR};
use crate::wrapper::write_atomic;

/// Largest project list file read.
const FILE_LIMIT: u64 = 1024 * 1024;

pub struct Projects {
    file: PathBuf,
    paths: Mutex<Vec<String>>,
}

impl Projects {
    /// Loads the list from `file`. A missing file is an empty list; an unreadable or corrupt
    /// one is moved aside to `<file>.corrupt` with a warning, and the list starts empty.
    pub fn load(file: PathBuf) -> Self {
        let paths = match read(&file) {
            Ok(paths) => paths,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                let aside = file.with_extension("json.corrupt");
                eprintln!(
                    "hive: warning: ignoring {} ({err}); moved to {}",
                    file.display(),
                    aside.display()
                );
                let _ = std::fs::rename(&file, aside);
                Vec::new()
            }
        };
        Self {
            file,
            paths: Mutex::new(paths),
        }
    }

    /// Every project with its worktrees, in the order they were added.
    pub fn list(&self) -> Vec<Project> {
        let paths = self.paths().clone();
        paths.iter().map(|path| project(path)).collect()
    }

    /// Follows the git repository containing `path`. Adding one already followed changes
    /// nothing and answers the same project.
    pub fn add(&self, path: &str) -> Result<Project, (ProjectError, String)> {
        let id = validate(Path::new(path))?.to_string_lossy().into_owned();
        {
            let mut paths = self.paths();
            if !paths.contains(&id) {
                let mut next = paths.clone();
                next.push(id.clone());
                save(&self.file, &next).map_err(|err| {
                    let message = format!("cannot save {}: {err}", self.file.display());
                    (ProjectError::Storage, message)
                })?;
                *paths = next;
            }
        }
        Ok(project(&id))
    }

    fn paths(&self) -> MutexGuard<'_, Vec<String>> {
        self.paths.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn read(file: &Path) -> io::Result<Vec<String>> {
    let bytes = worktree::read_limited(&mut std::fs::File::open(file)?, FILE_LIMIT)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn save(file: &Path, paths: &[String]) -> io::Result<()> {
    file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
    let json = serde_json::to_vec_pretty(paths)?;
    write_atomic(file, &json, 0o600)
}

/// The main worktree of the repository containing `path`: absolute, an existing directory,
/// inside a git repository with a working tree.
fn validate(path: &Path) -> Result<PathBuf, (ProjectError, String)> {
    let shown = path.display();
    if !path.is_absolute() {
        let message = format!("{shown} is not an absolute path");
        return Err((ProjectError::NotAbsolute, message));
    }
    match std::fs::metadata(path) {
        Err(err) => Err((
            ProjectError::NotFound,
            format!("cannot open {shown}: {err}"),
        )),
        Ok(meta) if !meta.is_dir() => Err((
            ProjectError::NotADirectory,
            format!("{shown} is not a directory"),
        )),
        Ok(_) => worktree::main_root(path).map_err(|err| {
            let message = format!("{shown} is not in a git repository with a working tree: {err}");
            (ProjectError::NotAGitRepository, message)
        }),
    }
}

fn project(path: &str) -> Project {
    let root = Path::new(path);
    let (worktrees, error) = match worktree::list(root) {
        Ok(list) => (worktrees(root, list), None),
        Err(err) => (Vec::new(), Some(err.to_string())),
    };
    Project {
        id: path.to_owned(),
        name: file_name(root).unwrap_or_else(|| path.to_owned()),
        path: path.to_owned(),
        worktrees,
        error,
    }
}

/// Describes `git worktree list` for the sidebar, skipping bare entries.
fn worktrees(root: &Path, list: Vec<worktree::Worktree>) -> Vec<Worktree> {
    let claude_dir = root.join(WORKTREES_DIR);
    list.into_iter()
        .filter(|wt| !wt.bare)
        .enumerate()
        .map(|(i, wt)| {
            let path = wt.path.to_string_lossy().into_owned();
            let claude = wt.path.parent() == Some(claude_dir.as_path());
            let dir = file_name(&wt.path).unwrap_or_else(|| path.clone());
            let name = match &wt.branch {
                Some(branch) if !claude => branch.clone(),
                _ => dir,
            };
            Worktree {
                id: path.clone(),
                name,
                path,
                branch: wt.branch,
                main: i == 0,
                claude,
            }
        })
        .collect()
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn wt(path: &str, branch: Option<&str>, bare: bool) -> worktree::Worktree {
        worktree::Worktree {
            path: path.into(),
            branch: branch.map(Into::into),
            bare,
        }
    }

    #[test]
    fn worktrees_are_named_for_the_sidebar() {
        let list = vec![
            wt("/r", Some("main"), false),
            wt("/r/.claude/worktrees/fix-a", Some("worktree-fix-a"), false),
            wt("/r/.claude/worktrees/b", None, false),
            wt("/elsewhere/feat", Some("feat/x"), false),
            wt("/elsewhere/detached", None, false),
            wt("/r/.claude/worktrees/deep/c", Some("c"), false),
        ];
        let got: Vec<(String, bool, bool)> = worktrees(Path::new("/r"), list)
            .into_iter()
            .map(|w| (w.name, w.main, w.claude))
            .collect();
        let expected = [
            ("main", true, false),
            ("fix-a", false, true),
            ("b", false, true),
            ("feat/x", false, false),
            ("detached", false, false),
            ("c", false, false),
        ];
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(n, m, c)| (n.to_owned(), m, c))
            .collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn a_worktree_keeps_its_path_and_branch() {
        let got = worktrees(Path::new("/r"), vec![wt("/r", Some("main"), false)]);
        let main = Worktree {
            id: "/r".into(),
            name: "main".into(),
            path: "/r".into(),
            branch: Some("main".into()),
            main: true,
            claude: false,
        };
        assert_eq!(got, vec![main]);
    }

    #[test]
    fn bare_entries_are_skipped_and_the_root_has_no_file_name() {
        let got = worktrees(
            Path::new("/"),
            vec![wt("/b.git", None, true), wt("/", None, false)],
        );
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].name.as_str(), got[0].main), ("/", true));
    }

    #[test]
    fn invalid_folders_are_refused_before_git_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        std::fs::write(&file, "").unwrap();
        let cases = [
            (
                "relative/dir".to_owned(),
                ProjectError::NotAbsolute,
                "relative/dir is not an absolute path",
            ),
            (
                tmp.path().join("missing").display().to_string(),
                ProjectError::NotFound,
                "cannot open ",
            ),
            (
                file.display().to_string(),
                ProjectError::NotADirectory,
                " is not a directory",
            ),
        ];
        let projects = Projects::load(tmp.path().join("projects.json"));
        for (path, error, message) in cases {
            let (got, text) = projects.add(&path).unwrap_err();
            assert_eq!(got, error, "{path}");
            assert!(text.contains(message), "{text}");
        }
        assert!(projects.list().is_empty());
        assert!(!tmp.path().join("projects.json").exists());
    }

    #[test]
    fn a_missing_file_is_an_empty_list() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            Projects::load(tmp.path().join("projects.json"))
                .list()
                .is_empty()
        );
    }

    #[test]
    fn a_corrupt_file_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("projects.json");
        for bad in [&b"{not json"[..], &vec![b' '; FILE_LIMIT as usize + 1]] {
            std::fs::write(&file, bad).unwrap();
            assert!(Projects::load(file.clone()).paths().is_empty());
            assert!(!file.exists());
            assert_eq!(
                std::fs::read(tmp.path().join("projects.json.corrupt")).unwrap(),
                bad
            );
        }
    }

    #[test]
    fn saved_lists_are_private_and_load_back() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("data/hive/projects.json");
        // Longer than a few KiB, well within the read limit.
        let mut paths: Vec<String> = (0..500).map(|i| format!("/projects/{i:04}")).collect();
        paths.push("/b c".to_owned());
        save(&file, &paths).unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(*Projects::load(file).paths(), paths);
    }

    #[test]
    fn a_failed_save_is_a_storage_error() {
        let tmp = tempfile::tempdir().unwrap();
        // The data "directory" is a file, so nothing can be written under it.
        let blocker = tmp.path().join("data");
        std::fs::write(&blocker, "").unwrap();
        assert!(save(&blocker.join("projects.json"), &[]).is_err());
    }
}
