//! Spaces (6.14): groups of followed projects, e.g. work and personal, each with an optional
//! identity for the terminals opened in its projects. `hive::projects` keeps them in
//! `<data>/hive/spaces.json`; this module holds the rules.

use std::path::Path;

use hive_protocol::{Space, SpaceEnv};
use serde::{Deserialize, Serialize};

/// The space the flat project list of earlier versions moves into.
pub const DEFAULT_ID: &str = "default";
/// Longest space name, in characters.
const NAME_LIMIT: usize = 64;
/// Longest git name or email, in bytes.
const TEXT_LIMIT: usize = 256;
/// Longest folder path, in bytes.
const PATH_LIMIT: usize = 4096;

/// Every space, in the order they were created, and the current one's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spaces {
    pub current: String,
    pub spaces: Vec<Space>,
}

impl Spaces {
    /// One "Default" space holding `projects` (the list kept before spaces existed).
    pub fn with(projects: Vec<String>) -> Self {
        let space = Space {
            id: DEFAULT_ID.to_owned(),
            name: "Default".to_owned(),
            projects,
            env: SpaceEnv::default(),
        };
        Self {
            current: DEFAULT_ID.to_owned(),
            spaces: vec![space],
        }
    }

    /// The spaces as read from the file, when they follow the rules: at least one, unique
    /// ids, valid names and environments (folders are not required to exist still), each
    /// project in one space only, and a current space that exists.
    pub fn check(self) -> Result<Self, String> {
        let mut ids = std::collections::HashSet::new();
        let mut projects = std::collections::HashSet::new();
        for space in &self.spaces {
            check_name(&space.name)?;
            check_env(space.env.clone(), false)?;
            if !ids.insert(&space.id) {
                return Err(format!("the space id {:?} is used twice", space.id));
            }
            if let Some(twice) = space.projects.iter().find(|p| !projects.insert(*p)) {
                return Err(format!("{twice} is in two spaces"));
            }
        }
        if !ids.contains(&self.current) {
            return Err(format!("no space {:?} to be the current one", self.current));
        }
        Ok(self)
    }

    /// Every project of every space.
    pub fn projects(&self) -> impl Iterator<Item = &String> {
        self.spaces.iter().flat_map(|s| &s.projects)
    }

    /// The space holding the project `id`.
    pub fn of(&self, project: &str) -> Option<&Space> {
        self.spaces
            .iter()
            .find(|s| s.projects.iter().any(|p| p == project))
    }

    /// The current space's projects and environment.
    pub fn current(&self) -> (Vec<String>, SpaceEnv) {
        let space = self.spaces.iter().find(|s| s.id == self.current);
        space.map_or_else(Default::default, |s| (s.projects.clone(), s.env.clone()))
    }

    /// Adds the project `id` to the current space.
    pub fn add(&mut self, project: String) {
        let current = &self.current;
        if let Some(space) = self.spaces.iter_mut().find(|s| s.id == *current) {
            space.projects.push(project);
        }
    }

    /// A new, empty space, made the current one.
    pub fn create(&mut self, name: &str, env: SpaceEnv) -> Result<(), String> {
        let (name, env) = (check_name(name)?, check_env(env, true)?);
        // One of these is free: there are more of them than spaces.
        let id = (1..=self.spaces.len() + 1)
            .map(|n| format!("space-{n}"))
            .find(|id| self.spaces.iter().all(|s| s.id != *id))
            .unwrap_or_default();
        self.current = id.clone();
        let projects = Vec::new();
        self.spaces.push(Space {
            id,
            name,
            projects,
            env,
        });
        Ok(())
    }

    /// Renames the space `id` and replaces its environment.
    pub fn update(&mut self, id: &str, name: &str, env: SpaceEnv) -> Result<(), String> {
        let (name, env) = (check_name(name)?, check_env(env, true)?);
        let space = self.spaces.iter_mut().find(|s| s.id == id);
        let space = space.ok_or_else(|| unknown(id))?;
        space.name = name;
        space.env = env;
        Ok(())
    }

