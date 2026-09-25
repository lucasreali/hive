//! Projects: git repositories inside WSL that the app follows (#4), grouped in spaces
//! (`hive::spaces`). The service owns the list and keeps it in `<data>/hive/spaces.json`; the
//! flat `projects.json` of earlier versions (a JSON array of top-level paths) becomes the
//! "Default" space until the first change is saved.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use hive_protocol::{Control, Project, ProjectError, Worktree};
use serde::de::DeserializeOwned;

use crate::spaces::{self, Spaces};
use crate::worktree::{self, WORKTREES_DIR};
use crate::wrapper::write_atomic;
use crate::{git, procs};

/// Largest spaces or project list file read.
const FILE_LIMIT: u64 = 1024 * 1024;
/// Most processes named when a worktree is in use.
const BUSY_SHOWN: usize = 5;

pub struct Projects {
    file: PathBuf,
    spaces: Mutex<Spaces>,
}

impl Projects {
    /// Loads the spaces from `file`; without it, the projects listed in `legacy` (the file of
    /// earlier versions) make the "Default" space. A missing file is an empty list; an
    /// unreadable or invalid one is moved aside to `<file>.corrupt` with a warning, and the
    /// list starts empty.
    pub fn load(file: PathBuf, legacy: &Path) -> Self {
        let loaded = read(&file).and_then(|s: Spaces| s.check().map_err(io::Error::other));
        let spaces = match loaded {
            Ok(spaces) => spaces,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let mut paths = read(legacy).unwrap_or_else(|err| set_aside(legacy, err));
                // Earlier versions allowed a hand-written path twice; a space never does.
                let mut seen = std::collections::HashSet::new();
                paths.retain(|p: &String| seen.insert(p.clone()));
                Spaces::with(paths)
            }
            Err(err) => Spaces::with(set_aside(&file, err)),
        };
        Self {
            file,
            spaces: Mutex::new(spaces),
        }
    }

    /// Every project with its worktrees, in the order they were added.
    pub fn list(&self) -> Vec<Project> {
        let paths: Vec<String> = self.spaces().projects().cloned().collect();
        paths.iter().map(|path| project(path)).collect()
    }

    /// The spaces and the current one, for the app.
    pub fn spaces_message(&self) -> Control {
        let spaces = self.spaces();
        Control::Spaces {
            spaces: spaces.spaces.clone(),
            current: spaces.current.clone(),
        }
    }

    /// The current space's projects and Claude config folder (where its sessions are).
    pub fn current(&self) -> (Vec<Project>, Option<String>) {
        let (paths, env) = self.spaces().current();
        let projects = paths.iter().map(|path| project(path)).collect();
        (projects, env.claude_config_dir)
    }

    /// What a terminal opened in `cwd` gets from the space of the project holding it: its
    /// environment entries and Claude config folder. Nothing outside every project.
    pub fn terminal_env(&self, cwd: &str) -> (Vec<(&'static str, String)>, Option<String>) {
        // Only the projects holding `cwd` when some do, so git runs for them alone; else
        // every project (a linked worktree may be anywhere).
        // ponytail: a worktree of one project inside another's folder takes the outer one's space.
        let ids: Vec<String> = self.spaces().projects().cloned().collect();
        let inside: Vec<Project> = (ids.iter())
            .filter(|id| Path::new(cwd).starts_with(id))
            .map(|id| project(id))
            .collect();
        let listed = if inside.is_empty() {
            self.list()
        } else {
            inside
        };
        let place = place(&listed, cwd);
        let spaces = self.spaces();
        let space = place.and_then(|(project, _)| spaces.of(&project).cloned());
        let env = space.map(|s| s.env).unwrap_or_default();
        (spaces::vars(&env), env.claude_config_dir)
    }

    /// Applies a space request and saves the result (when it changed anything); nothing
    /// changes when either fails.
    pub fn change_spaces(
        &self,
        change: impl FnOnce(&mut Spaces) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut spaces = self.spaces();
        let mut next = spaces.clone();
        change(&mut next)?;
        if next == *spaces {
            return Ok(());
        }
        save(&self.file, &next)
            .map_err(|err| format!("cannot save {}: {err}", self.file.display()))?;
        *spaces = next;
        Ok(())
    }

    /// Follows the git repository containing `path` in the current space. Adding one
    /// already there changes nothing and answers the same project; one in another space is
    /// refused (a project is in one space only).
    pub fn add(&self, path: &str) -> Result<Project, (ProjectError, String)> {
        let id = validate(path)?.to_string_lossy().into_owned();
        // Checked and added under one lock, so two adds at once cannot both add it.
        let mut other = false;
        let added = self.change_spaces(|spaces| {
            spaces.add(id.clone()).map_err(|name| {
                other = true;
                format!("{id} is already in the space {name}")
            })
        });
        let error = if other {
            ProjectError::InOtherSpace
        } else {
            ProjectError::Storage
        };
        added.map_err(|message| (error, message))?;
        Ok(project(&id))
    }

    /// The branches of the followed project `id`.
    pub fn branches(&self, id: &str) -> io::Result<worktree::Branches> {
        worktree::branches(&self.root(id)?)
    }

    /// Checks a new worktree name for the followed project `id`, as `create` would.
    pub fn validate_worktree_name(&self, id: &str, name: &str) -> io::Result<()> {
        worktree::check_name(&self.root(id)?, name).map(drop)
    }

    /// `hive worktree create` in the followed project `id`; answers the project with its
    /// updated worktrees.
    pub fn create_worktree(
        &self,
        id: &str,
        name: &str,
        base: Option<&str>,
    ) -> io::Result<(Project, worktree::Created)> {
        let created = worktree::create(&self.root(id)?, name, base)?;
        Ok((project(id), created))
    }

    /// Removes the linked worktree `path` of a followed project, with `--force` when `force`;
    /// answers the project with its updated worktrees. Without `force`, a worktree that a
    /// process (as `proc` lists them) works in is kept, as git keeps one with changes.
    pub fn remove_worktree(
        &self,
        path: &str,
        force: bool,
        proc: procs::Source,
    ) -> io::Result<Project> {
        let (owner, _) = self.linked(path)?;
        if !force {
            unused(proc, path)?;
        }
        worktree::remove_path(Path::new(&owner.id), Path::new(path), force)?;
        Ok(project(&owner.id))
    }

    /// Renames the Claude worktree `path` of a followed project to `name`, never while a
    /// process works in it (its folder moves). Answers the updated project and the new path.
    pub fn rename_worktree(
        &self,
        path: &str,
        name: &str,
        proc: procs::Source,
    ) -> io::Result<(Project, String)> {
        let (owner, wt) = self.linked(path)?;
        if !wt.claude {
            return Err(io::Error::other(format!(
                "only worktrees under {WORKTREES_DIR} can be renamed"
            )));
        }
        unused(proc, path)?;
        let to = worktree::rename(Path::new(&owner.id), Path::new(path), name)?;
        Ok((project(&owner.id), to.to_string_lossy().into_owned()))
    }

    /// The followed project holding the worktree `path`, when it is not the main one.
    fn linked(&self, path: &str) -> io::Result<(Project, Worktree)> {
        let found = self.list().into_iter().find_map(|p| {
            let wt = p.worktrees.iter().find(|w| w.path == path).cloned();
            wt.map(|wt| (p, wt))
        });
        match found {
            Some((_, wt)) if wt.main => Err(io::Error::other(format!(
                "{path} is the project's main worktree"
            ))),
            Some(found) => Ok(found),
            None => Err(io::Error::other(format!(
                "{path} is not a worktree of a followed project"
            ))),
        }
    }

    /// `path` when it is a worktree of a followed project: the path comes from the app.
    pub fn worktree(&self, path: &str) -> io::Result<PathBuf> {
        let followed = self.list().into_iter().flat_map(|p| p.worktrees);
        if followed.into_iter().any(|w| w.path == path) {
            return Ok(PathBuf::from(path));
        }
        Err(io::Error::other(format!(
            "{path} is not a worktree of a followed project"
        )))
    }

    /// Only followed projects are acted on: the id comes from the app.
    fn root(&self, id: &str) -> io::Result<PathBuf> {
        if self.spaces().projects().any(|path| path == id) {
            return Ok(PathBuf::from(id));
        }
        Err(io::Error::other(format!("{id} is not a followed project")))
    }

    fn spaces(&self) -> MutexGuard<'_, Spaces> {
        self.spaces.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The followed worktree containing `cwd`, as `(project id, worktree id)`. Claude worktrees
/// live inside the main one, so the deepest match wins (#19).
pub fn place(projects: &[Project], cwd: &str) -> Option<(String, String)> {
    let cwd = Path::new(cwd);
    projects
        .iter()
        .flat_map(|p| p.worktrees.iter().map(move |w| (p, w)))
        .filter(|(_, w)| cwd.starts_with(&w.path))
        .max_by_key(|(_, w)| w.path.len())
        .map(|(p, w)| (p.id.clone(), w.id.clone()))
}

/// Refuses a worktree that some process (e.g. a terminal or an agent) works in.
fn unused(proc: procs::Source, path: &str) -> io::Result<()> {
    let mut busy = procs::inside(proc, Path::new(path));
    if busy.is_empty() {
        return Ok(());
    }
    busy.sort_by_key(|p| p.pid);
    let mut names: Vec<String> = busy
        .iter()
        .take(BUSY_SHOWN)
        .map(|p| format!("{} ({})", p.comm, p.pid))
        .collect();
    if busy.len() > BUSY_SHOWN {
        names.push(format!("{} more", busy.len() - BUSY_SHOWN));
    }
    Err(io::Error::other(format!(
        "in use by {}: close its terminals first",
        names.join(", ")
    )))
}

fn read<T: DeserializeOwned>(file: &Path) -> io::Result<T> {
    let bytes = git::read_limited(&mut std::fs::File::open(file)?, FILE_LIMIT)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

/// Moves an unreadable or invalid list aside with a warning (a missing one is only empty):
/// the service starts without it.
fn set_aside(file: &Path, err: io::Error) -> Vec<String> {
    if err.kind() != io::ErrorKind::NotFound {
        let aside = file.with_extension("json.corrupt");
        eprintln!(
            "hive: warning: ignoring {} ({err}); moved to {}",
            file.display(),
            aside.display()
        );
        let _ = std::fs::rename(file, aside);
    }
    Vec::new()
}

fn save(file: &Path, spaces: &Spaces) -> io::Result<()> {
    file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
    let json = serde_json::to_vec_pretty(spaces)?;
    // Never a file the next start would refuse to read.
    if json.len() as u64 > FILE_LIMIT {
        return Err(io::Error::other(format!(
            "the list would be over {FILE_LIMIT} bytes"
        )));
    }
    write_atomic(file, &json, 0o600)
}

/// The main worktree of the repository containing `path`: not blank, absolute, an existing
/// directory, inside a git repository with a working tree.
fn validate(path: &str) -> Result<PathBuf, (ProjectError, String)> {
    if path.trim().is_empty() {
        return Err((ProjectError::EmptyPath, "Enter a folder".to_owned()));
    }
    let path = Path::new(path);
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
        name: file_name(root).unwrap_or(path.to_owned()),
        path: path.to_owned(),
        worktrees,
        error,
    }
}

/// Describes `git worktree list` for the sidebar, skipping bare entries and worktrees whose
/// directory is gone (git lists them as prunable until `git worktree prune`).
fn worktrees(root: &Path, list: Vec<worktree::Worktree>) -> Vec<Worktree> {
    let claude_dir = root.join(WORKTREES_DIR);
    list.into_iter()
        .filter(|wt| !wt.bare && !wt.prunable)
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
                status: None,
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
            prunable: false,
        }
    }

    #[test]
    fn a_worktree_in_use_names_its_processes() {
        let busy = |count: i32| {
            let proc = tempfile::tempdir().unwrap();
            let dir = proc.path().join("wt");
            for pid in 1..=count {
                let entry = proc.path().join(pid.to_string());
                std::fs::create_dir(&entry).unwrap();
                let stat = format!("{pid} (p{pid}) S 1 {pid} {pid} 0");
                std::fs::write(entry.join("stat"), stat).unwrap();
                std::os::unix::fs::symlink(&dir, entry.join("cwd")).unwrap();
            }
            let err = unused(procs::Source::Dir(proc.path()), dir.to_str().unwrap()).unwrap_err();
            assert!(unused(procs::Source::Dir(proc.path()), "/elsewhere").is_ok());
            err.to_string()
        };
        assert_eq!(
            busy(7),
            "in use by p1 (1), p2 (2), p3 (3), p4 (4), p5 (5), 2 more: close its terminals first"
        );
        assert_eq!(
            busy(5),
            "in use by p1 (1), p2 (2), p3 (3), p4 (4), p5 (5): close its terminals first"
        );
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
            status: None,
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
    fn worktrees_whose_directory_is_gone_are_skipped() {
        let gone = worktree::Worktree {
            prunable: true,
            ..wt("/r/.claude/worktrees/gone", None, false)
        };
        let list = vec![wt("/r", None, false), gone, wt("/r/x", None, false)];
        let got: Vec<String> = worktrees(Path::new("/r"), list)
            .into_iter()
            .map(|w| w.id)
            .collect();
        assert_eq!(got, ["/r", "/r/x"]);
    }

    #[test]
    fn an_agent_is_placed_in_the_deepest_worktree_containing_its_cwd() {
        let project = |root: &str, list| Project {
            id: root.into(),
            name: String::new(),
            path: root.into(),
            worktrees: worktrees(Path::new(root), list),
            error: None,
        };
        let projects = [
            project(
                "/r",
                vec![
                    wt("/r", Some("main"), false),
                    wt("/r/.claude/worktrees/a", None, false),
                ],
            ),
            project("/r2", vec![wt("/r2", None, false)]),
        ];
        let place = |cwd| place(&projects, cwd);
        let at = |p: &str, w: &str| Some((p.to_owned(), w.to_owned()));
        assert_eq!(place("/r"), at("/r", "/r"));
        assert_eq!(place("/r/src/x"), at("/r", "/r"));
        assert_eq!(
            place("/r/.claude/worktrees/a"),
            at("/r", "/r/.claude/worktrees/a")
        );
        assert_eq!(
            place("/r/.claude/worktrees/a/src"),
            at("/r", "/r/.claude/worktrees/a")
        );
        assert_eq!(place("/r/.claude/worktrees/ab"), at("/r", "/r"));
        // Whole path components only: /r2 is not inside /r.
        assert_eq!(place("/r2/y"), at("/r2", "/r2"));
        assert_eq!(place("/elsewhere"), None);
        assert_eq!(place(""), None);
    }

    #[test]
    fn invalid_folders_are_refused_before_git_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        std::fs::write(&file, "").unwrap();
        let cases = [
            ("".to_owned(), ProjectError::EmptyPath, "Enter a folder"),
            (" \t ".to_owned(), ProjectError::EmptyPath, "Enter a folder"),
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
        let projects = load(tmp.path());
        for (path, error, message) in cases {
            let (got, text) = projects.add(&path).unwrap_err();
            assert_eq!(got, error, "{path}");
            assert!(text.contains(message), "{text}");
        }
        assert!(projects.list().is_empty());
        assert!(!tmp.path().join("spaces.json").exists());
    }

    /// Projects kept in `dir`: `spaces.json`, else the older `projects.json`.
    fn load(dir: &Path) -> Projects {
        Projects::load(dir.join("spaces.json"), &dir.join("projects.json"))
    }

    #[test]
    fn worktree_requests_need_a_followed_project() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = load(tmp.path());
        let id = tmp.path().display().to_string();
        let refused = format!("{id} is not a followed project");
        assert_eq!(projects.branches(&id).unwrap_err().to_string(), refused);
        let err = projects.validate_worktree_name(&id, "x").unwrap_err();
        assert_eq!(err.to_string(), refused);
        let err = projects.create_worktree(&id, "x", None).unwrap_err();
        assert_eq!(err.to_string(), refused);
        assert!(!tmp.path().join(WORKTREES_DIR).exists());
        // A followed one gets the same checks as the CLI, without git for the name.
        projects.spaces().add(id.clone()).unwrap();
        assert!(projects.validate_worktree_name(&id, "free").is_ok());
        let err = projects.validate_worktree_name(&id, "Bad").unwrap_err();
        assert!(err.to_string().starts_with("invalid worktree name"));
    }

    #[test]
    fn missing_files_are_an_empty_default_space() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = load(tmp.path());
        assert!(projects.list().is_empty());
        assert_eq!(*projects.spaces(), Spaces::with(vec![]));
        assert_eq!(projects.current(), (vec![], None));
        assert_eq!(
            projects.spaces_message(),
            Control::Spaces {
                spaces: Spaces::with(vec![]).spaces,
                current: "default".into(),
            }
        );
    }

    #[test]
    fn the_older_project_list_becomes_the_default_space() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("projects.json");
        std::fs::write(&legacy, r#"["/a", "/b c", "/a"]"#).unwrap();
        let projects = load(tmp.path());
        let migrated = Spaces::with(vec!["/a".into(), "/b c".into()]);
        assert_eq!(*projects.spaces(), migrated);
        // Saved as spaces on the first change; the older file stays as it was.
        let create = |s: &mut Spaces| s.create("Work", Default::default());
        projects.change_spaces(create).unwrap();
        let kept = std::fs::read_to_string(&legacy).unwrap();
        assert_eq!(kept, r#"["/a", "/b c", "/a"]"#);
        let again = load(tmp.path());
        assert_eq!(*again.spaces(), *projects.spaces());
        assert_eq!(again.spaces().current, "space-1");
        assert_eq!(again.current(), (vec![], None));
    }

    #[test]
    fn corrupt_files_are_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let no_space = br#"{"current":"x","spaces":[]}"#;
        let long = vec![b' '; FILE_LIMIT as usize + 1];
        for name in ["spaces.json", "projects.json"] {
            let file = tmp.path().join(name);
            for bad in [&b"{not json"[..], &long, no_space] {
                std::fs::write(&file, bad).unwrap();
                assert_eq!(*load(tmp.path()).spaces(), Spaces::with(vec![]));
                assert!(!file.exists());
                let aside = tmp.path().join(format!("{name}.corrupt"));
                assert_eq!(std::fs::read(aside).unwrap(), bad);
            }
        }
    }

    #[test]
    fn saved_spaces_are_private_and_load_back() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("data/hive");
        // Longer than a few KiB, well within the read limit.
        let mut paths: Vec<String> = (0..500).map(|i| format!("/projects/{i:04}")).collect();
        paths.push("/b c".to_owned());
        let spaces = Spaces::with(paths);
        save(&dir.join("spaces.json"), &spaces).unwrap();
        let meta = std::fs::metadata(dir.join("spaces.json")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(*load(&dir).spaces(), spaces);
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        // The data "directory" is a file, so nothing can be written under it.
        let blocker = tmp.path().join("data");
        std::fs::write(&blocker, "").unwrap();
        let projects = load(&blocker);
        let create = |s: &mut Spaces| s.create("Work", Default::default());
        let err = projects.change_spaces(create).unwrap_err();
        assert!(err.starts_with("cannot save "), "{err}");
        assert_eq!(*projects.spaces(), Spaces::with(vec![]));
        // A refused request saves nothing either.
        let refused = projects.change_spaces(|s| s.select("nope"));
        assert_eq!(refused, Err(r#"no space "nope""#.to_owned()));
        // A request that changes nothing does not write at all.
        assert_eq!(projects.change_spaces(|s| s.select("default")), Ok(()));
    }

    #[test]
    fn a_list_too_big_to_read_back_is_not_saved() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("spaces.json");
        let long = |i: usize| format!("/{i}/{}", "x".repeat(4000));
        let paths: Vec<String> = (0..300).map(long).collect();
        let err = save(&file, &Spaces::with(paths)).unwrap_err();
        assert_eq!(err.to_string(), "the list would be over 1048576 bytes");
        assert!(!file.exists());
        // Just within the limit is saved.
        let json = serde_json::to_vec_pretty(&Spaces::with(vec![])).unwrap();
        let fill = FILE_LIMIT as usize - json.len() - 20;
        let fits = Spaces::with(vec![format!("/{}", "x".repeat(fill))]);
        let size = serde_json::to_vec_pretty(&fits).unwrap().len() as u64;
        assert!(size <= FILE_LIMIT && size > FILE_LIMIT - 40, "{size}");
        save(&file, &fits).unwrap();
    }

    #[test]
    fn a_terminal_outside_every_project_gets_no_space_environment() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(load(tmp.path()).terminal_env("/anywhere"), (vec![], None));
    }
}
