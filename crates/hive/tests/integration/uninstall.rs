//! Uninstalling Hive (23.5): from the app (`uninstall`) and from a terminal (`hive uninstall`),
//! in a throwaway home. On macOS what goes is moved to the Trash: the home and the runtime
//! folder are on a disk image of their own, whose Trash is never the user's.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use hive_protocol::{Control, Role, UninstallItem, UninstallKind};

use crate::common::Env;

/// A home with what Hive makes and what it must never touch, its runtime folder beside it.
struct Home {
    /// Dropped before `env`: the image is detached before the folders go.
    #[cfg(target_os = "macos")]
    volume: crate::common::Volume,
    env: Env,
}

/// What the service or `hive uninstall` never removes.
const KEPT: [&str; 6] = [
    ".claude/settings.json",
    ".claude.json",
    ".claude-work/settings.json",
    "src/shop/.claude/worktrees/a/file",
    ".local/bin/other",
    "Library/Preferences/com.other.plist",
];

impl Home {
    fn new() -> Self {
        let env = Env::new();
        #[cfg(target_os = "macos")]
        let volume = crate::common::Volume::new(&env.path("trash.dmg"), &env.path("run"));
        let home = Self {
            #[cfg(target_os = "macos")]
            volume,
            env,
        };
        for dir in [
            ".hive/accounts/work",
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
            "apps/Hive.app/Contents",
        ] {
            std::fs::create_dir_all(home.path(dir)).unwrap();
        }
        for file in KEPT {
            std::fs::write(home.path(file), "x").unwrap();
        }
        // The old folders, left where they were: their files were already in `~/.hive`.
        for file in [
            ".hive/ports.json",
            ".local/share/hive/ports.json",
            ".config/hive/notes.txt",
            "Library/Preferences/com.lucas.hive.plist",
        ] {
            std::fs::write(home.path(file), "{}").unwrap();
        }
        let shortcut = home.path(".local/bin/hive");
        std::os::unix::fs::symlink(home.path(".hive/bin/hive"), shortcut).unwrap();
        home
    }

    /// `rel` in the home (`run/home`, on the image on macOS).
    fn path(&self, rel: &str) -> PathBuf {
        self.env.path("run/home").join(rel)
    }

    fn app(&self) -> PathBuf {
        self.path("apps/Hive.app")
    }