    /// Removes the space `id` when it has no projects and is not the last one.
    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        let space = self
            .spaces
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| unknown(id))?;
        if !space.projects.is_empty() {
            let name = &space.name;
            return Err(format!(
                "{name} has projects: only an empty space can be deleted"
            ));
        }
        if self.spaces.len() == 1 {
            return Err("the last space cannot be deleted".to_owned());
        }
        self.spaces.retain(|s| s.id != id);
        if self.current == id {
            self.current = self
                .spaces
                .iter()
                .map(|s| s.id.clone())
                .next()
                .unwrap_or_default();
        }
        Ok(())
    }

    /// Makes `id` the current space.
    pub fn select(&mut self, id: &str) -> Result<(), String> {
        if self.spaces.iter().all(|s| s.id != id) {
            return Err(unknown(id));
        }
        self.current = id.to_owned();
        Ok(())
    }
}

fn unknown(id: &str) -> String {
    format!("no space {id:?}")
}

/// The name, trimmed: not blank, at most [`NAME_LIMIT`] characters, no control characters.
fn check_name(name: &str) -> Result<String, String> {
    let name = text("The name", Some(name.to_owned()), usize::MAX)?;
    let name = name.ok_or_else(|| "Enter a name for the space".to_owned())?;
    if name.chars().count() > NAME_LIMIT {
        return Err(format!("The name is longer than {NAME_LIMIT} characters"));
    }
    Ok(name)
}

/// `value` trimmed, `None` when blank; refused when longer than `limit` bytes or holding a
/// control character (it goes into a terminal's environment).
fn text(field: &str, value: Option<String>, limit: usize) -> Result<Option<String>, String> {
    let value = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    match value {
        Some(v) if v.len() > limit => Err(format!("{field} is longer than {limit} bytes")),
        Some(v) if v.chars().any(char::is_control) => {
            Err(format!("{field} has control characters"))
        }
        value => Ok(value),
    }
}

/// A folder: like [`text`], and absolute; an existing directory too when `on_disk`.
fn dir(field: &str, value: Option<String>, on_disk: bool) -> Result<Option<String>, String> {
    let value = text(field, value, PATH_LIMIT)?;
    if let Some(dir) = &value {
        if !Path::new(dir).is_absolute() {
            return Err(format!("{field} must be an absolute path"));
        }
        if on_disk && !Path::new(dir).is_dir() {
            return Err(format!("{field}: {dir} is not a folder"));
        }
    }
    Ok(value)
}

/// The environment as kept: blank values unset, the rest checked (see [`text`], [`dir`]).
fn check_env(env: SpaceEnv, on_disk: bool) -> Result<SpaceEnv, String> {
    Ok(SpaceEnv {
        claude_config_dir: dir("The Claude config folder", env.claude_config_dir, on_disk)?,
        git_name: text("The git name", env.git_name, TEXT_LIMIT)?,
        git_email: text("The git email", env.git_email, TEXT_LIMIT)?,
        gh_config_dir: dir("The GitHub CLI config folder", env.gh_config_dir, on_disk)?,
    })
}

