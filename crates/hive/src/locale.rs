//! The environment a terminal's shell gets besides `TERM` (D18.2): a UTF-8 `LANG` when the
//! service has no locale, and the terminal's own name.
//!
//! An app started from the Finder has no `LANG`, so a shell would run in the `C` locale. There
//! macOS's `/bin/bash` 3.2 (readline's `convert-meta`, no bracketed paste) turns every byte
//! from 0x80 up into a Meta key: a dropped or pasted file name such as `/tmp/$(touch x)ÅÅ`
//! (`Å` is `C3 85`, and `\x85` is M-C-e, shell-expand-line) would run its command. In a UTF-8
//! locale those bytes are typed as characters.

use std::path::{Path, PathBuf};

/// The variables that choose the character set, most specific first: any one set keeps the
/// user's choice.
pub const LOCALE_VARS: [&str; 3] = ["LC_ALL", "LC_CTYPE", "LANG"];

/// Where macOS (and BSDs) keep one folder per locale.
pub const LOCALE_DIR: &str = "/usr/share/locale";

/// When the system locale has no UTF-8 POSIX locale: macOS's English one (`en_BR` has none).
pub const MAC_FALLBACK: &str = "en_US.UTF-8";

/// Linux's: glibc (2.35 on) and musl build `C.UTF-8` in, and Debian and Ubuntu ship it, while
/// any other may be missing from a minimal system.
pub const LINUX_FALLBACK: &str = "C.UTF-8";

/// What the shell is told it runs in: Hive and its version.
pub const TERM_PROGRAM: &str = "Hive";

/// The variables a terminal's shell gets. `system` is the system locale (macOS's
/// `AppleLocale`, e.g. `pt_BR`, `en_US@rg=brzzzz`, `zh-Hans_CN`), `exists` whether a POSIX
/// locale of that name exists, `set` whether the service's environment has a variable set
/// (not empty), and `fallback` the locale used when the system's has no UTF-8 one.
pub fn terminal_env(
    system: Option<&str>,
    exists: impl Fn(&str) -> bool,
    set: impl Fn(&str) -> bool,
    fallback: &str,
) -> Vec<(&'static str, String)> {
    let mut env = Vec::with_capacity(4);
    if !LOCALE_VARS.into_iter().any(set) {
        let lang = system
            .and_then(utf8_locale)
            .filter(|name| exists(name))
            .unwrap_or_else(|| fallback.to_owned());
        env.push(("LANG", lang));
    }
    env.push(("COLORTERM", "truecolor".to_owned()));
    env.push(("TERM_PROGRAM", TERM_PROGRAM.to_owned()));
    env.push(("TERM_PROGRAM_VERSION", crate::VERSION.to_owned()));
    env
}

/// The UTF-8 POSIX name of a system locale: `pt_BR` → `pt_BR.UTF-8`. Options after `@` and a
/// script after `-` are dropped (`zh-Hans_CN` → `zh_CN.UTF-8`); `None` without a region or
/// with anything but ASCII letters and digits (the name becomes a path).
fn utf8_locale(system: &str) -> Option<String> {
    let system = system.split('@').next().unwrap_or_default();
    let (language, region) = system.split_once('_')?;
    let language = language.split('-').next().unwrap_or_default();
    let plain = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric());
    (plain(language) && plain(region)).then(|| format!("{language}_{region}.UTF-8"))
}

/// Whether a locale named `name` exists under `dir` (one folder per locale).
pub fn listed(dir: &Path) -> impl Fn(&str) -> bool {
    let dir: PathBuf = dir.to_owned();
    move |name| dir.join(name).is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The locales of this test's system.
    fn exists(name: &str) -> bool {
        ["pt_BR.UTF-8", "en_US.UTF-8", "zh_CN.UTF-8"].contains(&name)
    }

    fn lang(system: Option<&str>, set: &[&str]) -> Option<String> {
        let env = terminal_env(system, exists, |name| set.contains(&name), MAC_FALLBACK);
        let lang = env.iter().find(|(name, _)| *name == "LANG");
        lang.map(|(_, value)| value.clone())
    }

    #[test]
    fn the_system_locale_gives_its_utf8_lang() {
        assert_eq!(lang(Some("pt_BR"), &[]).as_deref(), Some("pt_BR.UTF-8"));
        assert_eq!(
            lang(Some("pt_BR@rg=uszzzz"), &[]).as_deref(),
            Some("pt_BR.UTF-8")
        );
        assert_eq!(
            lang(Some("zh-Hans_CN"), &[]).as_deref(),
            Some("zh_CN.UTF-8")
        );
    }

    #[test]
    fn a_locale_without_a_posix_one_falls_back() {
        for system in [
            Some("en_BR"),
            Some("es_419"),
            Some("en"),
            Some("_BR"),
            Some("pt_"),
            Some("../x_y"),
            Some("pt_BR/.."),
            Some(""),
            None,
        ] {
            assert_eq!(
                lang(system, &[]).as_deref(),
                Some("en_US.UTF-8"),
                "{system:?}"
            );
        }
        let env = terminal_env(Some("en_BR"), exists, |_| false, LINUX_FALLBACK);
        assert_eq!(env[0], ("LANG", "C.UTF-8".to_owned()));
    }

    #[test]
    fn only_a_plain_language_and_region_make_a_name() {
        assert_eq!(utf8_locale("en_BR").as_deref(), Some("en_BR.UTF-8"));
        for system in ["_BR", "pt_", "../x_y", "x_../y", "pt_BR/..", "en", ""] {
            assert_eq!(utf8_locale(system), None, "{system:?}");
        }
    }

    #[test]
    fn a_locale_already_set_is_kept() {
        for var in LOCALE_VARS {
            assert_eq!(lang(Some("pt_BR"), &[var]), None, "{var}");
        }
        // Other variables do not count.
        assert!(lang(Some("pt_BR"), &["LC_TIME", "LANGUAGE"]).is_some());
    }

    #[test]
    fn the_terminal_names_itself() {
        let env = terminal_env(None, exists, |_| true, MAC_FALLBACK);
        let want = [
            ("COLORTERM", "truecolor".to_owned()),
            ("TERM_PROGRAM", "Hive".to_owned()),
            ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION").to_owned()),
        ];
        assert_eq!(env, want);
        let env = terminal_env(None, exists, |_| false, MAC_FALLBACK);
        assert_eq!(env.len(), 4);
        assert_eq!(env[1..], want);
    }

    #[test]
    fn listed_locales_are_folders_of_the_locale_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pt_BR.UTF-8")).unwrap();
        std::fs::write(dir.path().join("en_US.UTF-8"), "").unwrap();
        let exists = listed(dir.path());
        assert!(exists("pt_BR.UTF-8"));
        assert!(!exists("en_US.UTF-8"));
        assert!(!exists("en_BR.UTF-8"));
    }
}
