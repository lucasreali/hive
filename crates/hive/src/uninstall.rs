//! Uninstalling Hive (23.5, D22.17): what goes ([`plan`]), moving it to the Trash ([`remove`]),
//! and `hive uninstall` ([`command`]).
//!
//! Only what Hive itself made is listed: its folder (`~/.hive`, or `HIVE_HOME`), the folders of
//! before 22.1, the system's folders for the app id, the logs, the runtime folder, the
//! `~/.local/bin/hive` shortcut when it points at Hive's `bin/hive`, and the app. Never
//! `~/.claude`, `~/.claude.json`, an account folder Hive did not make, the projects, their
//! worktrees and branches, or the credentials Claude Code keeps (#45).

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use hive_protocol::{UninstallFailure, UninstallItem, UninstallKind};

use crate::paths::Paths;

/// The app id (`tauri.conf.json`'s `identifier`): the name of the system's folders for the app.
pub const APP_ID: &str = "com.lucas.hive";

/// The app bundle's name.
pub const APP_NAME: &str = "Hive.app";

/// Where `hive uninstall` looks for the app instead of `/Applications` and `~/Applications`
/// (an absolute folder): the tests' own, so that a test never finds the installed app.
pub const APPLICATIONS: &str = "HIVE_APPLICATIONS";

/// What in Hive's folder is the service's program, regenerated at every start: it goes even
/// when the data is kept.
const PROGRAM: [&str; 3] = ["bin", "shell", "hive-hooks.json"];

/// What else Hive writes in its folder ([`Paths`]), and nothing else is removed from it: a
/// `HIVE_HOME` naming another folder (`~/.claude`, a projects folder) loses nothing of its own.
/// A name followed by a `.` and more is that file's own (`spaces.json.corrupt`, a `.tmp`).
const DATA: [&str; 9] = [
    "settings.json",
    "spaces.json",
    "projects.json",
    "ports.json",
    "offered-pulls.json",
    "usage.json",
    "open-sessions.json",
    "shared",
    "accounts",
];

/// Whether `entry` (of Hive's folder) is one of `names` or a file of one of them.
fn named(entry: &Path, names: &[&str]) -> bool {
    let name = entry
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let own = |known: &&str| {
        name == **known
            || name
                .strip_prefix(*known)
                .is_some_and(|rest| rest.starts_with('.'))
    };
    names.iter().any(own)
}

/// Whether `data` is Hive's folder: `~/.hive` itself, or one holding the hooks settings the
/// service writes at every start (a `HIVE_HOME`); never the home nor one holding it (compared
/// once links are resolved).
fn hives(data: &Path, home: &Path) -> bool {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    let hooks = std::fs::symlink_metadata(data.join("hive-hooks.json")).is_ok_and(|m| m.is_file());
    let marked = data == home.join(".hive") || hooks;
    marked && !real(home).starts_with(real(data))
}

/// The system's folders (under `~/Library`) for the app id that hold the app's own data: the
/// WebView's storage, the window's place and the preferences. Kept with the data.
const LIBRARY_DATA: [&str; 3] = ["Application Support/", "WebKit/", "Preferences/"];

/// Where Hive's things are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    pub home: PathBuf,
    pub paths: Paths,
    /// The data and config folders of before 22.1 ([`Paths::legacy`]).
    pub legacy: Option<(PathBuf, PathBuf)>,
    /// Whether the system keeps folders for the app id under `~/Library` (macOS).
    pub library: bool,
}

impl Places {
    /// The places of the user `HOME` names, with `paths` (the service's own, which may be the
    /// old folders when they could not be moved, 22.1); `None` without an absolute `HOME`.
    #[cfg(unix)]
    pub fn from_env(paths: &Paths) -> Option<Self> {
        let var = |key: &str| std::env::var_os(key);
        let home = var("HOME").map(PathBuf::from).filter(|h| h.is_absolute())?;
        Some(Self {
            home,
            paths: paths.clone(),
            legacy: Paths::legacy(var),
            library: cfg!(target_os = "macos"),
        })
    }
}

/// What to remove.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Choice {
    /// Keep the settings, spaces and accounts: only the app, the program, the shortcut, caches
    /// and logs go.
    pub keep_data: bool,
    /// Also the account folders Hive made (22.3); never with `keep_data`.
    pub accounts: bool,
    /// The app bundle, when it is one ([`bundle`]).
    pub app: Option<PathBuf>,
}

