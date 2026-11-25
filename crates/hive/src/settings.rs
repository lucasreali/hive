//! The user's settings, owned by the service (#37) and kept in
//! `~/.hive/settings.json`. The app reads and saves them whole through the
//! protocol (`get_settings`, `set_settings`); ranges are checked here.

use std::ffi::OsStr;
use std::fs::DirBuilder;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use hive_protocol::{
    Account, AccountDir, ClaudeSettings, DEFAULT_ACCOUNT, GhSettings, HOST_LIMIT, ProjectScripts,
    Settings, check_host,
};

use crate::git::read_limited;
use crate::spaces;
use crate::wrapper::write_atomic;

/// Largest settings file read or written.
const FILE_LIMIT: u64 = 256 * 1024;
/// Longest font family, branch or script name kept.
const TEXT_LIMIT: usize = 256;
/// Longest script kept.
const SCRIPT_LIMIT: usize = 16 * 1024;
/// Most Claude accounts kept: checked first, since the other account checks are pairwise.
const ACCOUNT_LIMIT: usize = 64;

pub struct Store {
    file: PathBuf,
    /// The settings in use, and why the file was ignored (until the settings are saved).
    current: Mutex<(Settings, Option<String>)>,
    /// Where Hive makes the Claude account folders (22.3, [`crate::paths::Paths::accounts_dir`]);
    /// empty until [`Store::with_accounts`].
    accounts: PathBuf,
    /// The file's `claude.account`, the account selected before 22.5, until the spaces have it
    /// ([`Store::migrate_account`]): `Some(None)` for the default account or a folder no
    /// account has.
    selected: Mutex<Option<Option<String>>>,
}

