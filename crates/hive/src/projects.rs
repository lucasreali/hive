//! Projects: git repositories inside WSL that the app follows (#4), grouped in spaces
//! (`hive::spaces`). The service owns the list and keeps it in `<data>/hive/spaces.json`; the
//! flat `projects.json` of earlier versions (a JSON array of top-level paths) becomes the
//! "Default" space until the first change is saved.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use hive_protocol::{Control, Project, ProjectError, SpaceEnv, Worktree};
use serde::de::DeserializeOwned;

use crate::paths::canonical;
use crate::spaces::Spaces;
use crate::worktree::{self, WORKTREES_DIR};
use crate::wrapper::write_atomic;
use crate::{git, health, procs};

/// Largest spaces or project list file read.
const FILE_LIMIT: u64 = 1024 * 1024;
/// Most processes named when a worktree is in use.
const BUSY_SHOWN: usize = 5;
/// Most entries of a group's folder read for its repositories (14.1).
const GROUP_ENTRIES: usize = 1024;

pub struct Projects {
    file: PathBuf,
    spaces: Mutex<Spaces>,
    /// Each project with its worktrees as git last listed them (9.14), until [`Projects::forget`].
    /// One lock per project, held while git lists it (9.20).
    listed: Mutex<HashMap<String, Arc<Mutex<Option<Project>>>>>,
    /// The Claude config folders the file's spaces had before accounts (12.2), with their
    /// space's name, until they are accounts ([`Projects::migrate_accounts`]).
    legacy: Mutex<Vec<(String, String)>>,
}