/// What [`plan`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// What goes, in order: the app last.
    pub items: Vec<UninstallItem>,
    /// The account folders Hive made, whether they go or not.
    pub account_folders: Vec<String>,
}

/// `path` as the app bundle to remove: an absolute path to a folder (not a link) named
/// `Hive.app`; anything else is not removed.
pub fn bundle(path: Option<&Path>) -> Option<PathBuf> {
    let path = path.filter(|p| p.is_absolute())?;
    let named = path.file_name().is_some_and(|name| name == APP_NAME);
    let folder = std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
    (named && folder).then(|| path.to_owned())
}

/// What uninstalling with `choice` removes, only what exists.
pub fn plan(places: &Places, choice: &Choice) -> Plan {
    let home = &places.home;
    let data = &places.paths.data;
    let mut found = Found::default();
    let accounts_dir = places.paths.accounts_dir();
    // A folder that is not Hive's (a mistaken `HIVE_HOME`) is never emptied.
    let own_folder = hives(data, home);
    let listed: Vec<PathBuf> = if own_folder {
        entries(data)
            .into_iter()
            .filter(|e| named(e, &PROGRAM) || named(e, &DATA))
            .collect()
    } else {
        Vec::new()
    };
    let account_folders = if own_folder {
        entries(&accounts_dir)
    } else {
        Vec::new()
    };
    let program = |entry: &Path| named(entry, &PROGRAM);
    if !choice.keep_data {
        for entry in listed.iter().filter(|e| !program(e) && **e != accounts_dir) {
            found.add(entry, UninstallKind::Data);
        }
        if let Some((old_data, old_config)) = &places.legacy {
            found.add(old_data, UninstallKind::Data);
            found.add(old_config, UninstallKind::Data);
        }
        if places.library {
            for folder in LIBRARY_DATA {
                let name = if folder == "Preferences/" {
                    format!("{folder}{APP_ID}.plist")
                } else {
                    format!("{folder}{APP_ID}")
                };
                found.add(&home.join("Library").join(name), UninstallKind::Data);
            }
        }
        if choice.accounts {
            for folder in &account_folders {
                found.add(folder, UninstallKind::Account);
            }
        }
    }
    for entry in listed.iter().filter(|e| program(e)) {
        found.add(entry, UninstallKind::Program);
    }
    let shortcut = home.join(".local/bin/hive");
    let targets = [Some(data.join("bin/hive"))]
        .into_iter()
        .chain([places.legacy.as_ref().map(|(old, _)| old.join("bin/hive"))]);
    let link = std::fs::read_link(&shortcut).ok();
    if targets
        .flatten()
        .any(|target| link.as_ref() == Some(&target))
    {
        found.add(&shortcut, UninstallKind::Program);
    }
    if places.library {
        let caches = home.join("Library/Caches").join(APP_ID);
        found.add(&caches, UninstallKind::Cache);
        found.add(&home.join("Library/Logs/Hive"), UninstallKind::Cache);
    }
    found.add(&places.paths.logs, UninstallKind::Cache);
    found.add(&places.paths.runtime, UninstallKind::Cache);
    if let Some(app) = &choice.app {
        found.add(app, UninstallKind::App);
    }
    Plan {
        items: found.items,
        account_folders: account_folders.iter().map(|p| text(p)).collect(),
    }
}

/// The items found so far: each path once, never the Claude folders.
#[derive(Default)]
struct Found {
    items: Vec<UninstallItem>,
}

impl Found {
    fn add(&mut self, path: &Path, kind: UninstallKind) {
        let name = path.file_name().map(|n| n.to_string_lossy());
        let claude = name.is_some_and(|n| n == ".claude" || n == ".claude.json");
        let path = text(path);
        let exists = std::fs::symlink_metadata(&path).is_ok();
        if exists && !claude && !self.items.iter().any(|item| item.path == path) {
            self.items.push(UninstallItem { path, kind });
        }
    }
}

/// The entries of the folder `dir`, sorted; none when it cannot be read.
fn entries(dir: &Path) -> Vec<PathBuf> {
    let read = std::fs::read_dir(dir).into_iter().flatten().flatten();
    let mut entries: Vec<PathBuf> = read.map(|entry| entry.path()).collect();
    entries.sort();
    entries
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// How an entry goes: to the Trash on macOS, deleted elsewhere. Behind a trait so that no test
/// moves real files.
pub trait Remove {
    fn remove(&self, path: &Path) -> io::Result<()>;
}

/// The system's way ([`Remove`]).
pub struct System;

impl Remove for System {
    fn remove(&self, path: &Path) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        return crate::macos::trash(path);
        #[cfg(not(target_os = "macos"))]
        return delete(path);
    }
}

