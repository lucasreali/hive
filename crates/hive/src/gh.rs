//! The GitHub CLI (9.30): the accounts logged in to `gh`, a space's account and its token, and
//! the one way Hive runs `gh` ([`Gh::run`]). Always the `gh` executable with separate
//! arguments, stdin closed, output size-limited and within [`TIME`]. A token only ever goes
//! into the environment of `gh` and of a space's terminals: never into a log, a message to the
//! app or a file.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use hive_protocol::{GhAccount, GhLogin, SpaceEnv};

use crate::git::{self, Stdout};

/// How long one `gh` command may take (most of them call GitHub).
pub const TIME: Duration = Duration::from_secs(30);
/// How long `gh auth token` and `gh auth switch` may take: they only read and write `gh`'s
/// config, and a terminal waits for the token.
const LOCAL_TIME: Duration = Duration::from_secs(5);
/// Most bytes read from `gh auth status`, `gh auth token` or `gh auth switch`.
const AUTH_OUTPUT: u64 = 64 * 1024;
/// Most accounts listed.
const ACCOUNTS_LIMIT: usize = 32;
/// Longest GitHub login (GitHub's own limit).
const LOGIN_LIMIT: usize = 39;
/// Longest host name.
const HOST_LIMIT: usize = 253;
/// Longest token taken from `gh auth token`.
const TOKEN_LIMIT: usize = 1024;
/// Why the account list is empty although `gh` ran.
pub const NOT_LOGGED_IN: &str = "No account is logged in to gh (run gh auth login)";
/// Why nothing about `gh` is known.
pub const NOT_INSTALLED: &str = "gh (the GitHub CLI) is not installed";

/// Variables of the service's own environment that would pick `gh`'s account, host, config
/// or repository: Hive sets its own or none.
const CLEARED: [&str; 7] = [
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "GH_HOST",
    "GH_CONFIG_DIR",
    "GH_REPO",
];

/// Where Hive's `gh` is.
pub struct Gh {
    /// `gh`, or a fake one in tests.
    pub program: OsString,
    /// The user's `PATH` (`wrapper::user_path`), where `gh` is looked for: a service started
    /// through `wsl.exe`, or a macOS app, lacks what the user's shell config adds.
    pub path: OsString,
}

impl Gh {
    /// `gh` on the user's `PATH`.
    pub fn on(path: OsString) -> Self {
        let program = "gh".into();
        Self { program, path }
    }

