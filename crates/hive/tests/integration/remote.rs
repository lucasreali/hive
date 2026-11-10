//! A remote Linux service (26.14, `hive daemon --remote`): its terminals run the user's `$SHELL`
//! as a login shell with Hive's bin dir first, and its own `PATH` is that shell's. Each shell is
//! the test's own, reading only the files written here: the human's config never loads.

use std::path::{Path, PathBuf};

use hive_protocol::{Control, Role};

use crate::common::{Daemon, wait_until};
use crate::worktree::Repo;

/// Writes an executable `sh` script `name` in `dir`.
fn script(dir: &Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let file = dir.join(name);
    std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The folder of the first `name` on the tests' `PATH`.
fn dir_of(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .find(|dir| dir.join(name).is_file())
        .expect("the program is on PATH")
}

/// Starts a remote service whose `$SHELL` is `shell` (unset when none), its login file `file`
/// (under the test's folder) holding what `first` writes to put a folder first on `PATH`.
/// That folder has a `claude` and a `git` that notes each run, then runs the real one.
async fn login_shell_runs_with_hive_first(shell: Option<&str>, file: &str, first: &str) {
    let repo = Repo::new();
    let env = &repo.env;
    let login = env.path("login");
    std::fs::create_dir(&login).unwrap();
    script(&login, "claude", "exit 0");
    let runs = env.path("git.log");
    let git = dir_of("git");
    let noted = format!(
        "echo \"$*\" >> '{}'\nexec '{}' \"$@\"",
        runs.display(),
        git.join("git").display()
    );
    script(&login, "git", &noted);
    // Ubuntu's `/etc/zsh/zshrc` runs `compinit`, which on CI's runner asks about insecure
    // folders and eats the typed command; a user's `.zshenv` may turn it off.
    std::fs::write(env.path("home/.zshenv"), "skip_global_compinit=1\n").unwrap();
    let login_file = env.path(file);
    std::fs::create_dir_all(login_file.parent().unwrap()).unwrap();
    let line = first.replace("{login}", &login.display().to_string());
    std::fs::write(&login_file, line).unwrap();
    // sshd's minimal `PATH`, with git and the shell (found by name).
    let mut dirs = vec![git, "/usr/bin".into(), "/bin".into()];
    dirs.extend(shell.filter(|shell| !shell.contains('/')).map(dir_of));
    let mut hive = env.hive();
    hive.env("PATH", std::env::join_paths(dirs).unwrap())
        .args(["daemon", "--remote"])
        .stdin(std::process::Stdio::null());
    match shell {
        Some(shell) => hive.env("SHELL", shell),
        None => hive.env_remove("SHELL"),
    };
    let mut daemon = Daemon(hive.spawn().unwrap());
    wait_until(|| std::os::unix::net::UnixStream::connect(env.socket()).is_ok());
    let mut app = env.connect(Role::App).await;

    // The wrapper's `claude` comes first, though the login file put another before it.
    app.open_terminal(1, &repo.root).await;
    app.input(1, "echo \"claude=$(command -v claude).\"\r")
        .await;
    let wrapper = env.path("data/hive/bin/claude");
    app.output_until(1, &format!("claude={}.", wrapper.display()))
        .await;

    // Once asked (by diagnostics), the service's git is the login shell's.
    app.send(0, Control::GetDiagnostics).await;
    let (_, diagnostics) = app.control().await;
    assert!(
        matches!(diagnostics, Control::Diagnostics { .. }),
        "{diagnostics:?}"
    );
    let add = Control::AddProject {
        path: repo.root.display().to_string(),
    };
    app.send(0, add).await;
    wait_until(|| std::fs::read_to_string(&runs).is_ok_and(|runs| !runs.is_empty()));
    drop(app);
    assert!(daemon.wait_exit().success(), "{shell:?}");
}

#[tokio::test]
async fn bash_runs_as_a_login_shell() {
    let profile = "export PATH='{login}':$PATH\n";
    login_shell_runs_with_hive_first(Some("bash"), "home/.bash_profile", profile).await;
}

#[tokio::test]
async fn zsh_runs_as_a_login_shell() {
    let profile = "export PATH='{login}':$PATH\n";
    login_shell_runs_with_hive_first(Some("zsh"), "home/.zprofile", profile).await;
}

#[tokio::test]
async fn fish_runs_as_a_login_shell() {
    let config = "set -gx PATH '{login}' $PATH\n";
    login_shell_runs_with_hive_first(Some("fish"), "config/fish/config.fish", config).await;
}

/// Without a `$SHELL` that can run: `/bin/sh -l`, which reads `~/.profile`.
#[tokio::test]
async fn sh_runs_without_a_usable_shell() {
    let profile = "export PATH='{login}':$PATH\n";
    login_shell_runs_with_hive_first(None, "home/.profile", profile).await;
    let missing = Some("/nonexistent/zsh");
    login_shell_runs_with_hive_first(missing, "home/.profile", profile).await;
}