/// Deletes `path` for good: a folder with what it holds, or a file or link itself.
#[cfg(not(target_os = "macos"))]
fn delete(path: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Removes `plan`'s items in order with `how`; one gone meanwhile is skipped. Then Hive's
/// folder and its `accounts/`, when nothing is left in them. What could not go is returned.
pub fn remove(plan: &Plan, paths: &Paths, how: &dyn Remove) -> Vec<UninstallFailure> {
    let mut failed = Vec::new();
    for item in &plan.items {
        let path = Path::new(&item.path);
        if std::fs::symlink_metadata(path).is_err() {
            continue;
        }
        if let Err(err) = how.remove(path) {
            let (path, error) = (item.path.clone(), err.to_string());
            failed.push(UninstallFailure { path, error });
        }
    }
    // Only empty folders: `remove_dir` refuses any other.
    let _ = std::fs::remove_dir(paths.accounts_dir());
    let _ = std::fs::remove_dir(&paths.data);
    failed
}

/// What the list's heading says the items' fate is.
const FATE: &str = if cfg!(target_os = "macos") {
    "moved to the Trash"
} else {
    "deleted"
};

/// `hive uninstall`: prints what goes, asks `[y/N]` on `input` unless `yes`, then removes it
/// with `how`. `Ok(false)` when nothing was removed because the answer was not yes; an error
/// names what could not go.
pub fn command(
    places: &Places,
    choice: &Choice,
    yes: bool,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    how: &dyn Remove,
) -> io::Result<bool> {
    let plan = plan(places, choice);
    if plan.items.is_empty() {
        writeln!(out, "Nothing of Hive's is left to remove.")?;
        return Ok(true);
    }
    writeln!(out, "These will be {FATE}:")?;
    for item in &plan.items {
        writeln!(out, "  {}", item.path)?;
    }
    if !choice.accounts && !plan.account_folders.is_empty() && !choice.keep_data {
        let kept = "Kept: the account folders Hive made (--accounts removes them too).";
        writeln!(out, "{kept}")?;
    }
    let never = "Never touched: ~/.claude, ~/.claude.json, your projects, their worktrees and \
                 branches. To remove Claude Code's login too, run /logout in claude first.";
    writeln!(out, "{never}")?;
    if !yes {
        write!(out, "Uninstall Hive? [y/N] ")?;
        out.flush()?;
        let mut answer = String::new();
        input.read_line(&mut answer)?;
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            writeln!(out, "Nothing removed.")?;
            return Ok(false);
        }
    }
    let failed = remove(&plan, &places.paths, how);
    if failed.is_empty() {
        writeln!(out, "Hive is uninstalled.")?;
        return Ok(true);
    }
    let why: Vec<String> = failed
        .iter()
        .map(|f| format!("{}: {}", f.path, f.error))
        .collect();
    Err(io::Error::other(format!(
        "could not remove:\n  {}",
        why.join("\n  ")
    )))
}

/// The app bundle `hive uninstall` removes: `Hive.app` in the first of `dirs` that has one.
pub fn find_app(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .find_map(|dir| bundle(Some(&dir.join(APP_NAME))))
}

/// Where `hive uninstall` looks for the app: [`APPLICATIONS`] when absolute, else
/// `/Applications` and `~/Applications` (macOS; elsewhere there is no app to remove).
pub fn app_dirs(var: impl Fn(&str) -> Option<std::ffi::OsString>, home: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    let system = vec![PathBuf::from("/Applications"), home.join("Applications")];
    #[cfg(not(target_os = "macos"))]
    let system = {
        let _ = home;
        Vec::new()
    };
    let over = var(APPLICATIONS).map(PathBuf::from);
    over.filter(|dir| dir.is_absolute())
        .map_or(system, |dir| vec![dir])
}

