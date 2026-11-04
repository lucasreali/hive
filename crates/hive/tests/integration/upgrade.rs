//! An upgrade from v0.23.0's layout (`~/.local/share/hive`, `~/.config/hive`) to Hive's folder,
//! `~/.hive` (22.1).

use std::os::unix::fs::PermissionsExt;

use hive_protocol::{Control, Role, Settings};

use crate::common::{DISTRO, Env, stop};

/// v0.23.0's files in `env`'s home, as the app left them; the new app installed its `hive`.
fn old_layout(env: &Env) {
    let write = |sub: &str, text: &str| {
        let path = env.path(sub);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "home/.config/hive/settings.json",
        r#"{"terminal": {"font_size": 20}}"#,
    );
    write("home/.local/share/hive/ports.json", "{}");
    write("home/.local/share/hive/offered-pulls.json", "{}");
    write("home/.local/share/hive/bin/hive", "old hive");
    write("home/.local/share/hive/shared/skills/a/SKILL.md", "skill");
    let link = env.path("home/.local/bin/hive");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(env.path("home/.local/share/hive/bin/hive"), &link).unwrap();
}

/// `hive` with neither `HIVE_HOME` nor the XDG data and config folders: the home's.
fn hive(env: &Env) -> std::process::Command {
    let mut hive = env.hive();
    for var in ["HIVE_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"] {
        hive.env_remove(var);
    }
    hive
}

async fn welcome(env: &Env) -> (crate::common::Conn, Control) {
    let mut app = env.raw().await;
    app.send(0, Control::hello(Role::App, hive::VERSION)).await;
    let (_, welcome) = app.control().await;
    (app, welcome)
}

#[tokio::test]
async fn an_upgrade_moves_the_old_files_into_the_hive_folder() {
    let env = Env::new();
    old_layout(&env);
    let installed = env.path("home/.hive/bin/hive");
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::write(&installed, "new hive").unwrap();
    // As the app's bridge makes it.
    let private = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(env.path("home/.hive"), private).unwrap();
    let daemon = env.daemon_with(&mut hive(&env));
    let (mut app, welcome) = welcome(&env).await;
    let expected = Control::Welcome {
        version: hive::VERSION.into(),
        distro: Some(DISTRO.into()),
        warning: None,
    };
    assert_eq!(welcome, expected);
    // The settings the service reads are the old ones, now in `~/.hive`.
    let mut settings = Settings::default();
    settings.terminal.font_size = 20;
    assert_eq!(app.control().await, (0, Control::Settings { settings }));
    let home = env.path("home/.hive");
    let mode = std::fs::metadata(&home).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    let read = |sub: &str| std::fs::read_to_string(home.join(sub)).unwrap();
    assert_eq!(read("ports.json"), "{}");
    assert_eq!(read("offered-pulls.json"), "{}");
    assert_eq!(read("shared/skills/a/SKILL.md"), "skill");
    assert!(read("settings.json").contains("20"));
    // The app's `hive` is kept; the wrapper and its hooks settings point into `~/.hive`.
    assert_eq!(read("bin/hive"), "new hive");
    let wrapper = read("bin/claude");
    assert!(wrapper.contains(&format!("'{}'", home.join("hive-hooks.json").display())));
    assert!(wrapper.contains(&format!("'{}'", home.join("shared").display())));
    // The shortcut follows; the old folders are left, holding only what was in the way.
    let link = std::fs::read_link(env.path("home/.local/bin/hive")).unwrap();
    assert_eq!(link, home.join("bin/hive"));
    let old: Vec<_> = std::fs::read_dir(env.path("home/.local/share/hive"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(old, ["bin"]);
    let config = std::fs::read_dir(env.path("home/.config/hive")).unwrap();
    assert_eq!(config.count(), 0);
    drop(app);
    stop(daemon);
}

#[tokio::test]
async fn files_that_cannot_move_keep_the_old_folders_and_the_status_bar_says_so() {
    let env = Env::new();
    old_layout(&env);
    // A file where Hive's folder should be.
    std::fs::write(env.path("home/.hive"), "").unwrap();
    let daemon = env.daemon_with(&mut hive(&env));
    let (mut app, welcome) = welcome(&env).await;
    let Control::Welcome {
        warning: Some(warning),
        ..
    } = welcome
    else {
        panic!("{welcome:?}");
    };
    assert!(
        warning.starts_with("cannot move Hive's files into"),
        "{warning}"
    );
    let mut settings = Settings::default();
    settings.terminal.font_size = 20;
    assert_eq!(app.control().await, (0, Control::Settings { settings }));
    // The wrapper is written in the old place.
    let wrapper = env.path("home/.local/share/hive/bin/claude");
    assert!(wrapper.is_file());
    drop(app);
    stop(daemon);
}
