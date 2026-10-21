//! The user's login `PATH` for every program the service starts (17.5). A service started by
//! the macOS app has the GUI's `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), one started through
//! `wsl.exe` lacks what the user's shell config adds: its git would not be the user's
//! (Homebrew's), and git hooks (husky, lefthook), `git-lfs` filters and the project's scripts
//! would not find `npm`, `bun` or `docker`. Once the service has asked the user's shell for its
//! `PATH` ([`crate::wrapper::user_path`], on first need), git and the scripts get that one;
//! before, the service's own, as always. (`gh` is given it directly, `crate::gh::Gh`.)

use std::ffi::{OsStr, OsString};
use std::process::Command;
use std::sync::{PoisonError, RwLock};

/// The user's `PATH`, once known.
static KNOWN: RwLock<Option<OsString>> = RwLock::new(None);

/// Records the user's `PATH`, for every program started from now on.
pub fn set(path: OsString) {
    *KNOWN.write().unwrap_or_else(PoisonError::into_inner) = Some(path);
}

/// `command` with the user's `PATH` once it is known (the service's own until then). A
/// `PATH` set on a `Command` is also where its program is looked for.
pub fn apply(command: &mut Command) -> &mut Command {
    let known = KNOWN.read().unwrap_or_else(PoisonError::into_inner);
    with_path(command, known.as_deref())
}

/// `command` with `path` as its `PATH`, unless there is none or it is empty (which would find
/// no program at all).
fn with_path<'a>(command: &'a mut Command, path: Option<&OsStr>) -> &'a mut Command {
    match path.filter(|path| !path.is_empty()) {
        Some(path) => command.env("PATH", path),
        None => command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path_of(command: &Command) -> Option<Option<&OsStr>> {
        let mut envs = command.get_envs();
        envs.find(|(key, _)| *key == "PATH").map(|(_, value)| value)
    }

    #[test]
    fn a_known_path_is_given_an_unknown_or_empty_one_is_not() {
        let mut command = Command::new("git");
        let given = with_path(&mut command, Some(OsStr::new("/login/bin:/usr/bin")));
        assert_eq!(
            path_of(given),
            Some(Some(OsStr::new("/login/bin:/usr/bin")))
        );
        for path in [None, Some(OsStr::new(""))] {
            let mut command = Command::new("git");
            assert_eq!(path_of(with_path(&mut command, path)), None);
        }
    }
}