impl Store {
    /// Loads `file`. A missing file is the defaults; an unreadable or invalid one is the
    /// defaults too, with a warning for the app, and stays untouched until the user saves.
    pub fn load(file: PathBuf) -> Self {
        let (current, selected) = match read(&file) {
            Ok((settings, selected)) => ((settings, None), selected),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                ((Settings::default(), None), None)
            }
            Err(err) => {
                let warning = format!(
                    "Ignoring {}: {err}. Using the defaults until the settings are saved.",
                    file.display()
                );
                eprintln!("hive: warning: {warning}");
                ((Settings::default(), Some(warning)), None)
            }
        };
        Self {
            file,
            current: Mutex::new(current),
            accounts: PathBuf::new(),
            selected: Mutex::new(selected),
        }
    }

    /// Hands the account selected before 22.5 (when it is one of the accounts) to `give`, which
    /// makes it every space's, then saves the settings without it, so nothing changes on
    /// upgrade. Nothing is saved when `give` fails (the warning is logged): the next start
    /// tries again.
    pub fn migrate_account(&self, give: &mut dyn FnMut(&str) -> Result<(), String>) {
        let mut selected = self.selected.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(dir) = selected.as_ref() else {
            return;
        };
        let mut current = self.current();
        let settings = current.0.clone();
        let saved = (dir.as_deref().map_or(Ok(()), give))
            .and_then(|()| self.save_in(&mut current, settings).map(drop));
        match saved {
            Ok(()) => *selected = None,
            Err(err) => eprintln!(
                "hive: warning: the selected Claude account is not the spaces' yet: {err}"
            ),
        }
    }

    /// The store, making the Claude account folders in `accounts` (22.3).
    pub fn with_accounts(mut self, accounts: PathBuf) -> Self {
        mark(&mut self.current().0, &accounts);
        self.accounts = accounts;
        self
    }

    fn current(&self) -> MutexGuard<'_, (Settings, Option<String>)> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The settings in use, and why the file was ignored, if it was.
    pub fn get(&self) -> (Settings, Option<String>) {
        self.current().clone()
    }

    /// [`Store::set_with`] where no space uses an account.
    #[cfg(test)]
    pub fn set(&self, settings: Settings) -> Result<Settings, String> {
        self.set_with(settings, &|_| None, &|_| None)
    }

    /// Checks and saves `settings`, which are then in use. Nothing changes on a failure. A
    /// Claude account's folder must exist when it is new; one whose folder `using` names a
    /// space of (22.5) cannot be removed. So with a gh config folder (24.13) and `using_gh`.
    /// (Both lock the spaces: they are never locked before the settings. Not generic: every
    /// caller shares one copy, whose coverage the tests give.)
    pub fn set_with(
        &self,
        mut settings: Settings,
        using: &dyn Fn(&str) -> Option<String>,
        using_gh: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Settings, String> {
        tidy(&mut settings);
        check(&settings)?;
        let mut current = self.current();
        let known = &current.0.claude.accounts;
        for account in &settings.claude.accounts {
            if known.iter().all(|a| a.config_dir != account.config_dir) {
                spaces::claude_config_dir(Some(account.config_dir.clone()), true)?;
            }
        }
        let kept = |dir: &str| settings.claude.accounts.iter().any(|a| a.config_dir == dir);
        for gone in known.iter().filter(|a| !kept(&a.config_dir)) {
            if let Some(space) = using(&gone.config_dir) {
                let name = &gone.name;
                return Err(format!(
                    "{name} is the Claude account of the space {space}: choose another one for it first"
                ));
            }
        }
        let (known, dirs) = (&current.0.gh.config_dirs, &settings.gh.config_dirs);
        for dir in dirs.iter().filter(|dir| !known.contains(dir)) {
            spaces::gh_config_dir(Some(dir.clone()), true)?;
        }
        for gone in known.iter().filter(|dir| !dirs.contains(dir)) {
            if let Some(space) = using_gh(gone) {
                return Err(format!(
                    "The GitHub account of the space {space} is in {gone}: choose another one for it first"
                ));
            }
        }
        settle(&mut settings);
        mark(&mut settings, &self.accounts);
        self.save_in(&mut current, settings)
    }

    /// Adds the Claude account `name` (22.3, D22.1) with a new folder of its own, made private
    /// (0700) in [`Store::accounts`] and named after it ([`slug`]). Refused, with nothing made,
    /// when the name has no letter or digit, when the folder is there already, or when the
    /// account would not be saved (its name taken).
    pub fn create_account(&self, name: &str) -> Result<Settings, String> {
        let slug = slug(name);
        if slug.is_empty() {
            return Err(format!(
                "{name:?} cannot name a folder: give the account a letter or a digit"
            ));
        }
        let dir = self.accounts.join(&slug);
        let config_dir = dir.to_str().ok_or("Hive's folder is no text")?.to_owned();
        let mut current = self.current();
        let mut settings = current.0.clone();
        let account = Account {
            name: name.to_owned(),
            config_dir,
            hive_folder: true,
        };
        settings.claude.accounts.push(account);
        check(&settings)?;
        let made = crate::mode::folder(DirBuilder::new().recursive(true), 0o700)
            .create(&self.accounts)
            .and_then(|()| crate::mode::folder(&mut DirBuilder::new(), 0o700).create(&dir));
        made.map_err(|err| match err.kind() {
            io::ErrorKind::AlreadyExists => format!(
                "{} exists already: give the account another name",
                dir.display()
            ),
            _ => format!("Cannot make {}: {err}", dir.display()),
        })?;
        settle(&mut settings);
        self.save_in(&mut current, settings).inspect_err(|_| {
            // Saved nothing: the empty folder just made goes too.
            let _ = std::fs::remove_dir(&dir);
        })
    }

    /// Removes the Claude account whose folder is `dir` (22.3), as [`Store::set_with`] would
    /// (refused, before anything is deleted, when `using` names a space of it, 22.5), then
    /// deletes the folder when `delete` and Hive made it (sessions, settings and skills; never
    /// credentials kept elsewhere, #45). The settings then in use, and why the folder could not
    /// be deleted. Refused, with nothing removed, for a folder Hive did not make.
    pub fn remove_account(
        &self,
        dir: &str,
        delete: bool,
        using: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(Settings, Option<String>), String> {
        if delete && !own(&self.accounts, dir) {
            return Err(format!("Hive did not make {dir}: it is never deleted"));
        }
        let mut settings = self.get().0;
        let before = settings.claude.accounts.len();
        settings.claude.accounts.retain(|a| a.config_dir != dir);
        if settings.claude.accounts.len() == before {
            return Err(format!("No Claude account has the folder {dir}"));
        }
        // Another account spelling the same folder otherwise keeps it (and its spaces use it).
        let same = |a: &&Account| Path::new(&a.config_dir) == Path::new(dir);
        if let Some(other) = settings
            .claude
            .accounts
            .iter()
            .find(same)
            .filter(|_| delete)
        {
            let other = &other.name;
            return Err(format!(
                "{other} has the folder {dir} too: it is not deleted"
            ));
        }
        // shortcut: the folder is deleted after the settings lock is released; a
        // `create_account` meanwhile finds it still there and is refused, so nothing is lost.
        // The gh config folders stay as they are: none is removed.
        let settings = self.set_with(settings, using, &|_| None)?;
        let deleted = match delete {
            // Not followed when it is a link: only the link goes.
            true => std::fs::remove_dir_all(dir).err(),
            false => None,
        };
        let why = deleted.map(|err| format!("Cannot delete {dir}: {err}"));
        Ok((settings, why))
    }

    /// Every Claude account's config folder but the default one's.
    pub fn account_dirs(&self) -> Vec<String> {
        let current = self.current();
        let accounts = current.0.claude.accounts.iter();
        accounts.map(|a| a.config_dir.clone()).collect()
    }

    /// The gh config folders (24.13) besides `gh`'s own.
    pub fn gh_dirs(&self) -> Vec<String> {
        self.current().0.gh.config_dirs.clone()
    }

    /// `dir` when it is one of the accounts' folders; else `None`, the default account (a
    /// space's account, 22.5, may name a folder no account has any more in a hand-edited file).
    pub fn known(&self, dir: Option<String>) -> Option<String> {
        dir.filter(|dir| self.account_dirs().contains(dir))
    }

    /// The Claude config folder a new terminal gets: the `asked` account's, which must be one
    /// of the accounts, else its `space`'s (22.5; `None`: the default account).
    pub fn claude_dir(
        &self,
        asked: Option<AccountDir>,
        space: Option<String>,
    ) -> Result<Option<String>, String> {
        match asked.map(|a| a.config_dir) {
            None => Ok(self.known(space)),
            Some(None) => Ok(None),
            Some(Some(dir)) if self.account_dirs().contains(&dir) => Ok(Some(dir)),
            Some(Some(dir)) => Err(format!("No Claude account has the folder {dir}")),
        }
    }

    /// The name of the account whose config folder is `dir` (21.7): `None`, or a folder no
    /// account has any more, is the default account's, as configured (21.4).
    pub fn account_name(&self, dir: Option<&str>) -> String {
        let current = self.current();
        let mut accounts = current.0.claude.accounts.iter();
        let named = dir.and_then(|dir| accounts.find(|a| a.config_dir == dir));
        let default = || default_name(&current.0.claude).to_owned();
        named.map_or_else(default, |a| a.name.clone())
    }

    /// Makes an account of each Claude config folder spaces had before accounts existed
    /// (12.2), given as `(space name, folder)`, unless an account has it already; each is
    /// named after its space. Adds each gh config folder spaces had before 24.13 (`gh`) to the
    /// settings' when they do not have it. Nothing is saved while the settings file is ignored
    /// (saving would replace it): the warning is returned.
    pub fn migrate(&self, legacy: Vec<(String, String)>, gh: Vec<String>) -> Result<(), String> {
        let mut current = self.current();
        if let Some(warning) = &current.1 {
            return Err(warning.clone());
        }
        let mut settings = current.0.clone();
        let claude = &mut settings.claude;
        for (name, config_dir) in legacy {
            // As every account folder is kept (22.2).
            let config_dir = spaces::trimmed(&config_dir).to_owned();
            if claude.accounts.iter().all(|a| a.config_dir != config_dir) {
                let name = free_name(claude, &name);
                let hive_folder = own(&self.accounts, &config_dir);
                claude.accounts.push(Account {
                    name,
                    config_dir,
                    hive_folder,
                });
            }
        }
        let dirs = &mut settings.gh.config_dirs;
        for dir in gh {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        if settings == current.0 {
            return Ok(());
        }
        check(&settings)?;
        self.save_in(&mut current, settings).map(drop)
    }

    /// Saves `settings` and makes them `current` (held by the caller). Nothing changes on a
    /// failure.
    fn save_in(
        &self,
        current: &mut (Settings, Option<String>),
        settings: Settings,
    ) -> Result<Settings, String> {
        save(&self.file, &settings)
            .map_err(|err| format!("Cannot save {}: {err}", self.file.display()))?;
        *current = (settings.clone(), None);
        Ok(settings)
    }

    /// The settings file.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The settings file, written with the settings in use when there is none yet (to be
    /// opened in an editor). An existing file is left alone, even an ignored one.
    pub fn ensure_file(&self) -> io::Result<&Path> {
        let current = self.current();
        if !self.file.exists() {
            save(&self.file, &current.0)?;
        }
        Ok(&self.file)
    }

    /// Rule 2's silence (`hive::states`): how long a working agent's terminal stays quiet
    /// before the agent waits for you.
    pub fn silence(&self) -> Duration {
        Duration::from_secs(self.current().0.agents.silence_secs.into())
    }

    /// Whether the computer is kept awake while agents work (17.17).
    pub fn keep_awake(&self) -> bool {
        self.current().0.agents.keep_awake
    }

    /// Drops the settings of the project `id` (9.28): the settings then in use, or `None` when
    /// it had none (nothing is written). Under one lock, so a save meanwhile is not undone.
    pub fn forget(&self, id: &str) -> Result<Option<Settings>, String> {
        let mut current = self.current();
        let mut settings = current.0.clone();
        if settings.projects.remove(id).is_none() {
            return Ok(None);
        }
        self.save_in(&mut current, settings).map(Some)
    }

    /// The scripts of the project `id` (none when it has no settings).
    pub fn scripts(&self, id: &str) -> ProjectScripts {
        let current = self.current();
        let project = current.0.projects.get(id);
        project.map(|p| p.scripts.clone()).unwrap_or_default()
    }
}

/// The terminal font default before 7.11. A file still holding it gets today's default: it
/// was saved with the old default, not chosen.
const OLD_FONT_DEFAULT: &str = "\"IBM Plex Mono\", monospace";

/// The settings in `file`, and its `claude.account` when it has one (the account selected
/// before 22.5; `Some(None)`: the default one, or a folder no account has).
fn read(file: &Path) -> io::Result<(Settings, Option<Option<String>>)> {
    let bytes = read_limited(&mut std::fs::File::open(file)?, FILE_LIMIT)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let selected = value
        .get("claude")
        .and_then(|c| c.get("account"))
        .map(|a| a.as_str().map(|dir| spaces::trimmed(dir).to_owned()));
    let mut settings: Settings = serde_json::from_value(value).map_err(io::Error::other)?;
    tidy(&mut settings);
    check(&settings).map_err(io::Error::other)?;
    // Compared as both are kept (22.2): `/x/` and `/x` are one folder.
    let accounts = &settings.claude.accounts;
    let selected =
        selected.map(|dir| dir.filter(|dir| accounts.iter().any(|a| a.config_dir == *dir)));
    settle(&mut settings);
    if settings.terminal.font_family == OLD_FONT_DEFAULT {
        settings.terminal.font_family = Settings::default().terminal.font_family;
    }
    Ok((settings, selected))
}

fn save(file: &Path, settings: &Settings) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(settings)?;
    if json.len() as u64 > FILE_LIMIT {
        return Err(io::Error::other(format!(
            "the settings are larger than {FILE_LIMIT} bytes"
        )));
    }
    file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
    write_atomic(file, &json, 0o600)
}

/// An empty default account name is [`DEFAULT_ACCOUNT`] (21.4).
/// Each account's folder without trailing separators ([`spaces::trimmed`], 22.2), as a space
/// names it: before [`check`], which would take `/x/` for a blank or padded folder.
fn tidy(settings: &mut Settings) {
    for account in &mut settings.claude.accounts {
        account.config_dir = spaces::trimmed(&account.config_dir).to_owned();
    }
    for dir in &mut settings.gh.config_dirs {
        *dir = spaces::trimmed(dir).to_owned();
    }
}

fn settle(settings: &mut Settings) {
    let claude = &mut settings.claude;
    if claude.default_name.is_empty() {
        claude.default_name = DEFAULT_ACCOUNT.to_owned();
    }
}

/// Marks each Claude account whose folder Hive made (22.3): [`own`] in `accounts`.
fn mark(settings: &mut Settings, accounts: &Path) {
    for account in &mut settings.claude.accounts {
        account.hive_folder = own(accounts, &account.config_dir);
    }
}

/// Whether `dir` is a folder Hive makes for an account (22.3): right in `accounts`, named as
/// [`slug`] names one (so never `..`), and spelled exactly as Hive spells it: another spelling
/// of the same folder (`work/`, `//work`, `/./work`) is not, so it never deletes the folder of
/// the account spelled the usual way.
fn own(accounts: &Path, dir: &str) -> bool {
    let name = Path::new(dir).file_name().and_then(|n| n.to_str());
    name.is_some_and(|n| slug(n) == n && accounts.join(n).as_os_str() == OsStr::new(dir))
}

/// The folder name of an account named `name` (22.3): its letters and digits, lowercase, any
/// run of other characters one `-` between them (`Work 2` is `work-2`); empty when it has none.
fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// `name`, or `name 2`, `name 3`… when an account, or the default one, has it.
fn free_name(claude: &ClaudeSettings, name: &str) -> String {
    let accounts = &claude.accounts;
    let taken = |n: &str| n == default_name(claude) || accounts.iter().any(|a| a.name == n);
    // One of these is free: there are more of them than accounts.
    let names = (1..=accounts.len() + 2).map(|n| match n {
        1 => name.to_owned(),
        n => format!("{name} {n}"),
    });
    names.into_iter().find(|n| !taken(n)).unwrap_or_default()
}

/// The default account's name (21.4): [`DEFAULT_ACCOUNT`] while it is empty.
fn default_name(claude: &ClaudeSettings) -> &str {
    match claude.default_name.as_str() {
        "" => DEFAULT_ACCOUNT,
        name => name,
    }
}

/// The Claude accounts (12.2): each name as a space's (`spaces::name_of`), unique and not the
/// default account's (its own name checked the same way, 21.4, or empty); each folder as a
/// space's was (absolute, printable), unique. Neither with spaces around it.
fn accounts(claude: &ClaudeSettings) -> Result<(), String> {
    let accounts = &claude.accounts;
    let default = &claude.default_name;
    if !default.is_empty() && spaces::name_of("the default account", default)? != *default {
        return Err(format!(
            "The default account's name {default:?} has spaces around it"
        ));
    }
    if accounts.len() > ACCOUNT_LIMIT {
        return Err(format!("At most {ACCOUNT_LIMIT} accounts can be added"));
    }
    for (i, account) in accounts.iter().enumerate() {
        let (name, dir) = (&account.name, &account.config_dir);
        if spaces::name_of("the account", name)? != *name {
            return Err(format!("The account name {name:?} has spaces around it"));
        }
        if spaces::claude_config_dir(Some(dir.clone()), false)?.as_ref() != Some(dir) {
            return Err(format!(
                "The Claude config folder of {name} is blank or has spaces around it"
            ));
        }
        let before = &accounts[..i];
        if name == default_name(claude) || before.iter().any(|a| a.name == *name) {
            return Err(format!("Another account is named {name}"));
        }
        if before.iter().any(|a| a.config_dir == *dir) {
            return Err(format!(
                "Another account has the Claude config folder {dir}"
            ));
        }
    }
    Ok(())
}

/// The gh config folders (24.13): at most [`ACCOUNT_LIMIT`], each checked as a space's
/// account's folder (absolute, printable), without spaces around it, and unique.
fn gh_folders(gh: &GhSettings) -> Result<(), String> {
    let dirs = &gh.config_dirs;
    if dirs.len() > ACCOUNT_LIMIT {
        return Err(format!(
            "At most {ACCOUNT_LIMIT} gh config folders can be added"
        ));
    }
    for (i, dir) in dirs.iter().enumerate() {
        if spaces::gh_config_dir(Some(dir.clone()), false)?.as_ref() != Some(dir) {
            return Err(format!(
                "The gh config folder {dir:?} is blank or has spaces around it"
            ));
        }
        if dirs[..i].contains(dir) {
            return Err(format!("The gh config folder {dir} is there twice"));
        }
    }
    Ok(())
}

/// The remote hosts (26.17): at most [`HOST_LIMIT`] aliases `ssh` may be given
/// ([`check_host`]), each once.
fn hosts(hosts: &[String]) -> Result<(), String> {
    if hosts.len() > HOST_LIMIT {
        return Err(format!("At most {HOST_LIMIT} hosts can be added"));
    }
    for (i, host) in hosts.iter().enumerate() {
        check_host(host)?;
        if hosts[..i].contains(host) {
            return Err(format!("The host {host} is there twice"));
        }
    }
    Ok(())
}

/// Every value within its range, with a message naming the first that is not.
pub fn check(settings: &Settings) -> Result<(), String> {
    accounts(&settings.claude)?;
    gh_folders(&settings.gh)?;
    hosts(&settings.remote.hosts)?;
    range(
        "remote.keep_minutes",
        settings.remote.keep_minutes,
        1,
        10_080,
    )?;
    let terminal = &settings.terminal;
    range("terminal.font_size", terminal.font_size, 8, 32)?;
    range("terminal.scrollback", terminal.scrollback, 1000, 100_000)?;
    range(
        "notifications.volume",
        settings.notifications.volume,
        0,
        100,
    )?;
    range(
        "notifications.command_secs",
        settings.notifications.command_secs,
        0,
        3600,
    )?;
    range("agents.silence_secs", settings.agents.silence_secs, 2, 60)?;
    text("terminal.font_family", &terminal.font_family)?;
    if let Some(base) = &settings.worktrees.default_base {
        text("worktrees.default_base", base)?;
    }
    for (id, project) in &settings.projects {
        let scripts = &project.scripts;
        let name = |what: &str| format!("projects.{id}.scripts.{what}");
        let optional = [("setup", &scripts.setup), ("archive", &scripts.archive)];
        for (what, value) in optional {
            value
                .as_deref()
                .map_or(Ok(()), |v| script(&name(what), v))?;
        }
        for (i, run) in scripts.run.iter().enumerate() {
            text(&name("run.name"), &run.name)?;
            script(&name("run.command"), &run.command)?;
            if scripts.run[..i].iter().any(|r| r.name == run.name) {
                let run = &run.name;
                return Err(format!("{} {run:?} is used twice", name("run.name")));
            }
        }
    }
    Ok(())
}

/// Not blank, at most [`SCRIPT_LIMIT`] bytes, no control characters but newlines and tabs.
fn script(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{name} must not be empty"))
    } else if value.len() > SCRIPT_LIMIT {
        Err(format!("{name} must be at most {SCRIPT_LIMIT} bytes"))
    } else if value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        Err(format!("{name} must not hold control characters"))
    } else {
        Ok(())
    }
}

