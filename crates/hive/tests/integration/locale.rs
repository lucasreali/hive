//! A terminal's locale and name (D18.2): UTF-8 when the service has no locale.

use std::process::Command;

use hive_protocol::Role;

use crate::common::Env;

/// `hive` without any locale variable, as an app started from the Finder.
fn without_locale(env: &Env) -> Command {
    let mut hive = env.hive();
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name == "LANG" || name.starts_with("LC_") {
            hive.env_remove(key);
        }
    }
    hive
}

/// The `LANG` a terminal gets when the service has no locale, as the service works it out.
fn fallback_lang() -> String {
    #[cfg(target_os = "macos")]
    let (system, fallback) = (hive::macos::system_locale(), hive::locale::MAC_FALLBACK);
    #[cfg(target_os = "linux")]
    let (system, fallback) = (None::<String>, hive::locale::LINUX_FALLBACK);
    let exists = hive::locale::listed(std::path::Path::new(hive::locale::LOCALE_DIR));
    let env = hive::locale::terminal_env(system.as_deref(), exists, |_| false, fallback);
    let lang = env.into_iter().find(|(name, _)| *name == "LANG");
    lang.map(|(_, lang)| lang).unwrap()
}

/// The shells a terminal can run here: on macOS `$SHELL` (fish, bash or zsh), on Linux fish.
fn shells() -> Vec<&'static str> {
    if cfg!(target_os = "macos") {
        vec!["fish", "/bin/bash", "/bin/zsh"]
    } else {
        vec!["fish"]
    }
}

#[tokio::test]
async fn a_terminal_without_a_locale_gets_a_utf8_one() {
    for shell in shells() {
        let env = Env::new();
        let mut hive = without_locale(&env);
        hive.env("SHELL", shell);
        let mut daemon = env.daemon_with(&mut hive);
        let mut app = env.connect(Role::App).await;
        app.open_terminal(1, &env.path("home")).await;
        // The quotes keep the command's echo from matching the answer.
        let ask = "echo \"cs=\"$(locale charmap)\" \"$TERM_PROGRAM\"/\"$COLORTERM\r";
        app.input(1, ask).await;
        app.output_until(1, "cs=UTF-8 Hive/truecolor").await;
        drop(app);
        assert!(daemon.wait_exit().success(), "{shell}");
    }
}

#[tokio::test]
async fn a_terminal_keeps_the_services_locale_and_names_its_version() {
    let kept = "xx_YY.kept";
    for (set, want) in [
        (kept.to_owned(), kept.to_owned()),
        (String::new(), fallback_lang()),
    ] {
        let env = Env::new();
        let mut hive = without_locale(&env);
        hive.env("LANG", &set);
        let mut daemon = env.daemon_with(&mut hive);
        let mut app = env.connect(Role::App).await;
        app.open_terminal(1, &env.path("home")).await;
        app.input(1, "echo \"lang=[\"$LANG\"] v=\"$TERM_PROGRAM_VERSION\r")
            .await;
        let version = hive::VERSION;
        app.output_until(1, &format!("lang=[{want}] v={version}"))
            .await;
        drop(app);
        assert!(daemon.wait_exit().success(), "{set:?}");
    }
}

/// The 18.1 security review's reproduction: in the `C` locale, readline in macOS's bash 3.2
/// reads `Å` (`C3 85`) as Meta keys, `\x85` being M-C-e (shell-expand-line), so the typed name
/// of a dropped file `/tmp/$(touch PWNED)ÅÅ` would run its command. In UTF-8 it stays text.
#[tokio::test]
async fn a_dropped_file_name_typed_into_bash_runs_nothing() {
    let env = Env::new();
    let mut daemon = env.daemon_with(&mut without_locale(&env));
    let mut app = env.connect(Role::App).await;
    let home = env.path("home");
    app.open_terminal(1, &home).await;
    // The prompt's quotes keep the command's echo from matching it.
    app.input(1, "env PS1='rd''y> ' bash --norc --noprofile -i\r")
        .await;
    app.output_until(1, "rdy> ").await;
    // As 18.1's `quote::typed` writes the path: backslash-escaped, no Enter.
    app.input(1, "/tmp/\\$\\(touch\\ PWNED\\)ÅÅ ").await;
    // Then the line is cleared (Ctrl+U) and a marker run, so every byte above was read.
    app.input(1, "\x15echo \"do\"\"ne\"\r").await;
    app.output_until(1, "done").await;
    assert!(!home.join("PWNED").exists());
    app.input(1, "exit\r").await;
    drop(app);
    assert!(daemon.wait_exit().success());
}
