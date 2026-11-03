//! Whether a Claude account has logged in (21.6): its Claude Code config file has an
//! `oauthAccount`. Only that key's presence is read, never its value, and never the
//! credentials Claude Code keeps elsewhere (the macOS Keychain, `.credentials.json`, #45).

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};

use hive_protocol::ClaudeLogin;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::git::read_limited;

/// Largest config file read: Claude Code keeps every project's state in it, so it grows; a
/// larger one reads as not logged in.
pub const FILE_LIMIT: u64 = 16 * 1024 * 1024;

/// The keys of the config file read: the account's details are skipped, never kept.
#[derive(Deserialize)]
struct Config {
    #[serde(rename = "oauthAccount")]
    oauth_account: Option<IgnoredAny>,
}

/// Claude Code's global config file for the account whose folder is `dir` (`None`: the
/// default account, which runs in the service's own environment, read through `var`), found
/// as Claude Code finds it: `.config.json` in the config folder (`dir`, else
/// `$CLAUDE_CONFIG_DIR`, else `$HOME/.claude`) when that legacy file exists, else
/// `.claude.json` in `dir`, else in `$CLAUDE_CONFIG_DIR`, else in `$HOME` (`~/.claude.json`,
/// beside the folder rather than in it). `None` without any of them.
pub fn config_file(dir: Option<&str>, var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |key: &str| match (key, dir) {
        ("CLAUDE_CONFIG_DIR", Some(dir)) => Some(OsString::from(dir)),
        _ => var(key).filter(|v| !v.is_empty()),
    };
    let config_home = crate::sessions::claude_dir(var)?;
    let legacy = config_home.join(".config.json");
    if legacy.exists() {
        return Some(legacy);
    }
    let base = var("CLAUDE_CONFIG_DIR").or_else(|| var(crate::dirs::HOME))?;
    Some(PathBuf::from(base).join(".claude.json"))
}

/// Whether the config file `file` has a top-level `oauthAccount` (not `null`). A missing,
/// unreadable, oversized, invalid or not regular file is not logged in.
pub fn logged_in(file: &Path) -> bool {
    read(file).is_some_and(|config| config.oauth_account.is_some())
}

fn read(file: &Path) -> Option<Config> {
    // Only a regular file: a FIFO would block the read. The folder is the user's own.
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let bytes = read_limited(&mut File::open(file).ok()?, FILE_LIMIT).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The `claude_logins` answer: the default account, then each of `dirs` (the settings'
/// accounts' folders).
pub fn answer(dirs: &[String], var: impl Fn(&str) -> Option<OsString>) -> Vec<ClaudeLogin> {
    let accounts = [None]
        .into_iter()
        .chain(dirs.iter().map(|d| Some(d.as_str())));
    let login = |dir: Option<&str>| ClaudeLogin {
        config_dir: dir.map(String::from),
        logged_in: config_file(dir, &var).is_some_and(|file| logged_in(&file)),
    };
    accounts.map(login).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a Path)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        |key| {
            let found = vars.iter().find(|(k, _)| *k == key);
            found.map(|(_, v)| v.as_os_str().to_owned())
        }
    }

    #[test]
    fn the_config_file_is_found_as_claude_code_finds_it() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let named = home.join(".claude-work");
        let named_dir = named.to_str().unwrap();
        let other = home.join("other");
        let plain = [(crate::dirs::HOME, home)];
        // The default account: `~/.claude.json`, beside `~/.claude`.
        let file = config_file(None, env(&plain));
        assert_eq!(file, Some(home.join(".claude.json")));
        // A named account: in its folder, whatever the service's own environment says.
        let set = [(crate::dirs::HOME, home), ("CLAUDE_CONFIG_DIR", &other)];
        let file = config_file(Some(named_dir), env(&set));
        assert_eq!(file, Some(named.join(".claude.json")));
        // The default account of a service run with `CLAUDE_CONFIG_DIR`: in that folder.
        assert_eq!(
            config_file(None, env(&set)),
            Some(other.join(".claude.json"))
        );
        let empty = [
            (crate::dirs::HOME, home),
            ("CLAUDE_CONFIG_DIR", Path::new("")),
        ];
        let file = config_file(None, env(&empty));
        assert_eq!(file, Some(home.join(".claude.json")));
        // No home and no folder: none.
        assert_eq!(config_file(None, env(&[])), None);
        assert_eq!(
            config_file(None, env(&[(crate::dirs::HOME, Path::new(""))])),
            None
        );
        // The legacy `.config.json` in the config folder wins when it exists.
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(home.join(".claude/.config.json"), "{}").unwrap();
        let file = config_file(None, env(&plain));
        assert_eq!(file, Some(home.join(".claude/.config.json")));
        std::fs::create_dir_all(&named).unwrap();
        std::fs::write(named.join(".config.json"), "{}").unwrap();
        let file = config_file(Some(named_dir), env(&plain));
        assert_eq!(file, Some(named.join(".config.json")));
    }

    #[test]
    fn an_account_is_logged_in_only_with_an_oauth_account() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".claude.json");
        let with = r#"{"numStartups":3,"oauthAccount":{"emailAddress":"me@example.com"}}"#;
        std::fs::write(&file, with).unwrap();
        assert!(logged_in(&file));
        for without in [
            r#"{"numStartups":3}"#,
            r#"{"oauthAccount":null}"#,
            r#"{"projects":{"/r":{"oauthAccount":{}}}}"#,
            "not json",
            "[]",
            "",
        ] {
            std::fs::write(&file, without).unwrap();
            assert!(!logged_in(&file), "{without}");
        }
        assert!(!logged_in(&dir.path().join("missing.json")));
        assert!(!logged_in(dir.path()));
    }

    #[test]
    fn a_config_file_over_the_limit_reads_not_logged_in() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".claude.json");
        let head = r#"{"oauthAccount":{},"pad":""#;
        let fit = |len: u64| {
            let pad = "x".repeat(len as usize - head.len() - 2);
            std::fs::write(&file, format!("{head}{pad}\"}}")).unwrap();
            assert_eq!(std::fs::metadata(&file).unwrap().len(), len);
        };
        // 16 MiB, written out: the limit itself is under test.
        fit(16 * 1024 * 1024);
        assert!(logged_in(&file));
        fit(16 * 1024 * 1024 + 1);
        assert!(!logged_in(&file));
    }

    #[test]
    fn the_answer_has_the_default_account_then_each_account() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let work = home.join("work");
        let gone = home.join("gone");
        std::fs::create_dir(&work).unwrap();
        std::fs::write(home.join(".claude.json"), r#"{"oauthAccount":{}}"#).unwrap();
        std::fs::write(work.join(".claude.json"), r#"{"numStartups":1}"#).unwrap();
        let dirs = [work, gone].map(|d| d.to_str().unwrap().to_owned());
        let logins = answer(&dirs, env(&[(crate::dirs::HOME, home)]));
        let login = |config_dir: Option<&String>, logged_in| ClaudeLogin {
            config_dir: config_dir.cloned(),
            logged_in,
        };
        let expected = [login(None, true), login(Some(&dirs[0]), false)];
        let expected = [&expected[..], &[login(Some(&dirs[1]), false)]].concat();
        assert_eq!(logins, expected);
        // Without a home, the default account is not logged in.
        assert!(!answer(&[], env(&[]))[0].logged_in);
    }
}
