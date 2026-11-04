//! Moves the files of Hive before 22.1, in the XDG data and config folders, into Hive's folder
//! (`~/.hive`, D22.16), when the service starts.

use std::io;
use std::path::{Path, PathBuf};

use crate::paths::Paths;

/// Moves into Hive's folder (`paths.data`, made with mode 0700) everything the old data folder
/// holds and the old config folder's `settings.json`, never over something already there,
/// which stays where it was (the old `bin/` once the app installed into the new one). So it
/// runs once: what moved is no longer in the old place. A move that fails puts back what moved
/// and leaves `paths` on the old folders, and the reason is returned (for the log and the status
/// bar). Then a `~/.local/bin/hive` link (`home`'s) to the old `bin/hive` is pointed at the new
/// one.
pub fn migrate(
    paths: &mut Paths,
    old_data: &Path,
    old_config: &Path,
    home: Option<&Path>,
) -> Option<String> {
    migrate_with(paths, old_data, old_config, home, &|from, to| {
        std::fs::rename(from, to)
    })
}

type Rename = dyn Fn(&Path, &Path) -> io::Result<()>;

fn migrate_with(
    paths: &mut Paths,
    old_data: &Path,
    old_config: &Path,
    home: Option<&Path>,
    rename: &Rename,
) -> Option<String> {
    let new = paths.data.clone();
    let moved = crate::mode::folder(std::fs::DirBuilder::new().recursive(true), 0o700)
        .create(&new)
        .and_then(|()| items(old_data, old_config, &new))
        .and_then(|items| move_all(items, rename));
    match moved {
        Ok(()) => {
            if let Some(home) = home {
                relink(
                    &home.join(".local/bin/hive"),
                    &old_data.join("bin/hive"),
                    &paths.bin_dir().join("hive"),
                );
            }
            None
        }
        Err(err) => {
            let why = format!(
                "cannot move Hive's files into {} ({err}): using {} and {}",
                new.display(),
                old_data.display(),
                old_config.display()
            );
            eprintln!("hive: {why}");
            paths.data = old_data.to_owned();
            paths.config = old_config.to_owned();
            Some(why)
        }
    }
}

/// What to move, as (old place, new place): only what is not already in `new` (all of it when
/// an old folder is the new one).
fn items(old_data: &Path, old_config: &Path, new: &Path) -> io::Result<Vec<(PathBuf, PathBuf)>> {
    let entries = match std::fs::read_dir(old_data) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
        entries => entries?.collect::<io::Result<_>>()?,
    };
    let mut items: Vec<_> = entries
        .iter()
        .map(|entry| (entry.path(), new.join(entry.file_name())))
        .collect();
    let settings = old_config.join("settings.json");
    if settings.symlink_metadata().is_ok() {
        items.push((settings, new.join("settings.json")));
    }
    items.retain(|(_, to)| to.symlink_metadata().is_err());
    Ok(items)
}

/// Moves each item, or none: after a failure what moved goes back.
fn move_all(items: Vec<(PathBuf, PathBuf)>, rename: &Rename) -> io::Result<()> {
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in items {
        if let Err(err) = move_one(&from, &to, rename) {
            for (from, to) in moved.iter().rev() {
                if let Err(back) = move_one(to, from, rename) {
                    eprintln!(
                        "hive: cannot put {} back in {}: {back}",
                        to.display(),
                        from.display()
                    );
                }
            }
            return Err(io::Error::new(
                err.kind(),
                format!("{}: {err}", from.display()),
            ));
        }
        moved.push((from, to));
    }
    Ok(())
}

/// A rename, or across file systems a copy then the removal of the old one. A copy that fails
/// is removed; an old one that cannot be removed stays, the copy being whole.
fn move_one(from: &Path, to: &Path, rename: &Rename) -> io::Result<()> {
    match rename(from, to) {
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices => {
            if let Err(err) = copy(from, to) {
                let _ = remove(to);
                return Err(err);
            }
            if let Err(err) = remove(from) {
                eprintln!(
                    "hive: copied {} but cannot remove it: {err}",
                    from.display()
                );
            }
            Ok(())
        }
        moved => moved,
    }
}