    /// `gh <args>` in `cwd` (the service's when `None`) with `config_dir` as `GH_CONFIG_DIR`
    /// and `vars` (an account's token, [`token_vars`]); any exit code outside `ok` is an error
    /// carrying `gh`'s stderr, and so is more stdout than `keep` allows or `time` passing.
    fn output(
        &self,
        (config_dir, vars): (Option<&str>, &[(&'static str, String)]),
        cwd: Option<&Path>,
        args: &[&str],
        (ok, keep, time): (&[i32], Stdout, Duration),
    ) -> Result<Vec<u8>, String> {
        let mut command = Command::new(&self.program);
        for key in CLEARED {
            command.env_remove(key);
        }
        command
            .env("PATH", &self.path)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GH_SPINNER_DISABLED", "1")
            .env("NO_COLOR", "1")
            .env("GH_PAGER", "cat")
            // The git that `gh` runs (e.g. `gh pr checkout`) gets what `git::command` passes
            // Hive's own: no fsmonitor program, no signature check.
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_0", "core.fsmonitor")
            .env("GIT_CONFIG_VALUE_0", "false")
            .env("GIT_CONFIG_KEY_1", "log.showSignature")
            .env("GIT_CONFIG_VALUE_1", "false")
            .args(args);
        command.envs(config_dir.map(|dir| ("GH_CONFIG_DIR", dir)));
        command.envs(vars.iter().cloned());
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let os: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let out = git::limited(command, "gh", &os, &[], ok, keep, Some(time));
        out.map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => NOT_INSTALLED.to_owned(),
            _ => err.to_string(),
        })
    }

    /// The accounts `gh auth status` lists in `config_dir` (`gh`'s own config when `None`);
    /// an error when there is none. `gh` checks each token with GitHub, so this calls it once
    /// per account: only on request, never polled.
    pub fn accounts(&self, config_dir: Option<&str>) -> Result<Vec<GhLogin>, String> {
        // 1: no account at all, or the active one failed (the others are still listed).
        let args = ["auth", "status"];
        let out = self.output(
            (config_dir, &[]),
            None,
            &args,
            (&[0, 1], Stdout::Max(AUTH_OUTPUT), TIME),
        )?;
        let accounts = parse_status(&String::from_utf8_lossy(&out));
        match accounts.is_empty() {
            true => Err(NOT_LOGGED_IN.to_owned()),
            false => Ok(accounts),
        }
    }

    /// `account`'s token from `gh auth token` in `config_dir`, for an environment only.
    pub fn token(&self, config_dir: Option<&str>, account: &GhAccount) -> Result<String, String> {
        check_account(account)?;
        let GhAccount { host, login } = account;
        let args = ["auth", "token", "--hostname", host, "--user", login];
        let limits = (&[0][..], Stdout::Max(AUTH_OUTPUT), LOCAL_TIME);
        let out = self.output((config_dir, &[]), None, &args, limits)?;
        let out = String::from_utf8(out).unwrap_or_default();
        let token = out.trim();
        let usable = token.len() <= TOKEN_LIMIT && token.bytes().all(|b| b.is_ascii_graphic());
        match usable && !token.is_empty() {
            true => Ok(token.to_owned()),
            false => Err(format!("gh gave no usable token for {login} on {host}")),
        }
    }

    /// Makes `account` `gh`'s active one on its host (`gh auth switch`), in `config_dir`.
    pub fn switch(&self, config_dir: Option<&str>, account: &GhAccount) -> Result<(), String> {
        check_account(account)?;
        let GhAccount { host, login } = account;
        let args = ["auth", "switch", "--hostname", host, "--user", login];
        let limits = (&[0][..], Stdout::Max(AUTH_OUTPUT), LOCAL_TIME);
        self.output((config_dir, &[]), None, &args, limits)
            .map(drop)
    }

    /// The `gh_accounts` answer: the accounts in `config_dir`, after making `switch` the
    /// active one when given, and what went wrong, if anything.
    pub fn answer(
        &self,
        config_dir: Option<&str>,
        switch: Option<&GhAccount>,
    ) -> (Vec<GhLogin>, Option<String>) {
        let switched = switch.map_or(Ok(()), |account| self.switch(config_dir, account));
        let (accounts, listed) = match self.accounts(config_dir) {
            Ok(accounts) => (accounts, Ok(())),
            Err(problem) => (Vec::new(), Err(problem)),
        };
        (accounts, switched.and(listed).err())
    }

    /// What a terminal of a space with `env` gets besides `spaces::vars`: its account's token
    /// ([`token_vars`]); none without an account. When `gh` gives no token, why, to show
    /// (never holding the token): the terminal then uses `gh`'s active account.
    pub fn vars(&self, env: &SpaceEnv) -> Result<Vec<(&'static str, String)>, String> {
        let Some(account) = &env.gh_account else {
            return Ok(Vec::new());
        };
        let token = self.token(env.gh_config_dir.as_deref(), account);
        let GhAccount { host, login } = account;
        let token = token.map_err(|err| {
            format!("No GitHub token for {login} on {host}: this terminal uses gh's active account ({err})")
        })?;
        Ok(token_vars(account, token))
    }

    /// `gh <args>` in `cwd` as a terminal of the space with `env` would run it: with its
    /// `GH_CONFIG_DIR` and its account's `GH_TOKEN` and `GH_HOST` (`gh`'s active account
    /// without one). Its stdout kept as `keep` says, within [`TIME`]; the error (`gh`'s
    /// stderr, or why there is no token) is fit to show as is. For the pull requests (9.31)
    /// and Actions (9.32) views: the stdout is untrusted, to parse with its own limits.
    pub fn run(
        &self,
        env: &SpaceEnv,
        cwd: &Path,
        args: &[&str],
        keep: Stdout,
    ) -> Result<Vec<u8>, String> {
        let config_dir = env.gh_config_dir.as_deref();
        let vars = match &env.gh_account {
            Some(account) => token_vars(account, self.token(config_dir, account)?),
            None => Vec::new(),
        };
        self.output((config_dir, &vars), Some(cwd), args, (&[0], keep, TIME))
    }
}

/// The environment giving `gh` `account`'s `token`: `GH_TOKEN` on github.com and `*.ghe.com`,
/// `GH_ENTERPRISE_TOKEN` on any other host (GitHub Enterprise Server, where `gh` ignores
/// `GH_TOKEN`), and `GH_HOST`.
fn token_vars(account: &GhAccount, token: String) -> Vec<(&'static str, String)> {
    let host = account.host.as_str();
    let key = match host == "github.com" || host.ends_with(".ghe.com") {
        true => "GH_TOKEN",
        false => "GH_ENTERPRISE_TOKEN",
    };
    vec![(key, token), ("GH_HOST", account.host.clone())]
}

/// The accounts in `gh auth status`'s output (gh 2.40 and later: "✓ Logged in to <host>
/// account <login> (<source>)" or "X Failed to log in to …", then "- Active account: <bool>"),
/// at most [`ACCOUNTS_LIMIT`]. One naming a bad host or login is left out.
fn parse_status(out: &str) -> Vec<GhLogin> {
    let mut found: Vec<Option<GhLogin>> = Vec::new();
    for line in out.lines().map(str::trim) {
        if let Some(active) = line.strip_prefix("- Active account: ") {
            if let Some(Some(last)) = found.last_mut() {
                last.active = active == "true";
            }
        } else if line.starts_with("✓ ") {
            found.push(status_line(line, true));
        } else if line.starts_with("X ") {
            found.push(status_line(line, false));
        }
    }
    found.into_iter().flatten().take(ACCOUNTS_LIMIT).collect()
}

/// The account a status line names, when its host and login are valid.
fn status_line(line: &str, logged_in: bool) -> Option<GhLogin> {
    let (before, after) = line.split_once(" account ")?;
    let host = before.rsplit(' ').next().unwrap_or_default();
    let login = after.split(' ').next().unwrap_or_default();
    let account = GhAccount {
        host: host.to_owned(),
        login: login.to_owned(),
    };
    check_account(&account).ok()?;
    Some(GhLogin {
        host: account.host,
        login: account.login,
        active: false,
        logged_in,
    })
}

/// A login as GitHub makes them (1 to [`LOGIN_LIMIT`] ASCII letters, digits and `-`, plus `_`
/// for enterprise ones) and a host name (1 to [`HOST_LIMIT`] letters, digits, `.`, `-` and a
/// `:` port), neither starting with `-`, so neither can pass for an option of `gh`.
pub fn check_account(account: &GhAccount) -> Result<(), String> {
    let valid = |text: &str, limit: usize, extra: &[u8]| {
        let chars = |b: u8| b.is_ascii_alphanumeric() || extra.contains(&b);
        let bytes = text.as_bytes();
        !text.is_empty()
            && text.len() <= limit
            && bytes[0] != b'-'
            && bytes.iter().all(|b| chars(*b))
    };
    if !valid(&account.login, LOGIN_LIMIT, b"-_") {
        return Err(format!("{:?} is not a GitHub login", account.login));
    }
    if !valid(&account.host, HOST_LIMIT, b"-.:") {
        return Err(format!("{:?} is not a GitHub host", account.host));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::path::PathBuf;

    use super::*;

    const STATUS: &str = include_str!("../tests/fixtures/gh/auth-status.txt");
    const NONE: &str = include_str!("../tests/fixtures/gh/auth-status-none.txt");

    fn account(host: &str, login: &str) -> GhAccount {
        GhAccount {
            host: host.into(),
            login: login.into(),
        }
    }

    fn login(login: &str, active: bool, logged_in: bool) -> GhLogin {
        GhLogin {
            host: "github.com".into(),
            login: login.into(),
            active,
            logged_in,
        }
    }

    /// A fake `gh` in a temporary folder, logging each call's arguments and `gh` environment
    /// to `log`; `auth status` prints `status` (on stderr with exit 1 when it is [`NONE`]).
    #[cfg(unix)]
    struct Fake {
        dir: tempfile::TempDir,
        gh: Gh,
    }

    #[cfg(unix)]
    impl Fake {
        #[cfg(unix)]
        fn new(status: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("status.txt");
            std::fs::write(&file, status).unwrap();
            let log = dir.path().join("log");
            let (file, log) = (file.display(), log.display());
            let script = format!(
                r#"#!/bin/sh
printf '%s|%s|%s|%s|%s|%s\n' "$*" "${{GH_TOKEN-}}" "${{GH_HOST-}}" "${{GH_CONFIG_DIR-}}" "${{GH_PROMPT_DISABLED-}}" "$PWD" >> '{log}'
case "$1 $2" in
  "auth status") if grep -q 'not logged' '{file}'; then cat '{file}' >&2; exit 1; fi; cat '{file}' ;;
  "auth token") case "$6" in
      me) echo ' tok-me ' ;;
      tab) printf 'tok\tx' ;;
      long) head -c 1025 /dev/zero | tr '\0' a ;;
      max) head -c 1024 /dev/zero | tr '\0' a ;;
      fail) echo tok-secret; echo 'no oauth token found for me' >&2; exit 1 ;;
    esac ;;
  "auth switch") [ "$6" = me ] || {{ echo 'not logged in as them' >&2; exit 1; }} ;;
  "big out") head -c 100 /dev/zero ;;
  *) echo "ran $*" ;;