impl Projects {
    /// Loads the spaces from `file`; without it, the projects listed in `legacy` (the file of
    /// earlier versions) make the "Default" space. A missing file is an empty list; an
    /// unreadable or invalid one is moved aside to `<file>.corrupt` with a warning, and the
    /// list starts empty.
    pub fn load(file: PathBuf, legacy: &Path) -> Self {
        let value: io::Result<serde_json::Value> = read(&file);
        let accounts = value.as_ref().map_or_else(|_| Vec::new(), legacy_accounts);
        let loaded = value
            .and_then(|value| serde_json::from_value(value).map_err(io::Error::other))
            .and_then(|s: Spaces| s.check().map_err(io::Error::other));
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
            listed: Mutex::default(),
            legacy: Mutex::new(accounts),
        }
    }

    /// Hands the Claude config folders spaces had (see `legacy`) to `make`, which makes them
    /// accounts, then saves the spaces without them. Nothing changes when `make` fails (the
    /// warning is logged): the next start tries again, so nothing is lost. (Not generic: every
    /// caller shares one copy of it, whose coverage the tests give.)
    pub fn migrate_accounts(
        &self,
        make: &mut dyn FnMut(Vec<(String, String)>) -> Result<(), String>,
    ) {
        let mut legacy = self.legacy.lock().unwrap_or_else(PoisonError::into_inner);
        if legacy.is_empty() {
            return;
        }
        let saved = make(legacy.clone())
            .and_then(|()| save(&self.file, &self.spaces()).map_err(|err| err.to_string()));
        match saved {
            Ok(()) => legacy.clear(),
            Err(err) => {
                eprintln!("hive: warning: the spaces' Claude folders are not accounts yet: {err}")
            }
        }
    }

    /// Every project with its worktrees, in the order they were added, each group (14.1)
    /// followed by its repositories.
    pub fn list(&self) -> Vec<Project> {
        let paths = self.all();
        let entries = self.entries(&paths);
        entries.into_iter().map(|e| self.build(e)).collect()
    }

    /// Every space's project ids, groups included.
    fn all(&self) -> Vec<String> {
        self.spaces().projects().cloned().collect()
    }

    /// What `paths` (project ids of the spaces) hold, without asking git: each project, and
    /// each group followed by its repositories, those of its folder's direct children that no
    /// space follows on their own.
    fn entries(&self, paths: &[String]) -> Vec<Entry> {
        let (groups, taken): (HashSet<String>, HashSet<String>) = {
            let spaces = self.spaces();
            let groups = spaces.spaces.iter().flat_map(|s| s.groups.iter().cloned());
            (groups.collect(), spaces.projects().cloned().collect())
        };
        let mut entries = Vec::new();
        for path in paths {
            if !groups.contains(path) {
                entries.push(Entry::Repo(path.clone(), None));
                continue;
            }
            let (inside, error) = match scan(Path::new(path)) {
                Ok(inside) => (inside, None),
                Err(err) => (Vec::new(), Some(format!("cannot read {path}: {err}"))),
            };
            entries.push(Entry::Group(path.clone(), error));
            let inside = inside.into_iter().filter(|id| !taken.contains(id));
            entries.extend(inside.map(|id| Entry::Repo(id, Some(path.clone()))));
        }
        entries
    }

    /// The project of `entry`, with its worktrees.
    fn build(&self, entry: Entry) -> Project {
        match entry {
            Entry::Group(path, error) => group(&path, error),
            Entry::Repo(id, parent) => Project {
                parent,
                ..self.listed(&id)
            },
        }
    }

    /// The followed project or group `id` with its worktrees (git lists only its own); else
    /// as the repository `id` alone.
    fn project(&self, id: &str) -> Project {
        let entry = self.entries(&self.all()).into_iter().find(|e| e.id() == id);
        self.build(entry.unwrap_or_else(|| Entry::Repo(id.to_owned(), None)))
    }

    /// Whether the followed project `id` is a group (14.1).
    pub fn is_group(&self, id: &str) -> bool {
        self.spaces().is_group(id)
    }

    /// The repository `id` with its worktrees; git lists them only when they are not known.
    fn listed(&self, id: &str) -> Project {
        let slot = (self.listed.lock().unwrap_or_else(PoisonError::into_inner))
            .entry(id.to_owned())
            .or_default()
            .clone();
        // Held while git runs: requests at once list a project once, and a git that hangs
        // holds up only the requests on its own project.
        let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
        slot.get_or_insert_with(|| project(id)).clone()
    }

    /// Forgets every project's worktrees: they changed, or may have (the health tick). The
    /// next request lists them again.
    pub fn forget(&self) {
        (self.listed.lock().unwrap_or_else(PoisonError::into_inner)).clear();
    }

    /// The spaces and the current one, for the app.
    pub fn spaces_message(&self) -> Control {
        let spaces = self.spaces();
        Control::Spaces {
            spaces: spaces.spaces.clone(),
            current: spaces.current.clone(),
        }
    }

    /// The current space's projects, each group followed by its repositories.
    pub fn current(&self) -> Vec<Project> {
        let (paths, _) = self.spaces().current();
        let entries = self.entries(&paths);
        entries.into_iter().map(|e| self.build(e)).collect()
    }

    /// The current space's repository folders, its groups' too, without asking git.
    pub fn roots(&self) -> Vec<String> {
        let (paths, _) = self.spaces().current();
        let entries = self.entries(&paths).into_iter();
        let repos = entries.filter_map(|e| matches!(e, Entry::Repo(..)).then(|| e.id().to_owned()));
        repos.collect()
    }

    /// The current space's group folders (14.1).
    pub fn groups(&self) -> Vec<String> {
        let spaces = self.spaces();
        let (paths, _) = spaces.current();
        paths.into_iter().filter(|p| spaces.is_group(p)).collect()
    }

    /// The environment of the space of the project holding `cwd` (also what `gh` gets for a
    /// project, 9.30); the default one outside every project.
    pub fn space_env(&self, cwd: &str) -> SpaceEnv {
        // Only the projects holding `cwd` when some do, so git runs for them alone; else
        // every project (a linked worktree may be anywhere).
        // ponytail: a worktree of one project inside another's folder takes the outer one's space.
        // Resolved first, as `place` does: `..` or a link may lead into another project.
        let real = canonical(Path::new(cwd)).unwrap_or_default();
        let mut ids = self.all();
        ids.retain(|id| real.starts_with(id));
        let listed = match ids.is_empty() {
            true => self.list(),
            false => self
                .entries(&ids)
                .into_iter()
                .map(|e| self.build(e))
                .collect(),
        };
        let place = place(&listed, cwd);
        // A group's repository is in its group's space (14.1).
        let placed = place.and_then(|(id, _)| listed.into_iter().find(|p| p.id == id));
        let top = placed.map(|p| p.parent.unwrap_or(p.id));
        let spaces = self.spaces();
        let space = top.and_then(|project| spaces.of(&project).cloned());
        space.map(|s| s.env).unwrap_or_default()
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
        drop(spaces);
        self.forget();
        Ok(())
    }

    /// Follows the git repository containing `path` in the current space; a folder outside
    /// git holding repositories becomes a group (14.1), into which the current space's
    /// projects among them move. Adding one already there (a group's repository too) changes
    /// nothing and answers the same project; one in another space is refused (a project is in
    /// one space only).
    pub fn add(&self, path: &str) -> Result<Project, (ProjectError, String)> {
        let (id, inside) = match validate(path) {
            Ok(root) => (root.to_string_lossy().into_owned(), None),
            Err((ProjectError::NotAGitRepository, message)) => {
                let dir = canonical(Path::new(path)).ok();
                let inside = dir.as_deref().map(scan).and_then(Result::ok);
                match dir.zip(inside.filter(|repos| !repos.is_empty())) {
                    Some((dir, repos)) => (dir.to_string_lossy().into_owned(), Some(repos)),
                    // No repository inside: refused as before.
                    None => return Err((ProjectError::NotAGitRepository, message)),
                }
            }
            Err(refused) => return Err(refused),
        };
        // A repository of a followed group is followed already, in its group's space.
        let held = match self.entries(&self.all()).into_iter().find(|e| e.id() == id) {
            Some(Entry::Repo(_, parent)) => parent,
            _ => None,
        };
        // Checked and added under one lock, so two adds at once cannot both add it.
        let mut other = false;
        let added = self.change_spaces(|spaces| {
            let added = match (&inside, &held) {
                (Some(inside), _) => spaces.add_group(id.clone(), inside),
                // Followed: nothing changes, or the other space's name.
                (None, Some(group)) => spaces.add(group.clone()),
                (None, None) => spaces.add(id.clone()),
            };
            added.map_err(|name| {
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
        Ok(self.project(&id))
    }

    /// Stops following the project `id` (9.28), a group with its repositories (14.1); nothing
    /// on disk changes. Refused while a process of one of Hive's terminals (their session
    /// ids, `terminals`, as `proc` lists processes) works in it or one of its worktrees, and
    /// for a group's repository (it follows its folder). Answers its worktrees' paths and a
    /// group's repositories.
    pub fn remove(
        &self,
        id: &str,
        proc: procs::Source,
        terminals: &HashSet<i32>,
    ) -> io::Result<(Vec<String>, Vec<String>)> {
        let entries = self.entries(&self.all());
        let repos: Vec<String> = match entries.iter().find(|e| e.id() == id) {
            None => return Err(not_followed(id)),
            Some(Entry::Repo(_, Some(group))) => {
                return Err(io::Error::other(format!(
                    "{id} is in the group {group}: remove the group to stop following it"
                )));
            }
            Some(Entry::Repo(_, None)) => vec![id.to_owned()],
            Some(Entry::Group(..)) => (entries.iter())
                .filter(|e| matches!(e, Entry::Repo(_, Some(group)) if group == id))
                .map(|e| e.id().to_owned())
                .collect(),
        };
        let worktrees: Vec<String> = (repos.iter())
            .flat_map(|repo| self.listed(repo).worktrees)
            .map(|w| w.path)
            .collect();
        // The root too, in case its worktrees cannot be listed (e.g. its folder is gone).
        let dirs = std::iter::once(id).chain(worktrees.iter().map(String::as_str));
        let mut busy: Vec<procs::Proc> = dirs
            .flat_map(|dir| procs::inside(proc, Path::new(dir)))
            .filter(|p| terminals.contains(&p.session))
            .collect();
        busy.sort_by_key(|p| p.pid);
        busy.dedup();
        refuse(busy)?;
        self.change_spaces(|spaces| spaces.remove(id))
            .map_err(io::Error::other)?;
        let inside = repos.into_iter().filter(|repo| repo != id).collect();
        Ok((worktrees, inside))
    }

    /// The branches of the followed project `id`.
    pub fn branches(&self, id: &str) -> io::Result<worktree::Branches> {
        worktree::branches(&self.root(id)?)
    }

    /// Checks a new worktree name for the followed project `id`, as `create` would.
    pub fn validate_worktree_name(&self, id: &str, name: &str) -> io::Result<()> {
        worktree::check_name(&self.root(id)?, name).map(drop)
    }

    /// [`worktree::fetch_base`] in the followed project `id`; none in another folder.
    pub fn fetch_base(&self, id: &str, base: &str, env: &[(&str, String)]) -> Option<String> {
        worktree::fetch_base(&self.root(id).ok()?, base, env)
    }

    /// `hive worktree create` in the followed project `id`; answers the project with its
    /// updated worktrees.
    pub fn create_worktree(
        &self,
        id: &str,
        name: &str,
        base: Option<&str>,
    ) -> io::Result<(Project, worktree::Created)> {
        let created = worktree::create(&self.root(id)?, name, base);
        self.forget();
        Ok((self.project(id), created?))
    }

    /// Removes the linked worktree `path` of a followed project, with `--force` when `force`;
    /// answers the project with its updated worktrees. Without `force`, a worktree that a
    /// process (as `proc` lists them) works in is kept, as git keeps one with changes.
    /// `before` (given the project's id) runs once those checks passed; its failure keeps the
    /// worktree unless `force`.
    pub fn remove_worktree(
        &self,
        path: &str,
        force: bool,
        proc: procs::Source,
        before: impl FnOnce(&str) -> io::Result<()>,
    ) -> io::Result<Project> {
        let (owner, _) = self.linked(path)?;
        if !force {
            unused(proc, Path::new(path))?;
        }
        let ran = before(&owner.id);
        if !force {
            ran?;
        }
        let removed = worktree::remove_path(Path::new(&owner.id), Path::new(path), force);
        self.forget();
        removed?;
        Ok(self.project(&owner.id))
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
        unused(proc, Path::new(path))?;
        let to = worktree::rename(Path::new(&owner.id), Path::new(path), name);
        self.forget();
        let to = to?;
        Ok((self.project(&owner.id), to.to_string_lossy().into_owned()))
    }

    /// The followed project holding the worktree `path`, when it is not the main one.
    fn linked(&self, path: &str) -> io::Result<(Project, Worktree)> {
        match self.holding(path)? {
            (_, wt) if wt.main => Err(io::Error::other(format!(
                "{path} is the project's main worktree"
            ))),
            found => Ok(found),
        }
    }

    /// The followed project holding the worktree `path` (its main one included), and it.
    pub fn holding(&self, path: &str) -> io::Result<(Project, Worktree)> {
        let mut repos = self.list().into_iter().filter(|p| !p.group);
        let found = repos.find_map(|p| {
            let wt = p.worktrees.iter().find(|w| w.path == path).cloned();
            wt.map(|wt| (p, wt))
        });
        found.ok_or_else(|| {
            io::Error::other(format!("{path} is not a worktree of a followed project"))
        })
    }

    /// The followed project `id` with its worktrees.
    pub fn followed(&self, id: &str) -> io::Result<Project> {
        self.root(id).map(|_| self.project(id))
    }

    /// `path` when it is a worktree of a followed project: the path comes from the app.
    pub fn worktree(&self, path: &str) -> io::Result<PathBuf> {
        followed(&self.list(), path).map(|(dir, _)| dir)
    }

    /// Only followed repositories are acted on (a group's too, never a group): the id comes
    /// from the app.
    fn root(&self, id: &str) -> io::Result<PathBuf> {
        let mut entries = self.entries(&self.all()).into_iter();
        if entries.any(|e| matches!(&e, Entry::Repo(repo, _) if repo == id)) {
            return Ok(PathBuf::from(id));
        }
        Err(not_followed(id))
    }

    fn spaces(&self) -> MutexGuard<'_, Spaces> {
        self.spaces.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The worktree `path` of `projects` and the branch it is compared with
/// ([`health::branch`]); an error when no project has it.
pub fn followed(projects: &[Project], path: &str) -> io::Result<(PathBuf, Option<String>)> {
    // A group's folder is no git worktree (14.1).
    for project in projects.iter().filter(|p| !p.group) {
        if let Some(w) = project.worktrees.iter().find(|w| w.path == path) {
            let branch = health::branch(project, w).map(str::to_owned);
            return Ok((PathBuf::from(path), branch));
        }
    }
    Err(io::Error::other(format!(
        "{path} is not a worktree of a followed project"
    )))
}

/// The followed worktree containing `cwd`, as `(project id, worktree id)`. Claude worktrees
/// live inside the main one, so the deepest match wins (#19). Both sides are resolved first,
/// so `..` or a link cannot take a path out of a worktree; one that does not resolve is in none.
pub fn place(projects: &[Project], cwd: &str) -> Option<(String, String)> {
    let cwd = canonical(Path::new(cwd)).ok()?;
    projects
        .iter()
        .flat_map(|p| p.worktrees.iter().map(move |w| (p, w)))
        .filter_map(|(p, w)| Some((p, w, canonical(Path::new(&w.path)).ok()?)))
        .filter(|(_, _, real)| cwd.starts_with(real))
        .max_by_key(|(_, _, real)| real.as_os_str().len())
        .map(|(p, w, _)| (p.id.clone(), w.id.clone()))
}

/// Refuses a folder (resolved) of a worktree before it is renamed, moved or removed: one that is
/// or holds a worktree of `projects`, or that some process (e.g. a terminal or an agent) works
/// in. Its path would change under them.
pub fn held(projects: &[Project], folder: &Path, proc: procs::Source) -> io::Result<()> {
    let holds = |w: &&Worktree| {
        let real = canonical(Path::new(&w.path));
        real.is_ok_and(|real| real.starts_with(folder))
    };
    if let Some(w) = projects.iter().flat_map(|p| &p.worktrees).find(holds) {
        return Err(io::Error::other(format!(
            "it holds the worktree {}",
            w.path
        )));
    }
    unused(proc, folder)
}

/// Refuses a worktree that some process (e.g. a terminal or an agent) works in.
fn unused(proc: procs::Source, path: &Path) -> io::Result<()> {
    refuse(procs::inside(proc, path))
}

/// Refuses when some process (`busy`) works there, naming them.
fn refuse(mut busy: Vec<procs::Proc>) -> io::Result<()> {
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

/// Each space's Claude config folder in a spaces file written before accounts (12.2), with
/// the space's name.
fn legacy_accounts(value: &serde_json::Value) -> Vec<(String, String)> {
    let spaces = value["spaces"].as_array().map_or(&[][..], Vec::as_slice);
    let account = |space: &serde_json::Value| {
        let dir = space["env"]["claude_config_dir"].as_str()?;
        Some((space["name"].as_str()?.to_owned(), dir.to_owned()))
    };
    spaces.iter().filter_map(account).collect()
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
    #[cfg(windows)]
    crate::windows::local(path).map_err(|err| (ProjectError::NotFound, err.to_string()))?;
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

/// What a space's project id holds.
enum Entry {
    /// A repository, with the group holding it when it is a group's.
    Repo(String, Option<String>),
    /// A group (14.1), with why its folder cannot be read, if it cannot.
    Group(String, Option<String>),
}

impl Entry {
    fn id(&self) -> &str {
        match self {
            Entry::Repo(id, _) | Entry::Group(id, _) => id,
        }
    }
}

fn not_followed(id: &str) -> io::Error {
    io::Error::other(format!("{id} is not a followed project"))
}

/// The repositories directly inside the folder `dir`, a group's (14.1): its subfolders (not
/// links) holding a `.git` folder, sorted. Only the first [`GROUP_ENTRIES`] entries are read;
/// a name that is not UTF-8 is skipped.
fn scan(dir: &Path) -> io::Result<Vec<String>> {
    let mut repos = Vec::new();
    for entry in std::fs::read_dir(dir)?.take(GROUP_ENTRIES).flatten() {
        let folder = entry.file_type().is_ok_and(|t| t.is_dir());
        let git = entry.path().join(".git").symlink_metadata();
        let name = entry.file_name();
        match name.to_str() {
            Some(name) if folder && git.is_ok_and(|m| m.is_dir()) => {
                repos.push(dir.join(name).to_string_lossy().into_owned())
            }
            _ => {}
        }
    }
    repos.sort();
    Ok(repos)
}

/// A group (14.1): no repository, so its one worktree is its folder, where its own
/// terminals and agents are placed, with no branch and no status.
fn group(path: &str, error: Option<String>) -> Project {
    let name = file_name(Path::new(path)).unwrap_or(path.to_owned());
    let folder = Worktree {
        id: path.to_owned(),
        name: name.clone(),
        path: path.to_owned(),
        branch: None,
        main: true,
        claude: false,
        status: None,
    };
    Project {
        id: path.to_owned(),
        name,
        path: path.to_owned(),
        worktrees: vec![folder],
        error,
        group: true,
        parent: None,
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
        group: false,
        parent: None,
    }
}

/// Describes `git worktree list` for the sidebar, skipping bare entries and worktrees whose
/// directory is gone (git lists them as prunable until `git worktree prune`).
fn worktrees(root: &Path, list: Vec<worktree::Worktree>) -> Vec<Worktree> {
    let claude_dir = worktree::worktrees_dir(root);
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn wt(path: &str, branch: Option<&str>, bare: bool) -> worktree::Worktree {
        worktree::Worktree {
            path: path.into(),
            branch: branch.map(Into::into),
            bare,
            prunable: false,
        }
    }

    #[cfg(unix)]
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
            let err = unused(procs::Source::Dir(proc.path()), &dir).unwrap_err();
            let elsewhere = Path::new("/elsewhere");
            assert!(unused(procs::Source::Dir(proc.path()), elsewhere).is_ok());
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

    #[cfg(unix)]
    #[test]
    fn a_project_is_removed_only_when_no_terminal_of_hive_works_in_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/f"), "kept").unwrap();
        let id = root.display().to_string();
        let projects = load(tmp.path());
        projects.change_spaces(|s| s.add(id.clone())).unwrap();
        // Two processes in the project: a terminal of Hive's (session 7) and one outside Hive.
        let proc = tmp.path().join("proc");
        for (pid, session) in [(10, 7), (11, 11)] {
            let entry = proc.join(pid.to_string());
            std::fs::create_dir_all(&entry).unwrap();
            let stat = format!("{pid} (p{pid}) S 1 {session} {session} 0");
            std::fs::write(entry.join("stat"), stat).unwrap();
            std::os::unix::fs::symlink(root.join("src"), entry.join("cwd")).unwrap();
        }
        let proc = procs::Source::Dir(&proc);
        let remove = |id: &str, sessions: &[i32]| {
            projects.remove(id, proc, &sessions.iter().copied().collect())
        };

        let err = remove(&id, &[7, 8]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "in use by p10 (10): close its terminals first"
        );
        let err = remove("/elsewhere", &[]).unwrap_err();
        assert_eq!(err.to_string(), "/elsewhere is not a followed project");
        assert_eq!(load(tmp.path()).spaces().projects().count(), 1);

        // A process outside Hive's terminals does not keep it.
        assert_eq!(remove(&id, &[8]).unwrap(), (vec![], vec![]));
        assert!(projects.list().is_empty());
        assert_eq!(*load(tmp.path()).spaces(), Spaces::with(vec![]));
        assert_eq!(std::fs::read_to_string(root.join("src/f")).unwrap(), "kept");
        let err = remove(&id, &[]).unwrap_err();
        assert_eq!(err.to_string(), format!("{id} is not a followed project"));
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_holding_a_worktree_or_a_process_is_held() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap().join("r");
        let at = |p: &str| root.join(p);
        std::fs::create_dir_all(at("src/x")).unwrap();
        std::fs::create_dir_all(at(".claude/worktrees/a")).unwrap();
        std::fs::create_dir_all(at(".claude/other")).unwrap();
        let (r, a) = (root.to_string_lossy(), at(".claude/worktrees/a"));
        let list = vec![
            wt(&r, Some("main"), false),
            wt(&a.to_string_lossy(), None, false),
        ];
        let projects = [Project {
            id: r.to_string(),
            name: String::new(),
            path: r.to_string(),
            worktrees: worktrees(&root, list),
            error: None,
            group: false,
            parent: None,
        }];
        let proc = tempfile::tempdir().unwrap();
        let held = |folder: &str| {
            held(&projects, &at(folder), procs::Source::Dir(proc.path()))
                .map_err(|err| err.to_string())
        };
        assert_eq!(held("src"), Ok(()));
        assert_eq!(held(".claude/other"), Ok(()));
        let holds = format!("it holds the worktree {}", a.display());
        for folder in [".claude", ".claude/worktrees", ".claude/worktrees/a"] {
            assert_eq!(held(folder), Err(holds.clone()), "{folder}");
        }
        // A process working in it, or in a folder inside it.
        let entry = proc.path().join("7");
        std::fs::create_dir(&entry).unwrap();
        std::fs::write(entry.join("stat"), "7 (bash) S 1 7 7 0").unwrap();
        std::os::unix::fs::symlink(at("src/x"), entry.join("cwd")).unwrap();
        let busy = Err("in use by bash (7): close its terminals first".to_owned());
        assert_eq!(held("src"), busy);
        assert_eq!(held("src/x"), busy);
        assert_eq!(held(".claude/other"), Ok(()));
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

    #[cfg(unix)]
    #[test]
    fn an_agent_is_placed_in_the_deepest_worktree_containing_its_cwd() {
        // The projects are added through a link to their real folder: both sides resolve.
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        for dir in [
            "r/src/x",
            "r/.claude/worktrees/a/src",
            "r/.claude/worktrees/ab",
            "r2/y",
        ] {
            std::fs::create_dir_all(real.join(dir)).unwrap();
        }
        std::fs::create_dir_all(real.join("elsewhere")).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // A link from inside a worktree to a folder outside every one.
        std::os::unix::fs::symlink(real.join("elsewhere"), real.join("r/out")).unwrap();
        let at_link = |p: &str| link.join(p).to_string_lossy().into_owned();
        let project = |root: &str, list| Project {
            id: root.into(),
            name: String::new(),
            path: root.into(),
            worktrees: worktrees(Path::new(root), list),
            error: None,
            group: false,
            parent: None,
        };
        let (r, a, r2) = (
            at_link("r"),
            at_link("r/.claude/worktrees/a"),
            at_link("r2"),
        );
        let projects = [
            project(
                &r,
                vec![
                    // A worktree whose folder does not resolve is in no match.
                    wt(&at_link("r/.claude/worktrees/gone"), None, false),
                    wt(&r, Some("main"), false),
                    wt(&a, None, false),
                ],
            ),
            project(&r2, vec![wt(&r2, None, false)]),
        ];
        let place = |cwd: &Path| place(&projects, &cwd.to_string_lossy());
        let at = |p: &str, w: &str| Some((p.to_owned(), w.to_owned()));
        for base in [&link, &real] {
            assert_eq!(place(&base.join("r")), at(&r, &r));
            assert_eq!(place(&base.join("r/src/x")), at(&r, &r));
            assert_eq!(place(&base.join("r/.claude/worktrees/a")), at(&r, &a));
            assert_eq!(place(&base.join("r/.claude/worktrees/a/src")), at(&r, &a));
            assert_eq!(place(&base.join("r/.claude/worktrees/ab")), at(&r, &r));
            // Whole path components only: r2 is not inside r.
            assert_eq!(place(&base.join("r2/y")), at(&r2, &r2));
            assert_eq!(place(&base.join("elsewhere")), None);
            // `..` and a link cannot take a path out of a worktree.
            assert_eq!(place(&base.join("r/../elsewhere")), None);
            assert_eq!(place(&base.join("r/src/../../r2/y")), at(&r2, &r2));
            assert_eq!(place(&base.join("r/out")), None);
            // Nor can a path that does not resolve.
            assert_eq!(place(&base.join("r/missing")), None);
        }
        assert_eq!(place(Path::new("")), None);
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
        // On Windows, never a network or device path: opening one would send the user's
        // credentials.
        #[cfg(windows)]
        for path in [
            r"\\host\share\repo",
            r"\\?\UNC\host\share\repo",
            r"\\.\C:\repo",
            r"\\?\GLOBALROOT\Device\repo",
        ] {
            let said = format!(
                r"{path}: network and device paths are not supported: use a drive path such as C:\…"
            );
            assert_eq!(projects.add(path), Err((ProjectError::NotFound, said)));
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
    fn worktrees_are_listed_again_only_once_forgotten() {
        use crate::health::tests::{commit, run};
        let tmp = tempfile::tempdir().unwrap();
        let top = canonical(tmp.path()).unwrap();
        let [root, other] = ["r", "o"].map(|name| {
            let root = top.join(name);
            std::fs::create_dir(&root).unwrap();
            run(&root, &["init", "-q", "-b", "main"]);
            commit(&root, "a");
            root
        });
        let id = root.display().to_string();
        let projects = load(tmp.path());
        let count = || projects.followed(&id).unwrap().worktrees.len();
        projects.add(&id).unwrap();
        let add = |name: &str| {
            let path = top.join(name).display().to_string();
            run(&root, &["worktree", "add", "-q", "-b", name, &path]);
        };
        // Made behind Hive's back: not seen until forgotten.
        add("w1");
        assert_eq!((count(), projects.list()[0].worktrees.len()), (1, 1));
        projects.forget();
        assert_eq!(count(), 2);
        // Adding a project forgets them too; adding one already followed changes nothing.
        add("w2");
        projects.add(&id).unwrap();
        assert_eq!(count(), 2);
        projects.add(&other.display().to_string()).unwrap();
        assert_eq!(count(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_git_is_an_error_of_its_project_only() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = load(tmp.path());
        let [hung, fine] = ["hung", "fine"].map(|name| {
            let root = tmp.path().canonicalize().unwrap().join(name);
            let status = std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .status()
                .unwrap();
            assert!(status.success());
            let id = root.display().to_string();
            projects.change_spaces(|s| s.add(id.clone())).unwrap();
            id
        });
        // Git reads the repository's config, which includes a pipe nobody writes to, as a
        // repository on a hung network drive would hang.
        let git_dir = Path::new(&hung).join(".git");
        nix::unistd::mkfifo(&git_dir.join("hang"), nix::sys::stat::Mode::S_IRWXU).unwrap();
        let mut config = std::fs::OpenOptions::new()
            .append(true)
            .open(git_dir.join("config"))
            .unwrap();
        std::io::Write::write_all(&mut config, b"[include]\n\tpath = hang\n").unwrap();
        let started = std::time::Instant::now();
        let (listed, fine_alone) = std::thread::scope(|scope| {
            let listing = scope.spawn(|| projects.list());
            // While git hangs in the other project, this one is listed at once (9.20).
            std::thread::sleep(std::time::Duration::from_millis(500));
            let alone = std::time::Instant::now();
            projects.followed(&fine).unwrap();
            let alone = alone.elapsed();
            (listing.join().unwrap(), alone)
        });
        assert!(started.elapsed() < git::TIME_LIMIT * 2, "not killed");
        assert!(fine_alone < git::TIME_LIMIT / 2, "waited {fine_alone:?}");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, hung);
        assert!(listed[0].worktrees.is_empty());
        let error = listed[0].error.as_deref().unwrap_or_default();
        assert!(error.ends_with("took longer than 10 s"), "{error}");
        assert_eq!(listed[1].id, fine);
        assert_eq!(listed[1].error, None);
        assert_eq!(listed[1].worktrees[0].path, fine);
    }

    #[test]
    fn missing_files_are_an_empty_default_space() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = load(tmp.path());
        assert!(projects.list().is_empty());
        assert_eq!(*projects.spaces(), Spaces::with(vec![]));
        assert_eq!(projects.current(), vec![]);
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
        assert_eq!(again.current(), vec![]);
    }

    #[test]
    fn the_spaces_claude_folders_leave_them_once_they_are_accounts() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("spaces.json");
        let old = r#"{"current":"space-1","spaces":[
            {"id":"default","name":"Default","projects":["/a"],"env":{"claude_config_dir":"/d","git_name":"Me"}},
            {"id":"space-1","name":"Work","env":{"claude_config_dir":"/w"}},
            {"id":"space-2","name":"Plain","env":{"claude_config_dir":null}},
            {"id":"space-3","name":"Bare"}]}"#;
        std::fs::write(&file, old).unwrap();
        let projects = load(tmp.path());
        assert_eq!(projects.spaces().current, "space-1");
        assert_eq!(
            projects.spaces().spaces[0].env.git_name.as_deref(),
            Some("Me")
        );
        let expected = vec![
            ("Default".into(), "/d".into()),
            ("Work".into(), "/w".into()),
        ];
        // What was handed over, each time; the next one fails while `fail` is set.
        let handed = std::cell::RefCell::new(Vec::new());
        let fail = std::cell::Cell::new(true);
        let mut make = |legacy| {
            handed.borrow_mut().push(legacy);
            if fail.get() {
                Err("no".to_owned())
            } else {
                Ok(())
            }
        };
        // A failure changes nothing: they are handed over again.
        projects.migrate_accounts(&mut make);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), old);
        fail.set(false);
        projects.migrate_accounts(&mut make);
        assert_eq!(*handed.borrow(), [expected.clone(), expected]);
        // Saved without them, projects and git identity kept; never handed over again.
        let saved = std::fs::read_to_string(&file).unwrap();
        assert!(!saved.contains("claude_config_dir"), "{saved}");
        projects.migrate_accounts(&mut make);
        let again = load(tmp.path());
        assert_eq!(*again.spaces(), *projects.spaces());
        again.migrate_accounts(&mut make);
        assert_eq!(handed.borrow().len(), 2);
        // A space file that cannot be written keeps them for the next start.
        std::fs::write(&file, old).unwrap();
        let blocked = load(tmp.path());
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        blocked.migrate_accounts(&mut make);
        blocked.migrate_accounts(&mut make);
        assert_eq!(handed.borrow().len(), 4);
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
        #[cfg(unix)]
        {
            let meta = std::fs::metadata(dir.join("spaces.json")).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
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
        // Exactly the limit is saved; one byte more is not.
        let sized = |n: usize| Spaces::with(vec![format!("/{}", "x".repeat(n))]);
        let len = |s: &Spaces| serde_json::to_vec_pretty(s).unwrap().len();
        let n = FILE_LIMIT as usize - (len(&sized(0)));
        assert_eq!(len(&sized(n)) as u64, FILE_LIMIT);
        save(&file, &sized(n)).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().len(), FILE_LIMIT);
        assert!(save(&file, &sized(n + 1)).is_err());
    }

    #[test]
    fn a_terminal_outside_every_project_gets_no_space_environment() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(load(tmp.path()).space_env("/anywhere"), SpaceEnv::default());
    }

    /// A git repository made at `dir`.
    fn repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        crate::health::tests::run(dir, &["init", "-q", "-b", "main"]);
    }

    /// The ids of `projects`, a group's marked `+`, a repository of a group's with its group.
    fn shown(projects: &[Project]) -> Vec<String> {
        let shown = |p: &Project| match (&p.parent, p.group) {
            (_, true) => format!("+{}", p.id),
            (Some(group), _) => format!("{} in {group}", p.id),
            (None, _) => p.id.clone(),
        };
        projects.iter().map(shown).collect()
    }

    #[test]
    fn a_folder_of_repositories_is_a_group_of_them() {
        let tmp = tempfile::tempdir().unwrap();
        let top = canonical(tmp.path()).unwrap();
        let at = |p: &str| top.join(p).display().to_string();
        let g = at("g");
        for name in ["api", "web", "loose/deep"] {
            repo(&top.join("g").join(name));
        }
        // Not repositories of the group: a plain folder, a file, a link to a repository, and a
        // folder whose `.git` is a file.
        std::fs::write(top.join("g/notes.txt"), "").unwrap();
        std::fs::create_dir_all(top.join("g/linked")).unwrap();
        std::fs::write(top.join("g/linked/.git"), "gitdir: /elsewhere").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(top.join("g/api"), top.join("g/link")).unwrap();
        repo(&top.join("other"));
        std::fs::create_dir(top.join("empty")).unwrap();
        let projects = load(&top.join("data"));
        // A folder with no repository inside is refused as before.
        let (error, message) = projects.add(&at("empty")).unwrap_err();
        assert_eq!(error, ProjectError::NotAGitRepository);
        assert!(message.contains("is not in a git repository"), "{message}");

        // A repository followed on its own moves into the group added after it.
        projects.add(&at("g/web")).unwrap();
        projects.add(&at("other")).unwrap();
        // Added through a path that is not its own: the group is the folder, resolved.
        let added = projects.add(&format!("{g}/api/..")).unwrap();
        assert_eq!(shown(std::slice::from_ref(&added)), [format!("+{g}")]);
        assert_eq!((added.name.as_str(), &added.error), ("g", &None));
        let folder = &added.worktrees[..];
        assert_eq!(folder.len(), 1);
        assert_eq!(
            (folder[0].path.as_str(), folder[0].main),
            (g.as_str(), true)
        );
        assert_eq!((&folder[0].branch, &folder[0].status), (&None, &None));
        let listed = [
            at("other"),
            format!("+{g}"),
            format!("{} in {g}", at("g/api")),
            format!("{} in {g}", at("g/web")),
        ];
        assert_eq!(shown(&projects.list()), listed);
        assert_eq!(shown(&projects.current()), listed);
        assert_eq!(
            projects.spaces().spaces[0].projects,
            [at("other"), g.clone()]
        );
        assert_eq!(projects.roots(), [at("other"), at("g/api"), at("g/web")]);
        assert_eq!(projects.groups(), [g.as_str()]);
        assert!(projects.is_group(&g) && !projects.is_group(&at("g/api")));

        // Its repositories are projects (with their group), the group itself is none.
        assert_eq!(
            projects.followed(&at("g/api")).unwrap().parent,
            Some(g.clone())
        );
        assert!(projects.branches(&at("g/web")).is_ok());
        let err = projects.branches(&g).unwrap_err();
        assert_eq!(err.to_string(), format!("{g} is not a followed project"));
        assert!(projects.worktree(&g).is_err());
        assert!(projects.holding(&g).is_err());
        assert!(projects.worktree(&at("g/api")).is_ok());

        // Adding it, or one of its repositories, again changes nothing.
        let again = projects.add(&g).unwrap();
        assert_eq!(again, added);
        let api = projects.add(&at("g/api")).unwrap();
        assert_eq!(shown(&[api]), [format!("{} in {g}", at("g/api"))]);
        assert_eq!(shown(&projects.list()), listed);
        // From another space both are refused.
        projects
            .change_spaces(|s| s.create("Work", Default::default()))
            .unwrap();
        for path in [&g, &at("g/web")] {
            let (error, message) = projects.add(path).unwrap_err();
            assert_eq!(error, ProjectError::InOtherSpace);
            assert_eq!(message, format!("{path} is already in the space Default"));
        }
        assert_eq!(projects.current(), vec![]);
        assert_eq!(projects.roots(), Vec::<String>::new());
        assert_eq!(projects.groups(), Vec::<String>::new());
        // A repository another space follows stays there, out of the group.
        let follow = |s: &mut Spaces| {
            s.spaces[1].projects.push(at("g/web"));
            Ok(())
        };
        projects.change_spaces(follow).unwrap();
        let mut moved = listed.to_vec();
        moved.pop();
        moved.push(at("g/web"));
        assert_eq!(shown(&projects.list()), moved);

        // It is kept as the projects are, and loads back the same.
        assert_eq!(shown(&load(&top.join("data")).list()), moved);
    }

    #[test]
    fn a_groups_repositories_follow_its_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let top = canonical(tmp.path()).unwrap();
        let g = top.join("g").display().to_string();
        repo(&top.join("g/api"));
        let projects = load(&top.join("data"));
        projects.add(&g).unwrap();
        let ids = || {
            projects
                .list()
                .into_iter()
                .map(|p| p.id)
                .collect::<Vec<_>>()
        };
        let web = top.join("g/web").display().to_string();
        let api = top.join("g/api").display().to_string();
        assert_eq!(ids(), [g.clone(), api.clone()]);
        // Made or removed in its folder: listed or not, with no change to the spaces.
        repo(&top.join("g/web"));
        assert_eq!(ids(), [g.clone(), api.clone(), web.clone()]);
        std::fs::remove_dir_all(top.join("g/api")).unwrap();
        assert_eq!(ids(), [g.clone(), web]);
        assert_eq!(projects.spaces().spaces[0].projects, [g.as_str()]);
        // Its folder gone: the group stays, with why.
        std::fs::remove_dir_all(top.join("g")).unwrap();
        let listed = projects.list();
        assert_eq!(listed.len(), 1);
        let error = listed[0].error.as_deref().unwrap_or_default();
        assert!(error.starts_with(&format!("cannot read {g}: ")), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_group_and_its_repositories_are_placed_and_removed_together() {
        let tmp = tempfile::tempdir().unwrap();
        let top = canonical(tmp.path()).unwrap();
        let at = |p: &str| top.join(p).display().to_string();
        let g = at("g");
        repo(&top.join("g/api"));
        repo(&top.join("g/web"));
        std::fs::create_dir_all(top.join("g/docs")).unwrap();
        let projects = load(&top.join("data"));
        let env = SpaceEnv {
            git_name: Some("Group".into()),
            ..SpaceEnv::default()
        };
        projects
            .change_spaces(|s| s.update("default", "D", env.clone()))
            .unwrap();
        projects.add(&g).unwrap();
        // A cwd in its folder is the group's; in a repository, the repository's.
        let list = projects.list();
        assert_eq!(place(&list, &at("g/docs")), Some((g.clone(), g.clone())));
        assert_eq!(place(&list, &at("g/api")), Some((at("g/api"), at("g/api"))));
        // Both get the group's space environment.
        assert_eq!(projects.space_env(&at("g/docs")), env);
        assert_eq!(projects.space_env(&at("g/web")), env);

        // A process of a terminal of Hive's in the folder keeps the group; a repository of it
        // is removed only with it.
        let proc = top.join("proc");
        let entry = proc.join("10");
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(entry.join("stat"), "10 (p10) S 1 7 7 0").unwrap();
        std::os::unix::fs::symlink(top.join("g/docs"), entry.join("cwd")).unwrap();
        let proc = procs::Source::Dir(&proc);
        let remove = |id: &str, sessions: &[i32]| {
            projects.remove(id, proc, &sessions.iter().copied().collect())
        };
        let err = remove(&g, &[7]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "in use by p10 (10): close its terminals first"
        );
        let err = remove(&at("g/api"), &[]).unwrap_err();
        let said = format!(
            "{} is in the group {g}: remove the group to stop following it",
            at("g/api")
        );
        assert_eq!(err.to_string(), said);
        let removed = remove(&g, &[]).unwrap();
        assert_eq!(
            removed,
            (
                vec![at("g/api"), at("g/web")],
                vec![at("g/api"), at("g/web")]
            )
        );
        assert_eq!(projects.list(), vec![]);
        assert!(!projects.is_group(&g));
        assert!(top.join("g/api/.git").is_dir());
    }

    #[test]
    fn a_spaces_file_from_before_groups_loads_and_saves_as_it_was() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("spaces.json");
        let old = r#"{"current":"default","spaces":[{"id":"default","name":"Default","projects":["/a"],"env":{"git_name":null,"git_email":null,"gh_config_dir":null,"gh_account":null}}]}"#;
        std::fs::write(&file, old).unwrap();
        let projects = load(tmp.path());
        assert_eq!(*projects.spaces(), Spaces::with(vec!["/a".into()]));
        assert!(!projects.is_group("/a"));
        projects
            .change_spaces(|s| s.create("Work", Default::default()))
            .unwrap();
        let saved = std::fs::read_to_string(&file).unwrap();
        assert!(!saved.contains("groups"), "{saved}");
    }
}