/// The environment entries a terminal of a space gets, as separate values (never a shell
/// command).
pub fn vars(env: &SpaceEnv) -> Vec<(&'static str, String)> {
    let vars = [
        ("CLAUDE_CONFIG_DIR", &env.claude_config_dir),
        ("GIT_AUTHOR_NAME", &env.git_name),
        ("GIT_COMMITTER_NAME", &env.git_name),
        ("GIT_AUTHOR_EMAIL", &env.git_email),
        ("GIT_COMMITTER_EMAIL", &env.git_email),
        ("GH_CONFIG_DIR", &env.gh_config_dir),
    ];
    let set = vars
        .into_iter()
        .map(|(key, value)| Some((key, value.clone()?)));
    set.flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(dir: Option<&str>, name: Option<&str>) -> SpaceEnv {
        SpaceEnv {
            claude_config_dir: dir.map(Into::into),
            git_name: name.map(Into::into),
            git_email: None,
            gh_config_dir: None,
        }
    }

    #[test]
    fn spaces_are_created_selected_renamed_and_deleted() {
        let mut spaces = Spaces::with(vec!["/a".into()]);
        spaces.create(" Work ", SpaceEnv::default()).unwrap();
        assert_eq!(spaces.current, "space-1");
        assert_eq!(spaces.spaces[1].name, "Work");
        spaces.add("/b".into());
        assert_eq!(
            spaces.current(),
            (vec!["/b".to_owned()], SpaceEnv::default())
        );
        assert_eq!(spaces.of("/a").unwrap().id, "default");
        assert_eq!(spaces.of("/c"), None);
        let all: Vec<&String> = spaces.projects().collect();
        assert_eq!(all, ["/a", "/b"]);

        // Ids are never reused while taken.
        spaces.create("Other", SpaceEnv::default()).unwrap();
        spaces.select("default").unwrap();
        spaces.delete("space-1").unwrap_err();
        spaces.spaces[1].projects.clear();
        spaces.delete("space-1").unwrap();
        spaces.create("Third", SpaceEnv::default()).unwrap();
        let ids: Vec<&str> = spaces.spaces.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["default", "space-2", "space-1"]);

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        spaces
            .update("space-2", "Renamed", env(Some(dir), Some(" Me ")))
            .unwrap();
        let renamed = &spaces.spaces[1];
        assert_eq!(renamed.name, "Renamed");
        assert_eq!(renamed.env, env(Some(dir), Some("Me")));
        assert_eq!(
            spaces.update("nope", "x", SpaceEnv::default()),
            Err(unknown("nope"))
        );

        // Deleting the current space makes the first one current.
        assert_eq!(spaces.current, "space-1");
        spaces.delete("space-1").unwrap();
        assert_eq!(spaces.current, "default");
        spaces.select("space-2").unwrap();
        spaces.delete("default").unwrap_err();
        assert_eq!(spaces.select("nope"), Err(unknown("nope")));
        assert_eq!(spaces.delete("nope"), Err(unknown("nope")));
        spaces.spaces[0].projects.clear();
        spaces.delete("default").unwrap();
        assert_eq!(spaces.current, "space-2");
        assert_eq!(
            spaces.delete("space-2"),
            Err("the last space cannot be deleted".to_owned())
        );
    }

    #[test]
    fn a_space_with_projects_is_kept() {
        let mut spaces = Spaces::with(vec!["/a".into()]);
        spaces.create("Other", SpaceEnv::default()).unwrap();
        assert_eq!(
            spaces.delete("default"),
            Err("Default has projects: only an empty space can be deleted".to_owned())
        );
        assert_eq!(spaces.spaces.len(), 2);
    }

    #[test]
    fn names_and_environments_are_checked() {
        let mut spaces = Spaces::with(vec![]);
        let create =
            |spaces: &mut Spaces, name: &str, env: SpaceEnv| spaces.create(name, env).unwrap_err();
        assert_eq!(
            create(&mut spaces, "  ", SpaceEnv::default()),
            "Enter a name for the space"
        );
        assert_eq!(
            create(&mut spaces, &"x".repeat(65), SpaceEnv::default()),
            "The name is longer than 64 characters"
        );
        assert!(spaces.create(&"é".repeat(64), SpaceEnv::default()).is_ok());
        assert_eq!(
            create(&mut spaces, "a\u{7}b", SpaceEnv::default()),
            "The name has control characters"
        );
        let long = "x".repeat(257);
        assert_eq!(
            create(&mut spaces, "n", env(None, Some(&long))),
            "The git name is longer than 256 bytes"
        );
        assert!(spaces.create("n", env(None, Some(&long[1..]))).is_ok());
        let email = SpaceEnv {
            git_email: Some("a@b\n".into()),
            ..SpaceEnv::default()
        };
        // Trimmed first: a trailing newline is only whitespace.
        assert!(spaces.create("n", email).is_ok());
        let email = SpaceEnv {
            git_email: Some("a\nb".into()),
            ..SpaceEnv::default()
        };
        assert_eq!(
            create(&mut spaces, "n", email),
            "The git email has control characters"
        );
        assert_eq!(
            create(&mut spaces, "n", env(Some("rel/dir"), None)),
            "The Claude config folder must be an absolute path"
        );
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing").display().to_string();
        assert_eq!(
            create(&mut spaces, "n", env(Some(&missing), None)),
            format!("The Claude config folder: {missing} is not a folder")
        );
        let gh = SpaceEnv {
            gh_config_dir: Some("/x".repeat(2049)),
            ..SpaceEnv::default()
        };
        assert_eq!(
            create(&mut spaces, "n", gh),
            "The GitHub CLI config folder is longer than 4096 bytes"
        );
        // Blank values are unset.
        spaces.create("blank", env(Some(" "), Some(""))).unwrap();
        assert_eq!(spaces.current().1, SpaceEnv::default());
    }

    #[test]
    fn a_file_is_checked_but_its_folders_may_be_gone() {
        let space = |id: &str, projects: &[&str]| Space {
            id: id.into(),
            name: id.into(),
            projects: projects.iter().map(|p| p.to_string()).collect(),
            env: env(Some("/gone"), None),
        };
        let spaces = |current: &str, list: Vec<Space>| Spaces {
            current: current.into(),
            spaces: list,
        };
        let good = spaces("a", vec![space("a", &["/r"]), space("b", &["/s"])]);
        assert_eq!(good.clone().check(), Ok(good));
        let bad = [
            (spaces("a", vec![]), r#"no space "a" to be the current one"#),
            (
                spaces("c", vec![space("a", &[])]),
                r#"no space "c" to be the current one"#,
            ),
            (
                spaces("a", vec![space("a", &[]), space("a", &[])]),
                r#"the space id "a" is used twice"#,
            ),
            (
                spaces("a", vec![space("a", &["/r"]), space("b", &["/r"])]),
                "/r is in two spaces",
            ),
            (
                spaces("a", vec![space("a", &["/r", "/r"])]),
                "/r is in two spaces",
            ),
            (
                spaces("", vec![space("", &[])]),
                "Enter a name for the space",
            ),
        ];
        for (spaces, message) in bad {
            assert_eq!(spaces.check(), Err(message.to_owned()));
        }
        let mut relative = spaces("a", vec![space("a", &[])]);
        relative.spaces[0].env = env(Some("gone"), None);
        assert!(relative.check().is_err());
    }

    #[test]
    fn a_space_sets_only_what_it_has() {
        assert_eq!(vars(&SpaceEnv::default()), []);
        let full = SpaceEnv {
            claude_config_dir: Some("/c".into()),
            git_name: Some("Me".into()),
            git_email: Some("me@x".into()),
            gh_config_dir: Some("/g".into()),
        };
        let expected = [
            ("CLAUDE_CONFIG_DIR", "/c"),
            ("GIT_AUTHOR_NAME", "Me"),
            ("GIT_COMMITTER_NAME", "Me"),
            ("GIT_AUTHOR_EMAIL", "me@x"),
            ("GIT_COMMITTER_EMAIL", "me@x"),
            ("GH_CONFIG_DIR", "/g"),
        ]
        .map(|(k, v)| (k, v.to_owned()));
        assert_eq!(vars(&full), expected);
        assert_eq!(
            vars(&env(None, Some("Me")))[1],
            ("GIT_COMMITTER_NAME", "Me".into())
        );
    }
}