/// Copies a file, a link or a whole folder, keeping their modes.
fn copy(from: &Path, to: &Path) -> io::Result<()> {
    let meta = from.symlink_metadata()?;
    if meta.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)
    } else if meta.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy(&entry.path(), &to.join(entry.file_name()))?;
        }
        std::fs::set_permissions(to, meta.permissions())
    } else {
        std::fs::copy(from, to).map(drop)
    }
}

fn remove(path: &Path) -> io::Result<()> {
    if path.symlink_metadata()?.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Points `link` at `new` when it is a link to `old` and `new` exists, in one step.
fn relink(link: &Path, old: &Path, new: &Path) {
    if std::fs::read_link(link).ok().as_deref() != Some(old) || !new.is_file() {
        return;
    }
    let tmp = link.with_extension("hive-new");
    let _ = std::fs::remove_file(&tmp);
    let linked = std::os::unix::fs::symlink(new, &tmp).and_then(|()| std::fs::rename(&tmp, link));
    if let Err(err) = linked {
        eprintln!(
            "hive: cannot point {} at {}: {err}",
            link.display(),
            new.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Setup {
        tmp: tempfile::TempDir,
        paths: Paths,
    }

    impl Setup {
        /// v0.23.0's layout under a temporary home: `~/.local/share/hive`, `~/.config/hive`.
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let home = tmp.path().join("home");
            let paths = Paths {
                runtime: tmp.path().join("run"),
                logs: tmp.path().join("run"),
                data: home.join(".hive"),
                config: home.join(".hive"),
            };
            std::fs::create_dir_all(&home).unwrap();
            Self { tmp, paths }
        }

        fn home(&self) -> PathBuf {
            self.tmp.path().join("home")
        }

        fn old_data(&self) -> PathBuf {
            self.home().join(".local/share/hive")
        }

        fn old_config(&self) -> PathBuf {
            self.home().join(".config/hive")
        }

        fn write(&self, path: &Path, text: &str) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        fn old_layout(&self) {
            let data = self.old_data();
            for name in [
                "spaces.json",
                "ports.json",
                "offered-pulls.json",
                "hive-hooks.json",
            ] {
                self.write(&data.join(name), name);
            }
            self.write(&data.join("bin/claude"), "wrapper");
            self.write(&data.join("bin/hive"), "old hive");
            self.write(&data.join("shell/.zshrc"), "zshrc");
            self.write(&data.join("shared/skills/a/SKILL.md"), "skill");
            self.write(&self.old_config().join("settings.json"), "settings");
        }

        fn run(&mut self, rename: &Rename) -> Option<String> {
            let (data, config, home) = (self.old_data(), self.old_config(), self.home());
            migrate_with(&mut self.paths, &data, &config, Some(&home), rename)
        }

        fn new_file(&self, sub: &str) -> String {
            std::fs::read_to_string(self.paths.data.join(sub)).unwrap()
        }
    }

    fn rename(from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn every_old_file_and_folder_moves_into_the_hive_folder() {
        let mut s = Setup::new();
        s.old_layout();
        assert_eq!(s.run(&rename), None);
        let home = s.home().join(".hive");
        assert_eq!(s.paths.data, home);
        assert_eq!(mode(&home), 0o700);
        for name in [
            "spaces.json",
            "ports.json",
            "offered-pulls.json",
            "hive-hooks.json",
        ] {
            assert_eq!(s.new_file(name), name);
        }
        assert_eq!(s.new_file("settings.json"), "settings");
        assert_eq!(s.new_file("bin/claude"), "wrapper");
        assert_eq!(s.new_file("shell/.zshrc"), "zshrc");
        assert_eq!(s.new_file("shared/skills/a/SKILL.md"), "skill");
        // The old folders are left, empty.
        assert_eq!(std::fs::read_dir(s.old_data()).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(s.old_config()).unwrap().count(), 0);
        // Once: nothing is left to move.
        assert_eq!(s.run(&rename), None);
        assert_eq!(s.new_file("spaces.json"), "spaces.json");
    }

    #[test]
    fn a_partial_old_layout_or_none_moves_what_there_is() {
        let mut s = Setup::new();
        s.write(&s.old_data().join("spaces.json"), "spaces");
        assert_eq!(s.run(&rename), None);
        assert_eq!(s.new_file("spaces.json"), "spaces");
        assert!(!s.paths.data.join("settings.json").exists());
        // A new install: no old folder at all.
        let mut s = Setup::new();
        assert_eq!(s.run(&rename), None);
        assert_eq!(std::fs::read_dir(&s.paths.data).unwrap().count(), 0);
        // Only settings.
        let mut s = Setup::new();
        s.write(&s.old_config().join("settings.json"), "settings");
        assert_eq!(s.run(&rename), None);
        assert_eq!(s.new_file("settings.json"), "settings");
    }

    #[test]
    fn a_file_already_in_the_hive_folder_is_not_overwritten() {
        let mut s = Setup::new();
        s.old_layout();
        // The app installed its `hive` into the new folder first.
        s.write(&s.paths.data.join("bin/hive"), "new hive");
        s.write(&s.paths.data.join("settings.json"), "new settings");
        assert_eq!(s.run(&rename), None);
        assert_eq!(s.new_file("bin/hive"), "new hive");
        assert!(!s.paths.data.join("bin/claude").exists());
        assert_eq!(s.new_file("settings.json"), "new settings");
        assert_eq!(s.new_file("spaces.json"), "spaces.json");
        // What was in the way stays in the old place.
        let old = |sub: &str| std::fs::read_to_string(s.old_data().join(sub)).unwrap();
        assert_eq!(old("bin/hive"), "old hive");
        let settings = std::fs::read_to_string(s.old_config().join("settings.json"));
        assert_eq!(settings.unwrap(), "settings");
        assert!(!s.old_data().join("spaces.json").exists());
    }

    #[test]
    fn the_same_folder_moves_nothing() {
        let mut s = Setup::new();
        s.write(&s.paths.data.join("spaces.json"), "spaces");
        let data = s.paths.data.clone();
        assert_eq!(migrate(&mut s.paths, &data, &data, None), None);
        assert_eq!(s.new_file("spaces.json"), "spaces");
    }

    #[test]
    fn a_failed_move_puts_back_what_moved_and_keeps_the_old_folders() {
        let mut s = Setup::new();
        s.old_layout();
        let fail = |from: &Path, to: &Path| {
            if from.ends_with("settings.json") {
                return Err(io::Error::other("no space"));
            }
            std::fs::rename(from, to)
        };
        let why = s.run(&fail).unwrap();
        assert!(
            why.contains("no space") && why.contains("settings.json"),
            "{why}"
        );
        assert!(why.contains(&s.old_data().display().to_string()), "{why}");
        assert_eq!(s.paths.data, s.old_data());
        assert_eq!(s.paths.config, s.old_config());
        assert_eq!(s.paths.settings(), s.old_config().join("settings.json"));
        // Everything is in its old place.
        let old = |sub: &str| std::fs::read_to_string(s.old_data().join(sub)).unwrap();
        assert_eq!(old("spaces.json"), "spaces.json");
        assert_eq!(old("shared/skills/a/SKILL.md"), "skill");
        let home = s.home().join(".hive");
        assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
        // One that cannot go back stays where it went, said in the log.
        let mut s = Setup::new();
        s.old_layout();
        let once = std::cell::Cell::new(false);
        let fail = move |from: &Path, to: &Path| {
            if once.replace(true) {
                return Err(io::Error::other("stuck"));
            }
            std::fs::rename(from, to)
        };
        assert!(s.run(&fail).is_some());
        let moved = std::fs::read_dir(s.home().join(".hive")).unwrap().count();
        assert_eq!(moved, 1);
    }

    #[test]
    fn a_hive_folder_that_cannot_be_made_or_an_unreadable_old_one_keeps_the_old_folders() {
        let mut s = Setup::new();
        s.old_layout();
        s.write(&s.home().join(".hive"), "a file");
        let why = s.run(&rename).unwrap();
        assert!(why.starts_with("cannot move Hive's files into"), "{why}");
        assert_eq!(s.paths.data, s.old_data());
        // An old data "folder" that is a file cannot be listed.
        let mut s = Setup::new();
        s.write(&s.old_data(), "a file");
        assert!(s.run(&rename).is_some());
        assert_eq!(s.paths.data, s.old_data());
    }

    #[test]
    fn across_file_systems_a_copy_replaces_the_old_one() {
        let mut s = Setup::new();
        s.old_layout();
        let data = s.old_data();
        std::os::unix::fs::symlink("skills/a", data.join("shared/link")).unwrap();
        std::fs::set_permissions(data.join("bin/claude"), PermissionsExt::from_mode(0o755))
            .unwrap();
        let cross = |_: &Path, _: &Path| Err(io::ErrorKind::CrossesDevices.into());
        assert_eq!(s.run(&cross), None);
        assert_eq!(s.new_file("shared/skills/a/SKILL.md"), "skill");
        assert_eq!(s.new_file("settings.json"), "settings");
        assert_eq!(mode(&s.paths.data.join("bin/claude")), 0o755);
        let link = std::fs::read_link(s.paths.data.join("shared/link")).unwrap();
        assert_eq!(link, Path::new("skills/a"));
        assert_eq!(std::fs::read_dir(&data).unwrap().count(), 0);
    }

    #[test]
    fn across_file_systems_a_failed_copy_is_removed_and_an_old_one_kept_stays() {
        let s = Setup::new();
        let cross = |_: &Path, _: &Path| Err(io::ErrorKind::CrossesDevices.into());
        // A socket cannot be copied.
        let from = s.tmp.path().join("sock");
        let _listener = std::os::unix::net::UnixListener::bind(&from).unwrap();
        let dir = s.tmp.path().join("dir");
        s.write(&dir.join("a"), "a");
        std::os::unix::net::UnixListener::bind(dir.join("sock")).unwrap();
        let to = s.tmp.path().join("to");
        assert!(move_one(&dir, &to, &cross).is_err());
        assert!(!to.exists());
        assert!(dir.join("a").exists());
        assert!(move_one(&from, &to, &cross).is_err());
        // An old one in a folder that cannot be written to: copied, and left.
        let locked = s.tmp.path().join("locked");
        s.write(&locked.join("file"), "kept");
        std::fs::set_permissions(&locked, PermissionsExt::from_mode(0o500)).unwrap();
        move_one(&locked.join("file"), &to, &cross).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "kept");
        assert!(locked.join("file").exists());
        std::fs::set_permissions(&locked, PermissionsExt::from_mode(0o700)).unwrap();
        // A plain rename is just that.
        move_one(&to, &s.tmp.path().join("again"), &rename).unwrap();
        assert!(move_one(&to, &s.tmp.path().join("again"), &rename).is_err());
    }

    #[test]
    fn the_shortcut_to_the_old_hive_points_at_the_new_one() {
        let mut s = Setup::new();
        s.old_layout();
        s.write(&s.paths.data.join("bin/hive"), "new hive");
        let link = s.home().join(".local/bin/hive");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(s.old_data().join("bin/hive"), &link).unwrap();
        assert_eq!(s.run(&rename), None);
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            s.paths.bin_dir().join("hive")
        );
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "new hive");
        // A link elsewhere, or with no new `hive` to point at, is left alone.
        let other = Path::new("/usr/local/bin/hive");
        let s2 = s.tmp.path().join("other");
        std::os::unix::fs::symlink(other, &s2).unwrap();
        relink(
            &s2,
            &s.old_data().join("bin/hive"),
            &s.paths.bin_dir().join("hive"),
        );
        assert_eq!(std::fs::read_link(&s2).unwrap(), other);
        relink(
            &link,
            &s.paths.bin_dir().join("hive"),
            &s.tmp.path().join("none"),
        );
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            s.paths.bin_dir().join("hive")
        );
        // A folder where it cannot be replaced: it stays as it was.
        let locked = s.tmp.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let old = s.old_data().join("bin/hive");
        std::os::unix::fs::symlink(&old, locked.join("hive")).unwrap();
        std::fs::set_permissions(&locked, PermissionsExt::from_mode(0o500)).unwrap();
        relink(&locked.join("hive"), &old, &s.paths.bin_dir().join("hive"));
        assert_eq!(std::fs::read_link(locked.join("hive")).unwrap(), old);
        std::fs::set_permissions(&locked, PermissionsExt::from_mode(0o700)).unwrap();
    }
}