esac
"#
            );
            let program = dir.path().join("gh");
            std::fs::write(&program, script).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            let gh = Gh {
                program: program.into(),
                path: "/usr/bin:/bin".into(),
            };
            Self { dir, gh }
        }

        fn log(&self) -> String {
            std::fs::read_to_string(self.dir.path().join("log")).unwrap_or_default()
        }

        fn path(&self) -> PathBuf {
            // Resolved: on macOS the temporary folder is behind a link, and gh sees the real one.
            self.dir.path().canonicalize().unwrap()
        }
    }

    #[test]
    fn the_recorded_status_lists_each_account_once_with_its_state() {
        // A second, valid and inactive account, as gh prints one.
        let work = "  ✓ Logged in to github.com account octo-work (/home/user/.config/gh/hosts.yml)\n  - Active account: false\n";
        let status = format!("{STATUS}\n{work}");
        assert_eq!(
            parse_status(&status),
            [
                login("octo-personal", true, true),
                login("octo-old", false, false),
                login("octo-work", false, true),
            ]
        );
        assert_eq!(parse_status(NONE), []);
    }

    #[test]
    fn a_status_line_with_a_bad_login_is_left_out_with_its_active_line() {
        let status = "  ✓ Logged in to github.com account --help (x)\n  - Active account: true\n  X Failed to log in to github.com using token (GH_TOKEN)\n  - Active account: true\n  ✓ Logged in to ghe.example:8443 account me_corp (x)\n";
        let listed = parse_status(status);
        assert_eq!(
            listed,
            [GhLogin {
                host: "ghe.example:8443".into(),
                login: "me_corp".into(),
                active: false,
                logged_in: true,
            }]
        );
        let many = "  ✓ Logged in to h account a (x)\n".repeat(ACCOUNTS_LIMIT + 1);
        assert_eq!(parse_status(&many).len(), ACCOUNTS_LIMIT);
    }

    #[test]
    fn logins_and_hosts_are_checked() {
        let max = "a".repeat(LOGIN_LIMIT);
        for good in [
            account("github.com", "me"),
            account("github.com", &max),
            account("a", "a-b_c9"),
            account(&"h".repeat(HOST_LIMIT), "me"),
        ] {
            assert_eq!(check_account(&good), Ok(()), "{good:?}");
        }
        let too_long = format!("{max}a");
        for (login, bad) in [
            ("", account("github.com", "")),
            (too_long.as_str(), account("github.com", &too_long)),
            ("-me", account("github.com", "-me")),
            ("a b", account("github.com", "a b")),
            ("é", account("github.com", "é")),
        ] {
            let err = format!("{login:?} is not a GitHub login");
            assert_eq!(check_account(&bad), Err(err));
        }
        let long_host = "h".repeat(HOST_LIMIT + 1);
        for host in ["", "-h", "a/b", "a_b", &long_host] {
            let err = format!("{host:?} is not a GitHub host");
            assert_eq!(check_account(&account(host, "me")), Err(err));
        }
    }

    #[cfg(unix)]
    #[test]
    fn accounts_are_listed_from_gh_auth_status_in_the_config_folder() {
        let fake = Fake::new(STATUS);
        let listed = fake.gh.accounts(Some("/cfg")).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0], login("octo-personal", true, true));
        // The service's own GH_* are never passed on; prompts are off.
        let cwd = std::env::current_dir().unwrap();
        let line = format!("auth status|||/cfg|1|{}\n", cwd.display());
        assert_eq!(fake.log(), line);
        // Not logged in: exit 1 and a message on stderr, said plainly.
        let none = Fake::new(NONE);
        assert_eq!(none.gh.accounts(None), Err(NOT_LOGGED_IN.to_owned()));
        let lines = Fake::new("gh: not a status\n");
        assert_eq!(lines.gh.accounts(None), Err(NOT_LOGGED_IN.to_owned()));
        // Up to 64 KiB is read; more is refused.
        let padded = |len: usize| format!("{STATUS}{}", "\n".repeat(len - STATUS.len()));
        let most = Fake::new(&padded(AUTH_OUTPUT as usize));
        assert_eq!(most.gh.accounts(None).unwrap().len(), 2);
        let over = Fake::new(&padded(AUTH_OUTPUT as usize + 1));
        let refused = "gh auth status printed more than 65536 bytes";
        assert_eq!(over.gh.accounts(None), Err(refused.to_owned()));
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_gh_is_said_plainly() {
        let tmp = tempfile::tempdir().unwrap();
        let gh = Gh::on(tmp.path().into());
        assert_eq!(gh.program, "gh");
        assert_eq!(gh.accounts(None), Err(NOT_INSTALLED.to_owned()));
        let answer = gh.answer(None, Some(&account("github.com", "me")));
        assert_eq!(answer, (vec![], Some(NOT_INSTALLED.to_owned())));
    }

    #[cfg(unix)]
    #[test]
    fn a_token_is_given_only_when_usable_and_never_shown() {
        let fake = Fake::new(STATUS);
        let me = account("github.com", "me");
        assert_eq!(fake.gh.token(None, &me), Ok("tok-me".to_owned()));
        assert!(
            fake.log()
                .starts_with("auth token --hostname github.com --user me||||1|")
        );
        let max = fake.gh.token(None, &account("github.com", "max")).unwrap();
        assert_eq!(max, "a".repeat(TOKEN_LIMIT));
        for bad in ["tab", "long", "nobody"] {
            let err = fake
                .gh
                .token(None, &account("github.com", bad))
                .unwrap_err();
            assert_eq!(
                err,
                format!("gh gave no usable token for {bad} on github.com")
            );
        }
        let err = fake
            .gh
            .token(Some("/c"), &account("github.com", "fail"))
            .unwrap_err();
        assert_eq!(
            err,
            "gh auth token --hostname github.com --user fail failed: no oauth token found for me"
        );
        assert!(!err.contains("tok-secret"));
        let bad = account("github.com", "-x");
        assert_eq!(
            fake.gh.token(None, &bad),
            Err(r#""-x" is not a GitHub login"#.into())
        );
        // Nothing ran for the bad login.
        assert_eq!(fake.log().lines().count(), 6);
    }

    #[cfg(unix)]
    #[test]
    fn switching_runs_gh_auth_switch_and_lists_again() {
        let fake = Fake::new(STATUS);
        let me = account("github.com", "me");
        let (accounts, problem) = fake.gh.answer(Some("/c"), Some(&me));
        assert_eq!((accounts.len(), problem), (2, None));
        let log = fake.log();
        let calls: Vec<&str> = log.lines().map(|l| l.split('|').next().unwrap()).collect();
        assert_eq!(
            calls,
            ["auth switch --hostname github.com --user me", "auth status"]
        );
        // A refused switch still lists, with gh's reason.
        let them = account("github.com", "them");
        let (accounts, problem) = fake.gh.answer(None, Some(&them));
        assert_eq!(accounts.len(), 2);
        let refused =
            "gh auth switch --hostname github.com --user them failed: not logged in as them";
        assert_eq!(problem.as_deref(), Some(refused));
        assert_eq!(
            fake.gh.switch(None, &account("-h", "me")),
            Err(r#""-h" is not a GitHub host"#.into())
        );
        // Listing alone.
        assert_eq!(fake.gh.answer(None, None).1, None);
        let none = Fake::new(NONE);
        assert_eq!(
            none.gh.answer(None, None),
            (vec![], Some(NOT_LOGGED_IN.into()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_space_account_gives_its_token_and_host_only() {
        let fake = Fake::new(STATUS);
        let mut env = SpaceEnv::default();
        assert_eq!(fake.gh.vars(&env), Ok(vec![]));
        assert_eq!(fake.log(), "");
        env.gh_account = Some(account("github.com", "me"));
        let expected = vec![
            ("GH_TOKEN", "tok-me".into()),
            ("GH_HOST", "github.com".into()),
        ];
        assert_eq!(fake.gh.vars(&env), Ok(expected));
        env.gh_account = Some(account("github.com", "fail"));
        let failed = "No GitHub token for fail on github.com: this terminal uses gh's active account (gh auth token --hostname github.com --user fail failed: no oauth token found for me)";
        assert_eq!(fake.gh.vars(&env), Err(failed.to_owned()));
        // Other hosts: gh reads GH_TOKEN on github.com and GHE.com only.
        let ghe = |host: &str| token_vars(&account(host, "me"), "t".into())[0].0;
        assert_eq!(ghe("tenant.ghe.com"), "GH_TOKEN");
        assert_eq!(ghe("ghe.example:8443"), "GH_ENTERPRISE_TOKEN");
        assert_eq!(ghe("ghe.com"), "GH_ENTERPRISE_TOKEN");
    }

    #[cfg(unix)]
    #[test]
    fn gh_runs_as_the_space_in_the_folder_with_its_output_limited() {
        let fake = Fake::new(STATUS);
        let dir = fake.path();
        let mut env = SpaceEnv {
            gh_config_dir: Some("/cfg".into()),
            ..SpaceEnv::default()
        };
        let most = Stdout::Max(1024);
        let out = fake.gh.run(&env, &dir, &["pr", "list"], most).unwrap();
        assert_eq!(out, b"ran pr list\n");
        let at = dir.display();
        assert_eq!(fake.log(), format!("pr list|||/cfg|1|{at}\n"));
        env.gh_account = Some(account("github.com", "me"));
        fake.gh.run(&env, &dir, &["run", "list"], most).unwrap();
        let last = fake.log().lines().last().unwrap().to_owned();
        assert_eq!(last, format!("run list|tok-me|github.com|/cfg|1|{at}"));
        let big = fake.gh.run(&env, &dir, &["big", "out"], Stdout::Max(99));
        assert_eq!(big.unwrap_err(), "gh big out printed more than 99 bytes");
        let all = fake.gh.run(&env, &dir, &["big", "out"], Stdout::Max(100));
        assert_eq!(all.unwrap().len(), 100);
        // Or only its tail.
        let tail = fake.gh.run(&env, &dir, &["big", "out"], Stdout::Tail(7));
        assert_eq!(tail.unwrap(), [0; 7]);
        // No token, nothing run.
        env.gh_account = Some(account("github.com", "fail"));
        let calls = fake.log().lines().count();
        let err = fake.gh.run(&env, &dir, &["pr", "list"], most).unwrap_err();
        assert!(err.starts_with("gh auth token"), "{err}");
        assert_eq!(fake.log().lines().count(), calls + 1);
    }
}