    /// `hive` with this home, `~/.hive` as Hive's folder (no `HIVE_HOME`) and the old folders
    /// under it, and `apps/` as the only folder holding apps.
    fn hive(&self) -> Command {
        let mut hive = self.env.hive();
        hive.env("HOME", self.path(""))
            .env_remove("HIVE_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env(hive::uninstall::APPLICATIONS, self.path("apps"));
        hive
    }

    /// `hive uninstall <args>`, answering `answer`.
    fn uninstall(&self, args: &[&str], answer: &str) -> Output {
        let mut child = self.hive();
        let child = child.arg("uninstall").args(args);
        let mut child = (child.stdin(Stdio::piped()).stdout(Stdio::piped()))
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(answer.as_bytes()).unwrap();
        drop(stdin);
        child.wait_with_output().unwrap()
    }

    /// Whether `rel` is still there.
    fn has(&self, rel: &str) -> bool {
        self.path(rel).symlink_metadata().is_ok()
    }

    /// How many entries this user's Trash of the image holds.
    #[cfg(target_os = "macos")]
    fn trashed(&self) -> usize {
        // Finder's own `.DS_Store` aside.
        let entries = std::fs::read_dir(self.volume.trashed("")).into_iter();
        let entries = entries.flatten().flatten();
        entries.filter(|e| e.file_name() != ".DS_Store").count()
    }

    fn assert_kept(&self) {
        for rel in KEPT {
            assert!(self.has(rel), "{rel} was removed");
        }
    }
}

/// What uninstalling lists in this home after a service ran in it, relative to `home`'s
/// folder (the runtime folder as `../hive`).
fn expected(data: bool, accounts: bool) -> Vec<(&'static str, UninstallKind)> {
    use UninstallKind::*;
    let mac = cfg!(target_os = "macos");
    let mut items = Vec::new();
    if data {
        items.extend([
            (".hive/ports.json", Data),
            (".hive/shared", Data),
            (".local/share/hive", Data),
            (".config/hive", Data),
        ]);
        if mac {
            items.extend([
                ("Library/Application Support/com.lucas.hive", Data),
                ("Library/WebKit/com.lucas.hive", Data),
                ("Library/Preferences/com.lucas.hive.plist", Data),
            ]);
        }
        if accounts {
            items.push((".hive/accounts/work", Account));
        }
    }
    items.extend([
        (".hive/bin", Program),
        (".hive/hive-hooks.json", Program),
        (".hive/shell", Program),
    ]);
    items.push((".local/bin/hive", Program));
    if mac {
        items.extend([
            ("Library/Caches/com.lucas.hive", Cache),
            ("Library/Logs/Hive", Cache),
        ]);
    }
    items.push(("../hive", Cache));
    items
}

fn relative(home: &Home, items: &[UninstallItem]) -> Vec<(String, UninstallKind)> {
    let root = home.path("");
    let run = home.env.path("run");
    items
        .iter()
        .map(|item| {
            let path = Path::new(&item.path);
            let rel = match path.strip_prefix(&root) {
                Ok(rel) => rel.display().to_string(),
                Err(_) => format!("../{}", path.strip_prefix(&run).unwrap().display()),
            };
            (rel, item.kind)
        })
        .collect()
}

fn owned(items: Vec<(&str, UninstallKind)>) -> Vec<(String, UninstallKind)> {
    items.into_iter().map(|(p, k)| (p.to_owned(), k)).collect()
}

#[tokio::test]
async fn the_app_uninstalls_hive_after_its_terminals_ended() {
    let home = Home::new();
    let mut daemon = home.env.daemon_with(&mut home.hive());
    let mut app = home.env.connect(Role::App).await;
    app.open_terminal(1, &home.path("src/shop")).await;
    let app_path = Some(home.app().display().to_string());

    let list = |keep_data, accounts, app: &Option<String>| Control::ListUninstall {
        keep_data,
        accounts,
        app: app.clone(),
    };
    for (keep_data, accounts) in [(false, false), (false, true), (true, true)] {
        app.send(0, list(keep_data, accounts, &None)).await;
        let Control::UninstallList {
            keep_data: kept,
            accounts: asked,
            items,
            account_folders,
        } = app.control().await.1
        else {
            panic!("expected the list");
        };
        assert_eq!((kept, asked), (keep_data, accounts));
        let want = owned(expected(!keep_data, accounts));
        assert_eq!(relative(&home, &items), want, "{keep_data} {accounts}");
        let work = home.path(".hive/accounts/work").display().to_string();
        assert_eq!(account_folders, [work]);
    }
    // The app's own bundle goes last; what is not one is not listed.
    for (bundle, last) in [
        (app_path.clone(), Some(home.app())),
        (Some(home.path("src/shop").display().to_string()), None),
    ] {
        app.send(0, list(false, false, &bundle)).await;
        let Control::UninstallList { items, .. } = app.control().await.1 else {
            panic!("expected the list");
        };
        let app_item = items.iter().find(|i| i.kind == UninstallKind::App);
        assert_eq!(app_item.map(|i| PathBuf::from(&i.path)), last);
        assert_eq!(
            items.len(),
            expected(true, false).len() + usize::from(last.is_some())
        );
    }

    // The diagnostics wait for the git resolved at start (D18.3): on macOS `xcrun`, and the
    // `xcodebuild` it may start, run in the background for up to 2 s, so without this they may
    // still be running when the processes are counted below.
    assert_eq!(crate::agents::settled(&mut app).await, []);
    // A hook connection already open: what it sends after the uninstall is not kept.
    let mut hook = home.env.connect(Role::Hook).await;
    app.send(
        0,
        Control::Uninstall {
            keep_data: false,
            accounts: false,
            app: app_path,
        },
    )
    .await;
    // Ignored from now on.
    app.send(0, Control::ListProjects).await;
    let failed = loop {
        match app.control().await.1 {
            Control::Uninstalled { failed } => break failed,
            Control::TerminalExited { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(failed, []);
    let window = hive_protocol::SessionWindow {
        used_percentage: 7,
        resets_at: u64::MAX / 2,
    };
    let report = Control::StatuslineUsage {
        claude_dir: home.path(".claude").display().to_string(),
        usage: window,
        week: None,
    };
    hook.send(0, report).await;
    // Its connection ends without a word, and nothing came back.
    assert!(hook.next().await.is_none());
    assert!(!home.has(".hive/usage.json"));
    // Its terminals ended before anything was removed; only the service is left.
    let pid = daemon.0.id() as i32;
    let left: Vec<_> = home
        .env
        .processes()
        .into_iter()
        .filter(|p| p.pid != pid)
        .collect();
    assert!(left.is_empty(), "{left:?}");
    for (rel, _) in expected(true, false) {
        assert!(!home.has(rel), "{rel} is still there");
    }
    assert!(!home.app().exists());
    home.assert_kept();
    // The account folder Hive made stays, and so Hive's folder.
    assert!(home.has(".hive/accounts/work"));
    #[cfg(target_os = "macos")]
    assert_eq!(home.trashed(), expected(true, false).len() + 1);
    // The service ends once the app is gone.
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[test]
fn hive_uninstall_lists_asks_and_removes() {
    let home = Home::new();
    // A service makes Hive's folder as it is, then ends.
    crate::common::stop(home.env.daemon_with(&mut home.hive()));

    // Refused while a service runs.
    let daemon = home.env.daemon_with(&mut home.hive());
    let out = home.uninstall(&["--yes"], "");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Hive is running: quit it first"), "{err}");
    crate::common::stop(daemon);

    // Anything but yes removes nothing.
    let out = home.uninstall(&[], "n\n");
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(&format!("  {}\n", home.path(".hive/bin").display())),
        "{text}"
    );
    assert!(
        text.contains(&format!("  {}\n", home.app().display())),
        "{text}"
    );
    assert!(
        text.ends_with("Uninstall Hive? [y/N] Nothing removed.\n"),
        "{text}"
    );
    for (rel, _) in expected(true, false) {
        assert!(home.has(rel), "{rel} was removed");
    }

    // Keeping the data.
    let out = home.uninstall(&["--keep-data"], "y\n");
    assert!(out.status.success(), "{out:?}");
    for (rel, _) in expected(false, false) {
        assert!(!home.has(rel), "{rel} is still there");
    }
    for rel in [
        ".hive/ports.json",
        ".hive/shared",
        ".config/hive",
        ".hive/accounts/work",
    ] {
        assert!(home.has(rel), "{rel} was removed");
    }
    assert!(!home.app().exists());

    // Then the rest, the account folders too, and Hive's folder once empty.
    let out = home.uninstall(&["--accounts", "-y"], "");
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).ends_with("Hive is uninstalled.\n"));
    for (rel, _) in expected(true, true) {
        assert!(!home.has(rel), "{rel} is still there");
    }
    assert!(!home.has(".hive"));
    home.assert_kept();
    #[cfg(target_os = "macos")]
    assert_eq!(home.trashed(), expected(true, true).len() + 1);
    let out = home.uninstall(&["--yes"], "");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(text, "Nothing of Hive's is left to remove.\n");
}

#[test]
fn hive_uninstall_needs_a_home_and_one_choice_of_data() {
    let home = Home::new();
    let both = home.uninstall(&["--keep-data", "--accounts"], "");
    assert!(!both.status.success());
    assert!(String::from_utf8_lossy(&both.stderr).contains("cannot be used with"));
    let mut hive = home.hive();
    let out = hive
        .env_remove("HOME")
        .args(["uninstall", "--yes"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("HOME is not set"));
    home.assert_kept();
    assert!(home.has(".hive/ports.json"));
}
