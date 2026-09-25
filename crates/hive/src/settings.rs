//! The user's settings, owned by the service (#37) and kept in
//! `$XDG_CONFIG_HOME/hive/settings.json`. The app reads and saves them whole through the
//! protocol (`get_settings`, `set_settings`); ranges are checked here.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use hive_protocol::{ProjectScripts, Settings};

use crate::git::read_limited;
use crate::wrapper::write_atomic;

/// Largest settings file read or written.
const FILE_LIMIT: u64 = 256 * 1024;
/// Longest font family, branch or script name kept.
const TEXT_LIMIT: usize = 256;
/// Longest script kept.
const SCRIPT_LIMIT: usize = 16 * 1024;

pub struct Store {
    file: PathBuf,
    /// The settings in use, and why the file was ignored (until the settings are saved).
    current: Mutex<(Settings, Option<String>)>,
}

impl Store {
    /// Loads `file`. A missing file is the defaults; an unreadable or invalid one is the
    /// defaults too, with a warning for the app, and stays untouched until the user saves.
    pub fn load(file: PathBuf) -> Self {
        let current = match read(&file) {
            Ok(settings) => (settings, None),
            Err(err) if err.kind() == io::ErrorKind::NotFound => (Settings::default(), None),
            Err(err) => {
                let warning = format!(
                    "Ignoring {}: {err}. Using the defaults until the settings are saved.",
                    file.display()
                );
                eprintln!("hive: warning: {warning}");
                (Settings::default(), Some(warning))
            }
        };
        Self {
            file,
            current: Mutex::new(current),
        }
    }

    fn current(&self) -> MutexGuard<'_, (Settings, Option<String>)> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The settings in use, and why the file was ignored, if it was.
    pub fn get(&self) -> (Settings, Option<String>) {
        self.current().clone()
    }

    /// Checks and saves `settings`, which are then in use. Nothing changes on a failure.
    pub fn set(&self, settings: Settings) -> Result<Settings, String> {
        check(&settings)?;
        let mut current = self.current();
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

    /// The scripts of the project `id` (none when it has no settings).
    pub fn scripts(&self, id: &str) -> ProjectScripts {
        let current = self.current();
        let project = current.0.projects.get(id);
        project.map(|p| p.scripts.clone()).unwrap_or_default()
    }
}

fn read(file: &Path) -> io::Result<Settings> {
    let bytes = read_limited(&mut std::fs::File::open(file)?, FILE_LIMIT)?;
    let settings = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    check(&settings).map_err(io::Error::other)?;
    Ok(settings)
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

/// Every value within its range, with a message naming the first that is not.
pub fn check(settings: &Settings) -> Result<(), String> {
    let terminal = &settings.terminal;
    range("terminal.font_size", terminal.font_size, 8, 32)?;
    range("terminal.scrollback", terminal.scrollback, 1000, 100_000)?;
    range(
        "notifications.volume",
        settings.notifications.volume,
        0,
        100,
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
        for run in &scripts.run {
            text(&name("run.name"), &run.name)?;
            script(&name("run.command"), &run.command)?;
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
    use std::os::unix::fs::PermissionsExt;

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
    }

    #[test]
    fn saved_settings_are_used_and_read_back_private() {
        let (tmp, store) = store();
        let mut settings = Settings::default();
        settings.agents.silence_secs = 12;
        settings.worktrees.default_base = Some("dev".into());
        assert_eq!(store.set(settings.clone()), Ok(settings.clone()));
        assert_eq!(store.get(), (settings.clone(), None));
        assert_eq!(store.silence(), Duration::from_secs(12));
        let file = tmp.path().join("hive/settings.json");
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(Store::load(file).get(), (settings, None));
    }

    #[test]
    fn a_partial_file_keeps_the_defaults_for_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        std::fs::write(&file, r#"{"notifications":{"volume":0},"other":true}"#).unwrap();
        let mut expected = Settings::default();
        expected.notifications.volume = 0;
        assert_eq!(Store::load(file).get(), (expected, None));
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
        let fields: [(Field, u32, u32); 4] = [
            (|s| &mut s.terminal.font_size, 8, 32),
            (|s| &mut s.terminal.scrollback, 1000, 100_000),
            (|s| &mut s.notifications.volume, 0, 100),
            (|s| &mut s.agents.silence_secs, 2, 60),
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
    }
}