fn range(name: &str, value: u32, min: u32, max: u32) -> Result<(), String> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{name} must be between {min} and {max} (got {value})"
        ))
    }
}

/// Not blank, at most [`TEXT_LIMIT`] bytes, no control characters.
fn text(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{name} must not be empty"))
    } else if value.len() > TEXT_LIMIT {
        Err(format!("{name} must be at most {TEXT_LIMIT} characters"))
    } else if value.chars().any(char::is_control) {
        Err(format!("{name} must not hold control characters"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_protocol::ProjectSettings;

    fn store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::load(tmp.path().join("hive/settings.json"));
        (tmp, store)
    }

    #[test]
    fn a_missing_file_is_the_defaults_without_a_warning() {
        let (_tmp, store) = store();
        assert_eq!(store.get(), (Settings::default(), None));
        assert_eq!(store.silence(), Duration::from_secs(5));
        assert!(store.keep_awake());
    }

    #[test]
    fn saved_settings_are_used_and_read_back_private() {
        let (tmp, store) = store();
        let mut settings = Settings::default();
        settings.agents.silence_secs = 12;
        settings.agents.keep_awake = false;
        settings.worktrees.default_base = Some("dev".into());
        assert_eq!(store.set(settings.clone()), Ok(settings.clone()));
        assert_eq!(store.get(), (settings.clone(), None));
        assert_eq!(store.silence(), Duration::from_secs(12));
        assert!(!store.keep_awake());
        let file = tmp.path().join("hive/settings.json");
        #[cfg(unix)]
        assert_eq!(
            crate::mode::of(&std::fs::metadata(&file).unwrap()) & 0o777,
            0o600
        );
        assert_eq!(Store::load(file).get(), (settings, None));
    }

    #[test]
    fn a_removed_project_takes_its_settings_along() {
        let (tmp, store) = store();
        let mut settings = with_scripts(ProjectScripts {
            setup: Some("make".into()),
            ..Default::default()
        });
        store.set(settings.clone()).unwrap();
        let file = tmp.path().join("hive/settings.json");
        // One without settings writes nothing.
        std::fs::remove_file(&file).unwrap();
        assert_eq!(store.forget("/other"), Ok(None));
        assert!(!file.exists());
        settings.projects.clear();
        assert_eq!(store.forget("/r"), Ok(Some(settings.clone())));
        assert_eq!(store.scripts("/r"), ProjectScripts::default());
        assert_eq!(Store::load(file).get(), (settings, None));
    }

    #[test]
    fn a_partial_file_keeps_the_defaults_for_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        std::fs::write(&file, r#"{"notifications":{"volume":0},"other":true}"#).unwrap();
        let mut expected = Settings::default();
        expected.notifications.volume = 0;
        assert_eq!(Store::load(file.clone()).get(), (expected, None));
        // A project allowed for the chats Hive once had (7.3) keeps its other settings.
        let old = r#"{"projects":{"/r":{"scripts":{"setup":"make"},"chat_confirmed":true}}}"#;
        std::fs::write(&file, old).unwrap();
        let setup = ProjectScripts {
            setup: Some("make".into()),
            ..Default::default()
        };
        assert_eq!(Store::load(file).get(), (with_scripts(setup), None));
    }

    #[test]
    fn the_old_default_font_becomes_the_new_default_and_a_chosen_one_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        std::fs::write(
            &file,
            r#"{"terminal":{"font_family":"\"IBM Plex Mono\", monospace","font_size":15}}"#,
        )
        .unwrap();
        let settings = Store::load(file.clone()).get().0;
        assert_eq!(
            settings.terminal.font_family,
            "\"Hive Mono\", \"Symbols Nerd Font\", monospace"
        );
        assert_eq!(settings.terminal.font_size, 15);
        std::fs::write(&file, r#"{"terminal":{"font_family":"\"IBM Plex Mono\""}}"#).unwrap();
        let settings = Store::load(file).get().0;
        assert_eq!(settings.terminal.font_family, "\"IBM Plex Mono\"");
    }

    #[test]
    fn an_invalid_file_is_the_defaults_with_a_warning_and_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        for (contents, why) in [
            ("{", "EOF while parsing"),
            (
                r#"{"terminal":{"font_size":99}}"#,
                "terminal.font_size must be between 8 and 32 (got 99)",
            ),
            (
                r#"{"terminal":{"shell":"zsh"}}"#,
                "unknown variant `zsh`, expected one of `default`, `cmd`, `git_bash`",
            ),
            (
                &" ".repeat(FILE_LIMIT as usize + 1),
                "input larger than 262144 bytes",
            ),
        ] {
            std::fs::write(&file, contents).unwrap();
            let store = Store::load(file.clone());
            let (settings, warning) = store.get();
            assert_eq!(settings, Settings::default());
            let warning = warning.unwrap();
            assert!(warning.starts_with("Ignoring "), "{warning}");
            assert!(warning.contains(why), "{warning}");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), contents);
            // Saving replaces the file and ends the warning.
            store.set(Settings::default()).unwrap();
            assert_eq!(store.get().1, None);
        }
    }

    #[test]
    fn ensuring_the_file_writes_it_only_when_missing() {
        let (tmp, store) = store();
        let file = tmp.path().join("hive/settings.json");
        assert_eq!(store.file(), file);
        assert_eq!(store.ensure_file().unwrap(), file);
        assert_eq!(Store::load(file.clone()).get(), (Settings::default(), None));
        std::fs::write(&file, "{").unwrap();
        assert_eq!(store.ensure_file().unwrap(), file);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{");
        // No directory for it: the error is returned.
        let blocked = tempfile::tempdir().unwrap();
        std::fs::write(blocked.path().join("hive"), "").unwrap();
        let store = Store::load(blocked.path().join("hive/settings.json"));
        assert!(store.ensure_file().is_err());
    }

    #[test]
    fn out_of_range_settings_are_refused_and_nothing_changes() {
        let (tmp, store) = store();
        let mut settings = Settings::default();
        settings.notifications.volume = 101;
        assert_eq!(
            store.set(settings),
            Err("notifications.volume must be between 0 and 100 (got 101)".into())
        );
        assert_eq!(store.get(), (Settings::default(), None));
        assert!(!tmp.path().join("hive").exists());
    }

    #[test]
    fn settings_too_large_for_the_file_are_refused() {
        let (_tmp, store) = store();
        let mut settings = Settings::default();
        for i in 0..2000 {
            let path = format!("/{i:0>200}");
            settings.projects.insert(path, ProjectSettings::default());
        }
        let err = store.set(settings).unwrap_err();
        assert!(
            err.ends_with("the settings are larger than 262144 bytes"),
            "{err}"
        );
        assert_eq!(store.get().0, Settings::default());
    }

    #[test]
    fn settings_exactly_as_large_as_the_file_limit_are_saved() {
        let (_tmp, store) = store();
        let sized = |key: String| {
            let mut settings = Settings::default();
            settings.projects.insert(key, ProjectSettings::default());
            settings
        };
        let base = serde_json::to_vec_pretty(&sized("/".into())).unwrap().len();
        let key = format!("/{}", "a".repeat(FILE_LIMIT as usize - base));
        let settings = sized(key);
        assert_eq!(
            serde_json::to_vec_pretty(&settings).unwrap().len() as u64,
            FILE_LIMIT
        );
        assert_eq!(store.set(settings.clone()), Ok(settings));
    }

    #[test]
    fn a_failed_write_is_reported_and_nothing_changes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("hive"), "").unwrap();
        let store = Store::load(tmp.path().join("hive/settings.json"));
        let mut settings = Settings::default();
        settings.agents.confirm_close = false;
        let err = store.set(settings).unwrap_err();
        assert!(err.starts_with("Cannot save "), "{err}");
        assert_eq!(store.get().0, Settings::default());
    }

    #[test]
    fn every_range_holds_at_its_ends_only() {
        type Field = fn(&mut Settings) -> &mut u32;
        let fields: [(Field, u32, u32); 6] = [
            (|s| &mut s.terminal.font_size, 8, 32),
            (|s| &mut s.terminal.scrollback, 1000, 100_000),
            (|s| &mut s.notifications.volume, 0, 100),
            (|s| &mut s.notifications.command_secs, 0, 3600),
            (|s| &mut s.agents.silence_secs, 2, 60),
            (|s| &mut s.remote.keep_minutes, 1, 10_080),
        ];
        for (field, min, max) in fields {
            let mut settings = Settings::default();
            for (value, ok) in [(min, true), (max, true), (max + 1, false)] {
                *field(&mut settings) = value;
                assert_eq!(check(&settings).is_ok(), ok, "{value}");
            }
            if min > 0 {
                *field(&mut settings) = min - 1;
                assert!(check(&settings).is_err(), "{min}");
            }
        }
    }

    #[test]
    fn hosts_are_valid_aliases_each_once_and_at_most_the_limit() {
        let with = |hosts: &[&str]| {
            let mut settings = Settings::default();
            settings.remote.hosts = hosts.iter().map(|h| (*h).to_owned()).collect();
            check(&settings)
        };
        assert_eq!(with(&["gpu-box", "user@lab.example"]), Ok(()));
        assert_eq!(
            with(&["gpu-box", "gpu-box"]),
            Err("The host gpu-box is there twice".into())
        );
        assert_eq!(
            with(&["-oProxyCommand=x"]),
            Err("The host alias \"-oProxyCommand=x\" must not start with -".into())
        );
        let many: Vec<String> = (0..=HOST_LIMIT).map(|i| format!("h{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(with(&many[..HOST_LIMIT]), Ok(()));
        assert_eq!(with(&many), Err("At most 32 hosts can be added".into()));
    }

    #[test]
    fn texts_must_be_short_printable_and_not_blank() {
        let font = |family: &str| {
            let mut settings = Settings::default();
            settings.terminal.font_family = family.into();
            check(&settings)
        };
        assert_eq!(font(&"a".repeat(TEXT_LIMIT)), Ok(()));
        assert_eq!(
            font(&"a".repeat(TEXT_LIMIT + 1)),
            Err("terminal.font_family must be at most 256 characters".into())
        );
        assert_eq!(
            font(" "),
            Err("terminal.font_family must not be empty".into())
        );
        assert_eq!(
            font("a\nb"),
            Err("terminal.font_family must not hold control characters".into())
        );
        let mut settings = Settings::default();
        settings.worktrees.default_base = Some(String::new());
        assert_eq!(
            check(&settings),
            Err("worktrees.default_base must not be empty".into())
        );
    }

    fn with_scripts(scripts: ProjectScripts) -> Settings {
        let mut settings = Settings::default();
        let project = ProjectSettings { scripts };
        settings.projects.insert("/r".into(), project);
        settings
    }

    #[test]
    fn scripts_are_checked_and_read_by_project() {
        let run = |name: &str, command: &str| hive_protocol::RunScript {
            name: name.into(),
            command: command.into(),
        };
        let scripts = ProjectScripts {
            setup: Some("bun install\n\tmake".into()),
            run: vec![run("dev", "bun dev")],
            archive: Some("a".repeat(SCRIPT_LIMIT)),
        };
        let (_tmp, store) = store();
        store.set(with_scripts(scripts.clone())).unwrap();
        assert_eq!(store.scripts("/r"), scripts);
        assert_eq!(store.scripts("/other"), ProjectScripts::default());
        let refused = |scripts: ProjectScripts| check(&with_scripts(scripts)).unwrap_err();
        let setup = |text: &str| ProjectScripts {
            setup: Some(text.into()),
            ..Default::default()
        };
        assert_eq!(
            refused(setup(" \n")),
            "projects./r.scripts.setup must not be empty"
        );
        assert_eq!(
            refused(setup("a\u{1b}[2J")),
            "projects./r.scripts.setup must not hold control characters"
        );
        assert_eq!(
            refused(ProjectScripts {
                archive: Some("a".repeat(SCRIPT_LIMIT + 1)),
                ..Default::default()
            }),
            "projects./r.scripts.archive must be at most 16384 bytes"
        );
        let runs = |r| ProjectScripts {
            run: vec![r],
            ..Default::default()
        };
        assert_eq!(
            refused(runs(run("a\nb", "x"))),
            "projects./r.scripts.run.name must not hold control characters"
        );
        assert_eq!(
            refused(runs(run("dev", ""))),
            "projects./r.scripts.run.command must not be empty"
        );
        let twice = ProjectScripts {
            run: vec![run("dev", "a"), run("test", "b"), run("dev", "c")],
            ..Default::default()
        };
        assert_eq!(
            refused(twice),
            "projects./r.scripts.run.name \"dev\" is used twice"
        );
    }

    fn account(name: &str, dir: &str) -> Account {
        Account {
            name: name.into(),
            config_dir: dir.into(),
            hive_folder: false,
        }
    }

    #[test]
    fn an_account_folder_with_a_trailing_separator_is_kept_without_it() {
        let (tmp, store) = store();
        let work = tmp.path().join("work").display().to_string();
        std::fs::create_dir(&work).unwrap();
        let clicked = format!("{work}{}", std::path::MAIN_SEPARATOR);
        let mut settings = Settings::default();
        settings.claude.accounts.push(account("Work", &clicked));
        let saved = store.set(settings).unwrap();
        assert_eq!(saved.claude.accounts[0].config_dir, work);
        // A file written before (22.2) reads the same way.
        let file = tmp.path().join("old.json");
        let mut old = saved.clone();
        old.claude.accounts[0].config_dir = clicked;
        std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(read(&file).unwrap().0, saved);
    }

    #[test]
    fn accounts_are_saved_read_back_and_chosen_for_new_terminals() {
        let (tmp, store) = store();
        let work = tmp.path().join("work").display().to_string();
        std::fs::create_dir(&work).unwrap();
        let mut settings = Settings::default();
        settings.claude.accounts.push(account("Work", &work));
        assert_eq!(store.set(settings.clone()), Ok(settings.clone()));
        assert_eq!(store.account_dirs(), std::slice::from_ref(&work));
        let file = tmp.path().join("hive/settings.json");
        assert_eq!(Store::load(file.clone()).get(), (settings.clone(), None));
        // A terminal gets its space's account (22.5), the default one for a folder no account
        // has, or the one asked for when it is an account.
        let asked = |dir: Option<&str>| {
            let config_dir = dir.map(Into::into);
            store.claude_dir(Some(AccountDir { config_dir }), Some(work.clone()))
        };
        assert_eq!(
            store.claude_dir(None, Some(work.clone())),
            Ok(Some(work.clone()))
        );
        assert_eq!(store.claude_dir(None, Some("/other".into())), Ok(None));
        assert_eq!(store.claude_dir(None, None), Ok(None));
        assert_eq!(store.known(Some(work.clone())), Some(work.clone()));
        assert_eq!(asked(None), Ok(None));
        assert_eq!(asked(Some(&work)), Ok(Some(work.clone())));
        assert_eq!(
            asked(Some("/other")),
            Err("No Claude account has the folder /other".into())
        );
        // Each terminal's account is shown by name (21.7).
        assert_eq!(store.account_name(Some(&work)), "Work");
        assert_eq!(store.account_name(None), DEFAULT_ACCOUNT);
        assert_eq!(store.account_name(Some("/other")), DEFAULT_ACCOUNT);

        // A known folder may be gone since; a new one must exist.
        std::fs::remove_dir(&work).unwrap();
        settings.agents.silence_secs = 9;
        assert_eq!(store.set(settings.clone()), Ok(settings.clone()));
        let missing = tmp.path().join("new").display().to_string();
        let mut added = settings.clone();
        added.claude.accounts.push(account("New", &missing));
        assert_eq!(
            store.set(added),
            Err(format!(
                "The Claude config folder: {missing} is not a folder"
            ))
        );
        // An account a space uses cannot be removed (22.5), naming the space; nothing changes.
        let mut removed = settings.clone();
        removed.claude.accounts.clear();
        let used = |dir: &str| (dir == work).then(|| "Work".to_owned());
        assert_eq!(
            store.set_with(removed.clone(), &used, &|_| None),
            Err(
                "Work is the Claude account of the space Work: choose another one for it first"
                    .into()
            )
        );
        assert_eq!(store.get().0, settings);
        // Kept, or used by no space: saved.
        assert_eq!(
            store.set_with(settings.clone(), &used, &|_| None),
            Ok(settings)
        );
        assert_eq!(
            store.set_with(removed.clone(), &|_| None, &|_| None),
            Ok(removed)
        );
        assert_eq!(store.account_dirs(), Vec::<String>::new());
    }

    #[test]
    fn the_account_selected_before_spaces_had_one_goes_to_them_and_leaves_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        // An absolute folder on this platform (`/w` is not one on Windows).
        let w = if cfg!(windows) { r"C:\w" } else { "/w" };
        let accounts = serde_json::json!([{"name": "Work", "config_dir": w}]);
        let old = serde_json::json!({"claude": {"accounts": accounts, "account": w}}).to_string();
        let old = old.as_str();
        std::fs::write(&file, old).unwrap();
        // The old file still loads, as before.
        let store = Store::load(file.clone());
        let mut expected = Settings::default();
        expected.claude.accounts.push(account("Work", w));
        assert_eq!(store.get(), (expected.clone(), None));
        // The spaces refuse it: the file is kept, and the next start tries again.
        let given = std::cell::RefCell::new(Vec::new());
        let mut refuse = |dir: &str| {
            given.borrow_mut().push(dir.to_owned());
            Err("no".to_owned())
        };
        store.migrate_account(&mut refuse);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), old);
        let mut take = |dir: &str| {
            given.borrow_mut().push(dir.to_owned());
            Ok(())
        };
        store.migrate_account(&mut take);
        assert_eq!(*given.borrow(), [w, w]);
        let saved = std::fs::read_to_string(&file).unwrap();
        assert!(!saved.contains("\"account\""), "{saved}");
        assert_eq!(Store::load(file.clone()).get(), (expected, None));
        // Once only.
        store.migrate_account(&mut take);
        assert_eq!(given.borrow().len(), 2);
        // The default account, or a folder no account has, is no space's: only the file changes.
        for old in [
            r#"{"claude":{"account":null}}"#,
            r#"{"claude":{"account":"/gone"}}"#,
        ] {
            std::fs::write(&file, old).unwrap();
            let store = Store::load(file.clone());
            assert_eq!(store.get(), (Settings::default(), None));
            store.migrate_account(&mut take);
            assert_eq!(given.borrow().len(), 2, "{old}");
            let saved = std::fs::read_to_string(&file).unwrap();
            assert!(!saved.contains("\"account\""), "{saved}");
        }
        // A file without it is not written.
        std::fs::write(&file, "{}").unwrap();
        Store::load(file.clone()).migrate_account(&mut take);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}");
    }

    // `/x` is no absolute path on Windows.
    #[cfg(unix)]
    #[test]
    fn old_files_with_a_trailing_separator_still_give_a_space_its_account() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        let accounts = serde_json::json!([{"name": "Work", "config_dir": "/x/"}]);
        let old = serde_json::json!({"claude": {"accounts": accounts}});
        std::fs::write(&file, old.to_string()).unwrap();
        let store = Store::load(file);
        // An old `spaces.json` as read (`projects.rs`): parsed, then checked.
        let env = r#"{"git_name":null,"git_email":null,"gh_config_dir":null,"gh_account":null,"claude_account":"/x/"}"#;
        let json = format!(
            r#"{{"current":"w","spaces":[{{"id":"w","name":"W","projects":[],"groups":[],"env":{env}}}]}}"#
        );
        let spaces = serde_json::from_str::<spaces::Spaces>(&json)
            .unwrap()
            .check()
            .unwrap();
        let account = spaces.spaces[0].env.claude_account.clone();
        assert_eq!(account.as_deref(), Some("/x"));
        // Its terminals get the account's folder as `CLAUDE_CONFIG_DIR`.
        assert_eq!(store.claude_dir(None, account), Ok(Some("/x".into())));
        // And the account cannot be removed while the space uses it (22.5).
        let mut removed = store.get().0;
        removed.claude.accounts.clear();
        let using = |dir: &str| spaces.using(dir).map(|s| s.name.clone());
        assert_eq!(
            store.set_with(removed, &using, &|_| None),
            Err(
                "Work is the Claude account of the space W: choose another one for it first".into()
            )
        );
    }

    #[test]
    fn a_selected_account_with_a_trailing_separator_goes_to_the_spaces_without_it() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        let w = if cfg!(windows) { r"C:\w" } else { "/w" };
        let slash = format!("{w}{}", std::path::MAIN_SEPARATOR);
        // Either side may have it (22.2): the account's folder, the selected one, or both.
        for (folder, selected) in [
            (&slash, w),
            (&w.to_owned(), slash.as_str()),
            (&slash, &slash),
        ] {
            let accounts = serde_json::json!([{"name": "Work", "config_dir": folder}]);
            let old = serde_json::json!({"claude": {"accounts": accounts, "account": selected}});
            std::fs::write(&file, old.to_string()).unwrap();
            let given = std::cell::RefCell::new(Vec::new());
            Store::load(file.clone()).migrate_account(&mut |dir: &str| {
                given.borrow_mut().push(dir.to_owned());
                Ok(())
            });
            assert_eq!(*given.borrow(), [w], "{folder} {selected}");
        }
    }

    // `/a` is no absolute path on Windows.
    #[cfg(unix)]
    #[test]
    fn account_names_and_folders_are_checked() {
        let refused = |accounts: &[Account]| {
            let mut settings = Settings::default();
            settings.claude.accounts = accounts.to_vec();
            check(&settings).unwrap_err()
        };
        let long = "x".repeat(spaces::NAME_LIMIT + 1);
        let cases = [
            (vec![account(" ", "/a")], "Enter a name for the account"),
            (
                vec![account(&long, "/a")],
                "The name is longer than 64 characters",
            ),
            (
                vec![account(" Work", "/a")],
                "The account name \" Work\" has spaces around it",
            ),
            (
                vec![account("W", "a")],
                "The Claude config folder must be an absolute path",
            ),
            (
                vec![account("W", " ")],
                "The Claude config folder of W is blank or has spaces around it",
            ),
            (
                vec![account("W", "/a ")],
                "The Claude config folder of W is blank or has spaces around it",
            ),
            (
                vec![account("Default", "/a")],
                "Another account is named Default",
            ),
            (
                vec![account("W", "/a"), account("W", "/b")],
                "Another account is named W",
            ),
            (
                vec![account("W", "/a"), account("V", "/a")],
                "Another account has the Claude config folder /a",
            ),
        ];
        for (accounts, message) in cases {
            assert_eq!(refused(&accounts), message);
        }
        let mut fine = Settings::default();
        fine.claude.accounts = vec![account("W", "/a"), account("V", "/b")];
        assert_eq!(check(&fine), Ok(()));
        // At most ACCOUNT_LIMIT accounts, counted before anything else is checked.
        let many = |n: usize| (0..n).map(|i| account(&format!("A{i}"), &format!("/{i}")));
        fine.claude.accounts = many(ACCOUNT_LIMIT).collect();
        assert_eq!(check(&fine), Ok(()));
        let mut too_many: Vec<Account> = many(ACCOUNT_LIMIT + 1).collect();
        too_many[0].name = " ".into();
        assert_eq!(refused(&too_many), "At most 64 accounts can be added");
    }

    #[test]
    fn a_terminal_names_the_default_account_by_its_configured_name() {
        let (tmp, store) = store();
        let file = tmp.path().join("hive/settings.json");
        std::fs::create_dir(tmp.path().join("hive")).unwrap();
        // Renamed (21.4): a terminal without an account, or of a folder no account has.
        let mut settings = Settings::default();
        settings.claude.default_name = "Personal".into();
        store.set(settings.clone()).unwrap();
        assert_eq!(store.account_name(None), "Personal");
        assert_eq!(store.account_name(Some("/other")), "Personal");
        // Emptied, or missing from the file: "Default".
        settings.claude.default_name = String::new();
        store.set(settings).unwrap();
        assert_eq!(store.account_name(None), "Default");
        std::fs::write(&file, r#"{"claude":{"accounts":[]}}"#).unwrap();
        assert_eq!(Store::load(file).account_name(None), "Default");
    }

    // `/a` is no absolute path on Windows.
    #[cfg(unix)]
    #[test]
    fn the_default_account_is_renamed_and_no_other_account_takes_its_name() {
        let (tmp, store) = store();
        let file = tmp.path().join("hive/settings.json");
        std::fs::create_dir(tmp.path().join("hive")).unwrap();
        // A file from before 21.4, without the name, and one with it empty: "Default".
        for old in [
            r#"{"claude":{"accounts":[]}}"#,
            r#"{"claude":{"default_name":""}}"#,
        ] {
            std::fs::write(&file, old).unwrap();
            let read = Store::load(file.clone()).get();
            assert_eq!(read.0.claude.default_name, "Default", "{old}");
            assert_eq!(read.1, None);
        }
        // Renamed, saved and read back.
        let mut settings = Settings::default();
        settings.claude.default_name = "Personal".into();
        assert_eq!(store.set(settings.clone()), Ok(settings.clone()));
        let read = Store::load(file.clone()).get().0;
        assert_eq!(read.claude.default_name, "Personal");
        // Emptied, it is "Default" again.
        let mut emptied = settings.clone();
        emptied.claude.default_name = String::new();
        assert_eq!(store.set(emptied).unwrap().claude.default_name, "Default");
        assert_eq!(store.get().0.claude.default_name, "Default");

        // Its current name is taken: "Default" is free once it is renamed.
        let with = |default: &str, names: &[&str]| {
            let mut settings = Settings::default();
            settings.claude.default_name = default.into();
            let dirs = names.iter().enumerate();
            let accounts = dirs.map(|(i, n)| account(n, &format!("/{i}")));
            settings.claude.accounts = accounts.collect();
            check(&settings)
        };
        let taken = |name: &str| Err(format!("Another account is named {name}"));
        assert_eq!(with("Personal", &["Personal"]), taken("Personal"));
        assert_eq!(with("", &["Default"]), taken("Default"));
        assert_eq!(with("Personal", &["Default", "Work"]), Ok(()));
        // Its name follows an account name's rules.
        let long = "x".repeat(spaces::NAME_LIMIT + 1);
        let cases = [
            (" ", "Enter a name for the default account".to_owned()),
            (&long, "The name is longer than 64 characters".to_owned()),
            ("A\u{7}", "The name has control characters".to_owned()),
            (
                " Personal",
                "The default account's name \" Personal\" has spaces around it".to_owned(),
            ),
        ];
        for (name, message) in cases {
            assert_eq!(with(name, &[]), Err(message), "{name:?}");
        }

        // A space's old folder named as the default account gets a number.
        let mut renamed = Settings::default();
        renamed.claude.default_name = "Personal".into();
        store.set(renamed).unwrap();
        let legacy = vec![("Personal".to_owned(), "/p".to_owned())];
        assert_eq!(store.migrate(legacy, vec![]), Ok(()));
        assert_eq!(store.get().0.claude.accounts, [account("Personal 2", "/p")]);
    }

    // `/a` is no absolute path on Windows.
    #[cfg(unix)]
    #[test]
    fn the_spaces_claude_folders_become_accounts_once() {
        let (tmp, store) = store();
        let legacy = |list: &[(&str, &str)]| -> Vec<(String, String)> {
            list.iter()
                .map(|(n, d)| (n.to_string(), d.to_string()))
                .collect()
        };
        // Nothing to make: nothing is written.
        assert_eq!(store.migrate(vec![], vec![]), Ok(()));
        assert!(!tmp.path().join("hive").exists());
        let spaces = [
            ("Work", "/w"),
            ("Default", "/d"),
            // A trailing separator is the same folder (22.2), kept without it.
            ("Also work", "/w/"),
            ("Work", "/x"),
            ("Default", "/y/"),
        ];
        assert_eq!(store.migrate(legacy(&spaces), vec![]), Ok(()));
        let made = [
            account("Work", "/w"),
            account("Default 2", "/d"),
            account("Work 2", "/x"),
            account("Default 3", "/y"),
        ];
        assert_eq!(store.get().0.claude.accounts, made);
        // Idempotent: a folder already an account is not made again.
        assert_eq!(store.migrate(legacy(&spaces), vec![]), Ok(()));
        let file = tmp.path().join("hive/settings.json");
        assert_eq!(Store::load(file.clone()).get().0.claude.accounts, made);
        // A name made too long by its number is refused, and nothing changes.
        let long = "x".repeat(spaces::NAME_LIMIT);
        let err = store.migrate(legacy(&[(&long, "/l"), (&long, "/m")]), vec![]);
        assert_eq!(err, Err("The name is longer than 64 characters".into()));
        assert_eq!(store.get().0.claude.accounts, made);
        // As many names are tried as it takes: one more than the accounts and the default one.
        let fresh = Store::load(tmp.path().join("fresh/settings.json"));
        let numbered = legacy(&[("Default 2", "/a"), ("Default", "/b")]);
        assert_eq!(fresh.migrate(numbered, vec![]), Ok(()));
        let names: Vec<String> = fresh
            .get()
            .0
            .claude
            .accounts
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, ["Default 2", "Default 3"]);
        // An ignored settings file is not replaced.
        std::fs::write(&file, "{").unwrap();
        let ignored = Store::load(file.clone());
        let err = ignored.migrate(legacy(&[("Z", "/z")]), vec![]).unwrap_err();
        assert!(err.starts_with("Ignoring "), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{");
    }

    #[test]
    fn gh_config_folders_are_checked_kept_and_not_removed_while_a_space_uses_one() {
        let (tmp, store) = store();
        let dir = tmp.path().join("gh-work");
        std::fs::create_dir(&dir).unwrap();
        let dir = dir.display().to_string();
        let with = |dirs: &[&str]| {
            let mut settings = Settings::default();
            settings.gh.config_dirs = dirs.iter().map(|d| d.to_string()).collect();
            settings
        };
        // Kept without a trailing separator (22.2), read back.
        let saved = store.set(with(&[&format!("{dir}/")])).unwrap();
        assert_eq!(saved, with(&[&dir]));
        let file = tmp.path().join("hive/settings.json");
        assert_eq!(Store::load(file).get().0, saved);
        // A new one must exist, absolute, once.
        let missing = tmp.path().join("missing").display().to_string();
        let refused = [
            (
                with(&[&dir, &missing]),
                format!("The GitHub CLI config folder: {missing} is not a folder"),
            ),
            (
                with(&[&dir, "rel"]),
                "The GitHub CLI config folder must be an absolute path".into(),
            ),
            (
                with(&[&dir, " "]),
                "The gh config folder \" \" is blank or has spaces around it".into(),
            ),
            (
                with(&[&dir, &dir]),
                format!("The gh config folder {dir} is there twice"),
            ),
        ];
        for (settings, message) in refused {
            assert_eq!(store.set(settings), Err(message));
        }
        let many: Vec<String> = (0..=ACCOUNT_LIMIT).map(|i| format!("{dir}{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(
            store.set(with(&many)),
            Err("At most 64 gh config folders can be added".into())
        );
        assert!(check(&with(&many[..ACCOUNT_LIMIT])).is_ok());
        // One a space's account is in cannot be removed (24.13), naming the space.
        let used = |d: &str| (d == dir).then(|| "Work".to_owned());
        assert_eq!(
            store.set_with(with(&[]), &|_| None, &used),
            Err(format!(
                "The GitHub account of the space Work is in {dir}: choose another one for it first"
            ))
        );
        assert_eq!(store.get().0, saved);
        assert_eq!(store.set_with(saved.clone(), &|_| None, &used), Ok(saved));
        assert_eq!(
            store.set_with(with(&[]), &|_| None, &|_| None),
            Ok(with(&[]))
        );
    }

    // `/g` is no absolute path on Windows.
    #[cfg(unix)]
    #[test]
    fn the_spaces_gh_folders_join_the_settings_once() {
        let (tmp, _) = store();
        let mut settings = Settings::default();
        settings.gh.config_dirs = vec!["/g".into()];
        let file = tmp.path().join("hive/settings.json");
        save(&file, &settings).unwrap();
        let store = Store::load(file.clone());
        let gh = vec!["/g".to_owned(), "/w".to_owned()];
        assert_eq!(store.migrate(vec![], gh.clone()), Ok(()));
        assert_eq!(store.get().0.gh.config_dirs, gh);
        assert_eq!(Store::load(file).get().0.gh.config_dirs, gh);
        // Already there: nothing changes.
        assert_eq!(store.migrate(vec![], vec!["/w".into()]), Ok(()));
        assert_eq!(store.get().0.gh.config_dirs, gh);
    }

    /// A store in a temporary folder whose account folders go in `accounts/` there.
    #[cfg(unix)]
    fn store_with_accounts() -> (tempfile::TempDir, Store, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let accounts = tmp.path().join("hive/accounts");
        let file = tmp.path().join("hive/settings.json");
        let store = Store::load(file).with_accounts(accounts.clone());
        (tmp, store, accounts)
    }

    #[test]
    fn an_account_folder_is_named_after_the_account() {
        assert_eq!(slug("Work"), "work");
        assert_eq!(slug("  Work 2 -- Client!  "), "work-2-client");
        assert_eq!(slug("Ação"), "ação");
        assert_eq!(slug("../.."), "");
        assert_eq!(slug("/"), "");
        let accounts = Path::new("/h/accounts");
        // Spelled as Hive joins it (with `\` on Windows).
        let made = accounts.join("work-2");
        assert!(own(accounts, made.to_str().unwrap()));
        assert!(!own(accounts, "/h/accounts/Work"));
        assert!(!own(accounts, "/h/accounts/.."));
        assert!(!own(accounts, "/h/accounts/work/inner"));
        assert!(!own(accounts, "/h/accounts"));
        assert!(!own(accounts, "/h/.claude-work"));
        // Only as Hive spells it.
        for alias in [
            "/h/accounts/work/",
            "/h/accounts//work",
            "/h/accounts/./work",
            "/h//accounts/work",
        ] {
            assert!(!own(accounts, alias), "{alias}");
        }
    }

    // Unix: the folder's mode, and absolute folders as `/`.
    #[cfg(unix)]
    #[test]
    fn a_new_account_gets_a_private_folder_of_its_own_and_keeps_it_marked() {
        let (tmp, store, accounts) = store_with_accounts();
        let settings = store.create_account("Work 2").unwrap();
        let dir = accounts.join("work-2");
        let made = Account {
            name: "Work 2".into(),
            config_dir: dir.display().to_string(),
            hive_folder: true,
        };
        assert_eq!(settings.claude.accounts, std::slice::from_ref(&made));
        assert_eq!(store.get().0, settings);
        let mode = |dir: &Path| crate::mode::of(&std::fs::metadata(dir).unwrap()) & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&accounts), 0o700);
        // Read back: marked by the service, whatever the file or the app says.
        let file = tmp.path().join("hive/settings.json");
        let saved = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, saved.replace("true", "false")).unwrap();
        let again = Store::load(file.clone()).with_accounts(accounts.clone());
        assert_eq!(again.get().0.claude.accounts, std::slice::from_ref(&made));
        assert!(!Store::load(file).get().0.claude.accounts[0].hive_folder);
        let mut unmarked = settings.clone();
        unmarked.claude.accounts[0].hive_folder = false;
        assert_eq!(again.set(unmarked), Ok(settings));
    }

    #[cfg(unix)]
    #[test]
    fn a_taken_or_unsafe_account_name_makes_no_folder() {
        let (tmp, store, accounts) = store_with_accounts();
        let before = store.create_account("Work").unwrap();
        // The name is taken, or the folder another account's.
        assert_eq!(
            store.create_account("Work"),
            Err("Another account is named Work".into())
        );
        let dir = accounts.join("work").display().to_string();
        assert_eq!(
            store.create_account("work!"),
            Err(format!(
                "Another account has the Claude config folder {dir}"
            ))
        );
        // A folder there already, though no account has it.
        std::fs::create_dir(accounts.join("home")).unwrap();
        let home = accounts.join("home").display().to_string();
        assert_eq!(
            store.create_account("Home"),
            Err(format!(
                "{home} exists already: give the account another name"
            ))
        );
        // No letter or digit to name a folder after.
        for name in ["../..", "/", "--"] {
            assert_eq!(
                store.create_account(name),
                Err(format!(
                    "{name:?} cannot name a folder: give the account a letter or a digit"
                ))
            );
        }
        assert_eq!(store.get().0, before);
        let mut left: Vec<_> = std::fs::read_dir(&accounts)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        left.sort();
        assert_eq!(left, ["home", "work"]);
        // A folder that cannot be made, or settings that cannot be saved, leave nothing.
        std::fs::write(tmp.path().join("file"), "").unwrap();
        let file = tmp.path().join("hive/b.json");
        let blocked = Store::load(file).with_accounts(tmp.path().join("file/accounts"));
        let err = blocked.create_account("Work").unwrap_err();
        assert!(err.starts_with("Cannot make "), "{err}");
        let file = tmp.path().join("file/settings.json");
        let unsaved = Store::load(file).with_accounts(accounts.clone());
        let err = unsaved.create_account("Other").unwrap_err();
        assert!(err.starts_with("Cannot save "), "{err}");
        assert!(!accounts.join("other").exists());
    }

    #[cfg(unix)]
    #[test]
    fn another_spelling_of_a_hive_folder_never_deletes_it() {
        let (_tmp, store, accounts) = store_with_accounts();
        store.create_account("Work").unwrap();
        let work = accounts.join("work");
        std::fs::write(work.join(".claude.json"), "{}").unwrap();
        let work = work.display().to_string();
        // A trailing separator is the same spelling (22.2): that folder is taken.
        let mut settings = store.get().0;
        settings
            .claude
            .accounts
            .push(account("Slash", &format!("{work}/")));
        assert_eq!(
            store.set(settings),
            Err(format!(
                "Another account has the Claude config folder {work}"
            ))
        );
        let double = format!("{}//work", accounts.display());
        let mut settings = store.get().0;
        settings.claude.accounts.push(account("Double", &double));
        let saved = store.set(settings).unwrap();
        // Not marked as Hive's, so never deleted: refused, nothing removed.
        let marks: Vec<_> = saved
            .claude
            .accounts
            .iter()
            .map(|a| a.hive_folder)
            .collect();
        assert_eq!(marks, [true, false]);
        assert_eq!(
            store.remove_account(&double, true, &|_| None),
            Err(format!("Hive did not make {double}: it is never deleted"))
        );
        // The folder Hive spells is not deleted either while another spelling has it.
        assert_eq!(
            store.remove_account(&work, true, &|_| None),
            Err(format!(
                "Double has the folder {work} too: it is not deleted"
            ))
        );
        assert_eq!(store.account_dirs().len(), 2);
        // The alias goes without its folder; the real one and what it holds stay.
        let (_, why) = store.remove_account(&double, false, &|_| None).unwrap();
        assert_eq!(why, None);
        assert!(Path::new(&work).join(".claude.json").is_file());
        assert_eq!(store.account_dirs(), [work]);
    }

    #[cfg(unix)]
    #[test]
    fn removing_an_account_deletes_only_a_folder_hive_made() {
        let (tmp, store, accounts) = store_with_accounts();
        store.create_account("Work").unwrap();
        store.create_account("Kept").unwrap();
        let work = accounts.join("work");
        std::fs::write(work.join(".claude.json"), "{}").unwrap();
        let theirs = tmp.path().join(".claude-mine");
        std::fs::create_dir(&theirs).unwrap();
        let theirs = theirs.display().to_string();
        let mut settings = store.get().0;
        settings.claude.accounts.push(account("Mine", &theirs));
        store.set(settings).unwrap();
        // A folder Hive did not make is never deleted: refused, nothing removed.
        assert_eq!(
            store.remove_account(&theirs, true, &|_| None),
            Err(format!("Hive did not make {theirs}: it is never deleted"))
        );
        assert_eq!(store.account_dirs().len(), 3);
        // Removed and kept on disk, as before.
        let (settings, why) = store.remove_account(&theirs, false, &|_| None).unwrap();
        assert_eq!((settings.claude.accounts.len(), why), (2, None));
        assert!(Path::new(&theirs).is_dir());
        // Used by a space (22.5): refused before its folder is touched.
        let work = work.display().to_string();
        let used = |d: &str| (d == work).then(|| "Job".to_owned());
        assert_eq!(
            store.remove_account(&work, true, &used),
            Err(
                "Work is the Claude account of the space Job: choose another one for it first"
                    .into()
            )
        );
        assert_eq!(store.account_dirs().len(), 2);
        assert!(Path::new(&work).join(".claude.json").is_file());
        // Hive's own: deleted with what it holds.
        let (settings, why) = store.remove_account(&work, true, &|_| None).unwrap();
        assert_eq!((settings.claude.accounts.len(), why), (1, None));
        assert!(!accounts.join("work").exists());
        assert_eq!(
            store.remove_account(&work, false, &|_| None),
            Err(format!("No Claude account has the folder {work}"))
        );
        // Gone already: removed, and the reason said.
        let kept = accounts.join("kept").display().to_string();
        std::fs::remove_dir(&kept).unwrap();
        let (settings, why) = store.remove_account(&kept, true, &|_| None).unwrap();
        assert!(settings.claude.accounts.is_empty());
        assert!(why.unwrap().starts_with(&format!("Cannot delete {kept}: ")));
    }
}