/// The service's lock, held, when no service runs; an error when one does (it would put back
/// its socket and lock, and its terminals hold their worktrees). `None` without a runtime
/// folder or lockfile: then nothing runs either.
pub fn not_running(paths: &Paths) -> io::Result<Option<std::fs::File>> {
    let lock = match std::fs::File::options().write(true).open(paths.lock()) {
        Ok(lock) => lock,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let locked = lock.try_lock();
    if let Err(std::fs::TryLockError::WouldBlock) = locked {
        let running = "Hive is running: quit it first, or choose Hive > Uninstall Hive… in the app";
        return Err(io::Error::other(running));
    }
    locked.map_err(io::Error::from)?;
    Ok(Some(lock))
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Records what it was asked to remove; refuses the paths in `refuse`; removes nothing.
    #[derive(Default)]
    struct Recorder {
        removed: RefCell<Vec<PathBuf>>,
        refuse: Vec<PathBuf>,
    }

    impl Remove for Recorder {
        fn remove(&self, path: &Path) -> io::Result<()> {
            self.removed.borrow_mut().push(path.to_owned());
            if self.refuse.iter().any(|p| p == path) {
                return Err(io::Error::other("refused"));
            }
            Ok(())
        }
    }

    struct Setup {
        tmp: tempfile::TempDir,
        places: Places,
    }

    impl Setup {
        /// A home with every thing Hive makes (macOS's layout when `library`), and what it must
        /// never touch.
        fn new(library: bool) -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let home = tmp.path().join("home");
            let data = home.join(".hive");
            let paths = Paths {
                runtime: tmp.path().join("tmp/hive"),
                data: data.clone(),
                config: data.clone(),
                logs: home.join("Library/Logs/Hive"),
            };
            let legacy = (home.join(".local/share/hive"), home.join(".config/hive"));
            let s = Self {
                places: Places {
                    home,
                    paths,
                    legacy: Some(legacy),
                    library,
                },
                tmp,
            };
            for dir in [
                ".hive/bin",
                ".hive/shell",
                ".hive/shared",
                ".hive/accounts/work",
                ".hive/accounts/home",
                ".local/share/hive",
                ".config/hive",
                ".local/bin",
                "Library/Application Support/com.lucas.hive",
                "Library/Caches/com.lucas.hive",
                "Library/WebKit/com.lucas.hive",
                "Library/Preferences",
                "Library/Logs/Hive",
                ".claude",
                ".claude-work",
                "src/shop/.claude/worktrees/a",
            ] {
                std::fs::create_dir_all(s.home().join(dir)).unwrap();
            }
            for file in [
                ".hive/bin/hive",
                ".hive/hive-hooks.json",
                ".hive/settings.json",
                ".hive/spaces.json",
                "Library/Preferences/com.lucas.hive.plist",
                "Library/Preferences/com.other.plist",
                ".claude.json",
            ] {
                std::fs::write(s.home().join(file), "x").unwrap();
            }
            std::fs::create_dir_all(&s.places.paths.runtime).unwrap();
            let link = s.home().join(".local/bin/hive");
            std::os::unix::fs::symlink(s.home().join(".hive/bin/hive"), link).unwrap();
            s
        }

        fn home(&self) -> &Path {
            &self.places.home
        }

        /// `plan`'s paths, relative to the home (the runtime folder as `$TMPDIR/hive`).
        fn listed(&self, choice: &Choice) -> Vec<(String, UninstallKind)> {
            let tmp = self.tmp.path().join("tmp");
            plan(&self.places, choice)
                .items
                .into_iter()
                .map(|item| {
                    let path = Path::new(&item.path);
                    let short = match path.strip_prefix(self.home()) {
                        Ok(rest) => format!("~/{}", rest.display()),
                        Err(_) => format!("$TMPDIR/{}", path.strip_prefix(&tmp).unwrap().display()),
                    };
                    (short, item.kind)
                })
                .collect()
        }
    }

    fn owned(items: &[(&str, UninstallKind)]) -> Vec<(String, UninstallKind)> {
        items.iter().map(|(p, k)| ((*p).to_owned(), *k)).collect()
    }

    use UninstallKind::{Account, App, Cache, Data, Program};

    #[test]
    fn every_thing_hive_made_goes_and_only_those() {
        let s = Setup::new(true);
        let app = s.tmp.path().join("Applications/Hive.app");
        std::fs::create_dir_all(&app).unwrap();
        let choice = Choice {
            app: Some(app.clone()),
            ..Choice::default()
        };
        let items = plan(&s.places, &choice).items;
        assert_eq!(items.last().unwrap().path, text(&app));
        assert_eq!(items.last().unwrap().kind, App);
        assert_eq!(
            s.listed(&Choice::default()),
            owned(&[
                ("~/.hive/settings.json", Data),
                ("~/.hive/shared", Data),
                ("~/.hive/spaces.json", Data),
                ("~/.local/share/hive", Data),
                ("~/.config/hive", Data),
                ("~/Library/Application Support/com.lucas.hive", Data),
                ("~/Library/WebKit/com.lucas.hive", Data),
                ("~/Library/Preferences/com.lucas.hive.plist", Data),
                ("~/.hive/bin", Program),
                ("~/.hive/hive-hooks.json", Program),
                ("~/.hive/shell", Program),
                ("~/.local/bin/hive", Program),
                ("~/Library/Caches/com.lucas.hive", Cache),
                ("~/Library/Logs/Hive", Cache),
                ("$TMPDIR/hive", Cache),
            ])
        );
        // The account folders Hive made are offered, not taken.
        let plan = plan(&s.places, &Choice::default());
        let accounts = s.home().join(".hive/accounts");
        assert_eq!(
            plan.account_folders,
            [text(&accounts.join("home")), text(&accounts.join("work"))]
        );
    }

    #[test]
    fn keeping_the_data_takes_only_the_program_the_shortcut_caches_and_logs() {
        let s = Setup::new(true);
        let choice = Choice {
            keep_data: true,
            accounts: true,
            app: None,
        };
        assert_eq!(
            s.listed(&choice),
            owned(&[
                ("~/.hive/bin", Program),
                ("~/.hive/hive-hooks.json", Program),
                ("~/.hive/shell", Program),
                ("~/.local/bin/hive", Program),
                ("~/Library/Caches/com.lucas.hive", Cache),
                ("~/Library/Logs/Hive", Cache),
                ("$TMPDIR/hive", Cache),
            ])
        );
        // Still offered, in case the box is wanted without keeping the rest.
        assert_eq!(plan(&s.places, &choice).account_folders.len(), 2);
    }

    #[test]
    fn hives_own_account_folders_go_only_when_asked() {
        let s = Setup::new(false);
        let choice = Choice {
            accounts: true,
            ..Choice::default()
        };
        let listed = s.listed(&choice);
        let accounts: Vec<_> = listed.iter().filter(|(_, k)| *k == Account).collect();
        assert_eq!(
            accounts,
            [
                &("~/.hive/accounts/home".to_owned(), Account),
                &("~/.hive/accounts/work".to_owned(), Account)
            ]
        );
        let without = s.listed(&Choice::default());
        assert!(without.iter().all(|(p, _)| !p.contains("accounts")));
    }

    #[test]
    fn off_macos_there_are_no_library_folders() {
        let s = Setup::new(false);
        let listed = s.listed(&Choice::default());
        assert!(
            listed
                .iter()
                .all(|(p, _)| !p.contains("Library/") || p == "~/Library/Logs/Hive")
        );
        // The logs folder is the service's own (here the macOS one), listed once.
        assert_eq!(listed.iter().filter(|(p, _)| p.contains("Logs")).count(), 1);
    }

    #[test]
    fn what_is_missing_is_not_listed() {
        let s = Setup::new(true);
        for gone in [
            ".local/share/hive",
            ".config/hive",
            "Library/Caches/com.lucas.hive",
        ] {
            std::fs::remove_dir_all(s.home().join(gone)).unwrap();
        }
        std::fs::remove_dir_all(&s.places.paths.runtime).unwrap();
        let listed = s.listed(&Choice::default());
        for gone in [
            "~/.local/share/hive",
            "~/.config/hive",
            "~/Library/Caches/com.lucas.hive",
            "$TMPDIR/hive",
        ] {
            assert!(listed.iter().all(|(p, _)| p != gone), "{gone}");
        }
        // No Hive folder at all.
        let mut places = s.places.clone();
        places.paths.data = s.home().join("missing");
        let plan = plan(&places, &Choice::default());
        assert!(plan.items.iter().all(|i| !i.path.contains(".hive/")));
        assert!(plan.account_folders.is_empty());
    }

    #[test]
    fn a_shortcut_hive_did_not_make_stays() {
        let s = Setup::new(false);
        let link = s.home().join(".local/bin/hive");
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("/usr/local/bin/hive", &link).unwrap();
        assert!(
            s.listed(&Choice::default())
                .iter()
                .all(|(p, _)| p != "~/.local/bin/hive")
        );
        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, "mine").unwrap();
        assert!(
            s.listed(&Choice::default())
                .iter()
                .all(|(p, _)| p != "~/.local/bin/hive")
        );
        // One pointing at the old folders' `bin/hive` is Hive's.
        std::fs::remove_file(&link).unwrap();
        let old = s.home().join(".local/share/hive/bin/hive");
        std::os::unix::fs::symlink(&old, &link).unwrap();
        assert!(
            s.listed(&Choice::default())
                .iter()
                .any(|(p, _)| p == "~/.local/bin/hive")
        );
        let mut places = s.places.clone();
        places.legacy = None;
        let plan = plan(&places, &Choice::default());
        assert!(
            plan.items
                .iter()
                .all(|i| !i.path.ends_with(".local/bin/hive"))
        );
    }

    #[test]
    fn a_folder_that_is_not_hives_is_never_emptied() {
        let s = Setup::new(false);
        // The home, or a folder holding it, even marked as Hive's or spelled with `..`.
        std::fs::write(s.home().join("hive-hooks.json"), "{}").unwrap();
        std::fs::write(s.home().join("settings.json"), "{}").unwrap();
        let link = s.tmp.path().join("hive-link");
        std::os::unix::fs::symlink(s.home(), &link).unwrap();
        // A folder that is not marked as Hive's: Claude's, with a `settings.json` of its own.
        std::fs::write(s.home().join(".claude/settings.json"), "{}").unwrap();
        for data in [
            s.home().to_owned(),
            s.tmp.path().to_owned(),
            s.home().join("src/.."),
            link,
            s.home().join(".claude"),
        ] {
            let mut places = s.places.clone();
            places.paths.data = data.clone();
            places.legacy = None;
            let choice = Choice {
                accounts: true,
                ..Choice::default()
            };
            let plan = plan(&places, &choice);
            assert!(
                plan.items
                    .iter()
                    .all(|i| Path::new(&i.path).parent() != Some(&data)),
                "{data:?}: {:?}",
                plan.items
            );
            assert!(plan.account_folders.is_empty(), "{data:?}");
        }
    }

    #[test]
    fn only_what_hive_writes_in_its_folder_goes() {
        let s = Setup::new(false);
        for name in ["notes.txt", "spaces.jsonx", "binary"] {
            std::fs::write(s.home().join(".hive").join(name), "mine").unwrap();
        }
        std::fs::write(s.home().join(".hive/spaces.json.corrupt"), "{").unwrap();
        let listed = s.listed(&Choice::default());
        assert!(listed.contains(&("~/.hive/spaces.json.corrupt".to_owned(), Data)));
        for mine in ["notes.txt", "spaces.jsonx", "binary"] {
            let path = format!("~/.hive/{mine}");
            assert!(listed.iter().all(|(p, _)| *p != path), "{mine}");
        }
        // `~/.hive` is Hive's without its hooks settings too (gone with a `--keep-data`).
        std::fs::remove_file(s.home().join(".hive/hive-hooks.json")).unwrap();
        let listed = s.listed(&Choice::default());
        assert!(listed.contains(&("~/.hive/settings.json".to_owned(), Data)));
        // Another folder (a `HIVE_HOME`) only with them.
        let mut places = s.places.clone();
        places.paths.data = s.home().join("dev-hive");
        std::fs::create_dir(&places.paths.data).unwrap();
        std::fs::write(places.paths.data.join("settings.json"), "{}").unwrap();
        let some = |places: &Places| {
            !plan(places, &Choice::default())
                .items
                .iter()
                .all(|i| !i.path.contains("dev-hive"))
        };
        assert!(!some(&places));
        std::fs::write(places.paths.data.join("hive-hooks.json"), "{}").unwrap();
        assert!(some(&places));
        // What is left keeps Hive's folder.
        let plan = plan(&s.places, &Choice::default());
        remove(&plan, &s.places.paths, &Recorder::default());
        assert!(s.home().join(".hive/notes.txt").exists());
    }

    #[test]
    fn the_claude_folders_are_never_listed() {
        let s = Setup::new(false);
        // Even as the old folders (a mistaken XDG folder).
        let mut places = s.places.clone();
        places.legacy = Some((s.home().join(".claude"), s.home().join(".claude.json")));
        let plan = plan(&places, &Choice::default());
        assert!(
            plan.items.iter().all(|i| !i.path.contains(".claude")),
            "{:?}",
            plan.items
        );
    }

    #[test]
    fn only_a_folder_named_app_is_the_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Hive.app");
        assert_eq!(bundle(Some(&app)), None);
        std::fs::create_dir(&app).unwrap();
        assert_eq!(bundle(Some(&app)), Some(app.clone()));
        assert_eq!(bundle(Some(Path::new("Hive.app"))), None);
        assert_eq!(bundle(None), None);
        let other_app = tmp.path().join("other/Safari.app");
        std::fs::create_dir_all(&other_app).unwrap();
        assert_eq!(bundle(Some(&other_app)), None);
        let other = tmp.path().join("debug");
        std::fs::create_dir(&other).unwrap();
        assert_eq!(bundle(Some(&other)), None);
        let file = tmp.path().join("File.app");
        std::fs::write(&file, "").unwrap();
        assert_eq!(bundle(Some(&file)), None);
        let link = tmp.path().join("Link.app");
        std::os::unix::fs::symlink(&app, &link).unwrap();
        assert_eq!(bundle(Some(&link)), None);
        assert_eq!(
            find_app(&[tmp.path().join("none"), tmp.path().into()]),
            Some(app)
        );
        assert_eq!(find_app(&[tmp.path().join("none")]), None);
    }

    #[test]
    fn the_app_is_looked_for_in_the_applications_folders() {
        let home = Path::new("/h");
        let none = |_: &str| None;
        let dirs = app_dirs(none, home);
        #[cfg(target_os = "macos")]
        assert_eq!(
            dirs,
            [PathBuf::from("/Applications"), home.join("Applications")]
        );
        #[cfg(not(target_os = "macos"))]
        assert!(dirs.is_empty());
        let over = |key: &str| (key == APPLICATIONS).then(|| "/t/apps".into());
        assert_eq!(app_dirs(over, home), [PathBuf::from("/t/apps")]);
        let relative = |_: &str| Some("apps".into());
        assert_eq!(app_dirs(relative, home), dirs);
    }

    #[test]
    fn items_go_in_order_a_missing_one_is_skipped_and_a_failure_is_told() {
        let s = Setup::new(true);
        let plan = plan(&s.places, &Choice::default());
        let refused = s.home().join(".hive/bin");
        let how = Recorder {
            refuse: vec![refused.clone()],
            ..Recorder::default()
        };
        // Gone after the list was made.
        std::fs::remove_dir_all(s.home().join(".config/hive")).unwrap();
        let failed = remove(&plan, &s.places.paths, &how);
        assert_eq!(
            failed,
            [UninstallFailure {
                path: text(&refused),
                error: "refused".into()
            }]
        );
        let expected: Vec<PathBuf> = plan
            .items
            .iter()
            .map(|i| PathBuf::from(&i.path))
            .filter(|p| !p.ends_with(".config/hive"))
            .collect();
        assert_eq!(*how.removed.borrow(), expected);
        // Nothing was really removed, so Hive's folder stays.
        assert!(s.home().join(".hive/settings.json").exists());
    }

    #[test]
    fn hives_folder_goes_once_it_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join(".hive");
        std::fs::create_dir_all(data.join("accounts")).unwrap();
        let paths = Paths {
            runtime: tmp.path().join("run"),
            data: data.clone(),
            config: data.clone(),
            logs: tmp.path().join("run"),
        };
        remove(&Plan::default(), &paths, &Recorder::default());
        assert!(!data.exists());
        // Not when something is left.
        std::fs::create_dir_all(data.join("accounts/work")).unwrap();
        remove(&Plan::default(), &paths, &Recorder::default());
        assert!(data.join("accounts/work").exists());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_items_are_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("d");
        std::fs::create_dir_all(folder.join("e")).unwrap();
        std::fs::write(folder.join("e/f"), "x").unwrap();
        let file = tmp.path().join("f");
        std::fs::write(&file, "x").unwrap();
        let link = tmp.path().join("l");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        System.remove(&link).unwrap();
        assert!(folder.join("e/f").exists());
        System.remove(&folder).unwrap();
        System.remove(&file).unwrap();
        assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());
        assert!(System.remove(&file).is_err());
    }

    fn run(
        s: &Setup,
        choice: &Choice,
        yes: bool,
        answer: &str,
    ) -> (io::Result<bool>, String, Vec<PathBuf>) {
        let how = Recorder::default();
        let mut out = Vec::new();
        let done = command(
            &s.places,
            choice,
            yes,
            &mut answer.as_bytes(),
            &mut out,
            &how,
        );
        let removed = how.removed.borrow().clone();
        (done, String::from_utf8(out).unwrap(), removed)
    }

    #[test]
    fn the_command_lists_and_asks_before_removing() {
        let s = Setup::new(true);
        for no in ["", "\n", "n\n", "nope\n"] {
            let (done, out, removed) = run(&s, &Choice::default(), false, no);
            assert!(!done.unwrap());
            assert!(removed.is_empty());
            assert!(out.contains(&format!("These will be {FATE}:\n  ")), "{out}");
            assert!(out.contains(&format!("  {}\n", s.home().join(".hive/bin").display())));
            assert!(
                out.contains("Uninstall Hive? [y/N] Nothing removed.\n"),
                "{out}"
            );
            assert!(out.contains("--accounts removes them too"));
            assert!(out.contains("run /logout in claude first"));
        }
        for yes in ["y\n", "Y\n", " yes \n", "YES"] {
            let (done, out, removed) = run(&s, &Choice::default(), false, yes);
            assert!(done.unwrap());
            assert_eq!(
                removed.len(),
                plan(&s.places, &Choice::default()).items.len()
            );
            assert!(out.ends_with("[y/N] Hive is uninstalled.\n"), "{out}");
        }
    }

    #[test]
    fn yes_removes_without_asking() {
        let s = Setup::new(false);
        let choice = Choice {
            accounts: true,
            ..Choice::default()
        };
        let (done, out, removed) = run(&s, &choice, true, "n\n");
        assert!(done.unwrap());
        assert!(!out.contains("[y/N]"));
        assert!(!out.contains("--accounts"));
        assert!(removed.contains(&s.home().join(".hive/accounts/work")));
        // Keeping the data never mentions the account folders.
        let keep = Choice {
            keep_data: true,
            ..Choice::default()
        };
        let (_, out, removed) = run(&s, &keep, true, "");
        assert!(!out.contains("--accounts"));
        assert!(!removed.contains(&s.home().join(".hive/settings.json")));
    }

    #[test]
    fn the_command_says_when_nothing_is_left_or_what_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = Places {
            home: tmp.path().join("home"),
            paths: Paths {
                runtime: tmp.path().join("run"),
                data: tmp.path().join("home/.hive"),
                config: tmp.path().join("home/.hive"),
                logs: tmp.path().join("run"),
            },
            legacy: None,
            library: false,
        };
        let s = Setup { places: empty, tmp };
        let (done, out, removed) = run(&s, &Choice::default(), false, "");
        assert!(done.unwrap());
        assert_eq!(out, "Nothing of Hive's is left to remove.\n");
        assert!(removed.is_empty());
        let s = Setup::new(false);
        let refused = s.home().join(".hive/bin");
        let how = Recorder {
            refuse: vec![refused.clone()],
            ..Recorder::default()
        };
        let mut out = Vec::new();
        let err = command(
            &s.places,
            &Choice::default(),
            true,
            &mut &b""[..],
            &mut out,
            &how,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("could not remove:\n  {}: refused", refused.display())
        );
    }

    #[test]
    fn a_running_service_is_told_to_quit_first() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            runtime: tmp.path().join("run"),
            data: tmp.path().join("data"),
            config: tmp.path().join("data"),
            logs: tmp.path().join("run"),
        };
        // No runtime folder: nothing runs.
        assert!(not_running(&paths).unwrap().is_none());
        std::fs::create_dir(&paths.runtime).unwrap();
        std::fs::write(paths.lock(), "").unwrap();
        let held = not_running(&paths).unwrap().unwrap();
        let err = not_running(&paths).unwrap_err();
        assert!(
            err.to_string().contains("Hive is running: quit it first"),
            "{err}"
        );
        drop(held);
        assert!(not_running(&paths).unwrap().is_some());
        // A lock that cannot be opened.
        std::fs::remove_file(paths.lock()).unwrap();
        std::fs::create_dir(paths.lock()).unwrap();
        assert!(not_running(&paths).is_err());
    }

    #[test]
    fn the_places_come_from_home() {
        let paths = Paths::from_env();
        let places = Places::from_env(&paths);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let home = home.filter(|home| home.is_absolute());
        let expected = home.map(|home| Places {
            home,
            paths: paths.clone(),
            legacy: Paths::legacy(|key| std::env::var_os(key)),
            library: cfg!(target_os = "macos"),
        });
        assert_eq!(places, expected);
    }
}
