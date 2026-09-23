use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::json;

use crate::common::Env;

/// A throwaway repository with one commit on `main`; git never sees the real user config.
struct Repo {
    env: Env,
    root: PathBuf,
}

fn isolate<'a>(cmd: &'a mut Command, env: &Env) -> &'a mut Command {
    cmd.env("HOME", env.path("home"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Asserts a failure that printed nothing on stdout and `message` on stderr.
fn assert_fails(out: &Output, message: &str) {
    assert!(!out.status.success(), "{}", stdout(out));
    assert_eq!(stdout(out), "");
    assert!(stderr(out).contains(message), "{}", stderr(out));
}

impl Repo {
    fn new() -> Self {
        let env = Env::new();
        let root = env.path("home/repo");
        std::fs::create_dir(&root).unwrap();
        let repo = Self {
            root: root.canonicalize().unwrap(),
            env,
        };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.commit("README", "hello");
        repo
    }

    fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        let out = isolate(&mut Command::new("git"), &self.env)
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", stderr(&out));
        stdout(&out).trim().to_owned()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.root, args)
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn commit(&self, rel: &str, content: &str) {
        self.write(rel, content);
        self.git(&["add", "-f", rel]);
        self.git(&["commit", "-q", "-m", rel]);
    }

    fn hive_cmd(&self, dir: &Path, args: &[&str]) -> Command {
        let mut cmd = self.env.hive();
        isolate(&mut cmd, &self.env)
            .current_dir(dir)
            .arg("worktree")
            .args(args);
        cmd
    }

    fn hive_in(&self, dir: &Path, args: &[&str]) -> Output {
        self.hive_cmd(dir, args).output().unwrap()
    }

    fn hive(&self, args: &[&str]) -> Output {
        self.hive_in(&self.root, args)
    }

    fn hook(&self, event: &str, input: &[u8]) -> Output {
        let mut child = self
            .hive_cmd(&self.env.path("home"), &[event])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The hook may refuse before reading everything.
        let _ = child.stdin.take().unwrap().write_all(input);
        child.wait_with_output().unwrap()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(".claude/worktrees").join(name)
    }

    /// Creates `name` and checks that stdout is exactly its path.
    fn create(&self, args: &[&str]) -> PathBuf {
        let out = self.hive(&[&["create"], args].concat());
        assert!(out.status.success(), "{}", stderr(&out));
        let path = self.path(args[0]);
        assert_eq!(stdout(&out), format!("{}\n", path.display()));
        assert!(path.join(".git").is_file());
        path
    }
}

#[test]
fn create_list_and_remove() {
    let repo = Repo::new();
    std::fs::create_dir(repo.root.join("sub")).unwrap();
    let out = repo.hive_in(&repo.root.join("sub"), &["create", "feat-1"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let feat = repo.path("feat-1");
    assert_eq!(stdout(&out), format!("{}\n", feat.display()));
    assert_eq!(
        repo.git_in(&feat, &["branch", "--show-current"]),
        "worktree-feat-1"
    );
    assert_eq!(
        repo.git_in(&feat, &["rev-parse", "HEAD"]),
        repo.git(&["rev-parse", "main"])
    );
    assert_eq!(stderr(&out), "");

    // From inside a linked worktree the new one still goes under the main repository.
    let out = repo.hive_in(&feat, &["create", "second"]);
    assert_eq!(stdout(&out), format!("{}\n", repo.path("second").display()));

    let list = repo.hive_in(&feat, &["list"]);
    assert!(list.status.success());
    assert_eq!(
        stdout(&list),
        format!(
            "{}\tmain\n{}\tworktree-feat-1\n{}\tworktree-second\n",
            repo.root.display(),
            feat.display(),
            repo.path("second").display()
        )
    );

    let out = repo.hive(&["remove", "feat-1"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(!feat.exists());
    // The branch is kept: removing a worktree never loses commits.
    assert_eq!(
        repo.git(&["branch", "--list", "worktree-feat-1"]),
        "worktree-feat-1"
    );
    assert!(!stdout(&repo.hive(&["list"])).contains("feat-1"));
}

#[test]
fn detached_worktrees_are_listed() {
    let repo = Repo::new();
    let dir = repo.root.join("elsewhere");
    repo.git(&["worktree", "add", "-q", "--detach", dir.to_str().unwrap()]);
    let list = stdout(&repo.hive(&["list"]));
    assert!(
        list.ends_with(&format!("{}\t(detached)\n", dir.display())),
        "{list}"
    );
}

#[test]
fn remove_refuses_a_worktree_with_changes() {
    let repo = Repo::new();
    let path = repo.create(&["dirty"]);
    std::fs::write(path.join("README"), "changed").unwrap();
    assert_fails(&repo.hive(&["remove", "dirty"]), "git worktree remove");
    assert!(path.join("README").exists());
    assert_fails(&repo.hive(&["remove", "missing"]), "is not a working tree");
    assert_fails(&repo.hive(&["remove", "Bad"]), "invalid worktree name");
}

#[test]
fn base_can_be_a_local_or_remote_branch() {
    let repo = Repo::new();
    let origin = repo.env.path("home/origin.git");
    repo.git(&["init", "-q", "--bare", origin.to_str().unwrap()]);
    repo.git(&["remote", "add", "origin", origin.to_str().unwrap()]);
    repo.git(&["push", "-q", "origin", "main"]);
    let pushed = repo.git(&["rev-parse", "main"]);
    repo.commit("local", "only here");
    repo.git(&["checkout", "-q", "-b", "later"]);
    repo.commit("later", "after");

    let remote = repo.create(&["from-remote", "--base", "origin/main"]);
    assert_eq!(repo.git_in(&remote, &["rev-parse", "HEAD"]), pushed);
    let local = repo.create(&["from-local", "--base", "main"]);
    assert_eq!(
        repo.git_in(&local, &["rev-parse", "HEAD"]),
        repo.git(&["rev-parse", "main"])
    );
    let head = repo.create(&["from-head"]);
    assert_eq!(
        repo.git_in(&head, &["rev-parse", "HEAD"]),
        repo.git(&["rev-parse", "later"])
    );

    assert_fails(
        &repo.hive(&["create", "nope", "--base", "origin/missing"]),
        "git worktree add",
    );
    assert!(!repo.path("nope").exists());
    assert_fails(
        &repo.hive(&["create", "opt", "--base=--orphan"]),
        "invalid base branch \"--orphan\"",
    );
}

#[test]
fn invalid_and_existing_names_are_refused() {
    let repo = Repo::new();
    for name in ["Feat", "-x", ".x", "a/b", ""] {
        assert_fails(&repo.hive(&["create", "--", name]), "invalid worktree name");
    }
    repo.create(&["dup"]);
    assert_fails(
        &repo.hive(&["create", "dup"]),
        "worktree \"dup\" already exists",
    );
    repo.git(&["branch", "worktree-taken"]);
    assert_fails(
        &repo.hive(&["create", "taken"]),
        "a branch named 'worktree-taken' already exists",
    );
    assert!(!repo.path("taken").exists());
}

#[test]
fn unreadable_worktreeinclude_fails_before_creating() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    repo.write(".worktreeinclude", ".env\n");
    let include = repo.root.join(".worktreeinclude");
    std::fs::set_permissions(&include, std::fs::Permissions::from_mode(0o000)).unwrap();
    assert_fails(&repo.hive(&["create", "x"]), "git ls-files");
    assert!(!repo.path("x").exists());
    assert_eq!(repo.git(&["branch", "--list", "worktree-x"]), "");
}

#[test]
fn worktreeinclude_copies_only_gitignored_matches() {
    let repo = Repo::new();
    repo.commit(".gitignore", ".env\nconfig/\n*.log\nlink\nsecrets/\n");
    repo.commit(
        ".worktreeinclude",
        ".env\nconfig/\ntracked.txt\nplain.txt\nlink\nsecrets/\n",
    );
    repo.commit("tracked.txt", "tracked");
    repo.write(".env", "SECRET=1");
    repo.write("config/deep/app.json", "{}");
    repo.write("plain.txt", "untracked but not ignored");
    repo.write("other.log", "ignored but not included");
    std::os::unix::fs::symlink(repo.env.path("home"), repo.root.join("link")).unwrap();

    let path = repo.create(&["inc"]);
    assert_eq!(
        std::fs::read_to_string(path.join(".env")).unwrap(),
        "SECRET=1"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("config/deep/app.json")).unwrap(),
        "{}"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
        "tracked"
    );
    assert!(!path.join("plain.txt").exists());
    assert!(!path.join("other.log").exists());
    assert!(!path.join("link").exists());
}

#[test]
fn worktreeinclude_never_writes_through_the_base_branch() {
    let repo = Repo::new();
    let outside = repo.env.path("home/outside");
    std::fs::create_dir(&outside).unwrap();
    // A base branch that tracks `.env` and has `secrets` as a symlink out of the repository.
    repo.git(&["checkout", "-q", "-b", "evil"]);
    repo.commit(".env", "tracked env");
    std::os::unix::fs::symlink(&outside, repo.root.join("secrets")).unwrap();
    repo.git(&["add", "secrets"]);
    repo.git(&["commit", "-q", "-m", "symlink"]);
    repo.git(&["checkout", "-q", "main"]);
    repo.commit(".gitignore", ".env\nsecrets/\n");
    repo.commit(".worktreeinclude", ".env\nsecrets/\n");
    repo.write(".env", "local env");
    repo.write("secrets/key", "k");

    let path = repo.create(&["safe", "--base", "evil"]);
    assert_eq!(
        std::fs::read_to_string(path.join(".env")).unwrap(),
        "tracked env"
    );
    assert!(!outside.join("key").exists());
}

#[test]
fn own_worktree_create_hook_is_warned_about() {
    let repo = Repo::new();
    // Real settings files run to several KiB; the size limit must not skip them.
    let hook = json!({
        "permissions": {"allow": vec!["Bash(true)"; 1000]},
        "hooks": {"WorktreeCreate": [{"hooks": [{"type": "command", "command": "x"}]}]},
    });
    assert!(hook.to_string().len() > 10 * 1024);
    repo.write(".claude/settings.json", &hook.to_string());
    repo.write(".claude/settings.local.json", "{}");
    let out = repo.hive(&["create", "warned"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), format!("{}\n", repo.path("warned").display()));
    assert_eq!(
        stderr(&out),
        "hive: warning: .claude/settings.json defines its own WorktreeCreate hook; it will compete with Hive's\n"
    );
    repo.write(".claude/settings.local.json", &hook.to_string());
    let out = repo.hook(
        "hook-create",
        json!({"name": "warned-too", "cwd": repo.root})
            .to_string()
            .as_bytes(),
    );
    assert!(out.status.success());
    assert!(stderr(&out).contains(".claude/settings.local.json defines"));
}

#[test]
fn hook_create_prints_only_the_path() {
    let repo = Repo::new();
    std::fs::create_dir(repo.root.join("sub")).unwrap();
    let input = json!({
        "session_id": "abc",
        "hook_event_name": "WorktreeCreate",
        "cwd": repo.root.join("sub"),
        "name": "bold-oak-a3f2",
    });
    let out = repo.hook("hook-create", input.to_string().as_bytes());
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!("{}\n", repo.path("bold-oak-a3f2").display())
    );
    assert_eq!(
        repo.git_in(&repo.path("bold-oak-a3f2"), &["branch", "--show-current"]),
        "worktree-bold-oak-a3f2"
    );
}

#[test]
fn hook_create_failures_print_nothing() {
    let repo = Repo::new();
    let payload = |name: &str| json!({"name": name, "cwd": repo.root}).to_string();
    let cases: [(Vec<u8>, &str); 6] = [
        (b"not json".to_vec(), "invalid hook input"),
        (
            json!({"cwd": repo.root}).to_string().into_bytes(),
            "no string field \"name\"",
        ),
        (
            json!({"name": "x"}).to_string().into_bytes(),
            "no string field \"cwd\"",
        ),
        (payload("Bad").into_bytes(), "invalid worktree name"),
        (vec![b' '; 64 * 1024 + 1], "input larger than 65536 bytes"),
        (
            json!({"name": "x", "cwd": repo.env.path("home")})
                .to_string()
                .into_bytes(),
            "not a git repository",
        ),
    ];
    for (input, message) in cases {
        assert_fails(&repo.hook("hook-create", &input), message);
    }
    assert!(
        repo.hook("hook-create", payload("once").as_bytes())
            .status
            .success()
    );
    assert_fails(
        &repo.hook("hook-create", payload("once").as_bytes()),
        "already exists",
    );
}

#[test]
fn hook_remove_removes_only_claude_worktrees() {
    let repo = Repo::new();
    let path = repo.create(&["gone"]);
    let input = |path: &Path| json!({"worktree_path": path}).to_string().into_bytes();
    let out = repo.hook("hook-remove", &input(&path));
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(!path.exists());

    let kept = repo.create(&["kept"]);
    let elsewhere = repo.root.join("elsewhere");
    repo.git(&[
        "worktree",
        "add",
        "-q",
        "--detach",
        elsewhere.to_str().unwrap(),
    ]);
    std::fs::create_dir(kept.join("sub")).unwrap();
    for outside in [repo.root.clone(), kept.join("sub"), elsewhere] {
        assert_fails(
            &repo.hook("hook-remove", &input(&outside)),
            "refusing to remove",
        );
        assert!(outside.exists());
    }
    assert_fails(
        &repo.hook("hook-remove", b"{}"),
        "no string field \"worktree_path\"",
    );
    assert_fails(
        &repo.hook("hook-remove", &input(&repo.env.path("home/missing"))),
        "No such file",
    );
    assert!(kept.exists());
}

#[test]
fn bare_repositories_have_no_main_worktree() {
    let repo = Repo::new();
    let bare = repo.env.path("home/bare.git");
    repo.git(&["init", "-q", "--bare", bare.to_str().unwrap()]);
    assert_fails(
        &repo.hive_in(&bare, &["create", "x"]),
        "a bare repository has no main worktree",
    );
    let list = repo.hive_in(&bare, &["list"]);
    assert_eq!(
        stdout(&list),
        format!("{}\t(bare)\n", bare.canonicalize().unwrap().display())
    );
}

#[test]
fn missing_git_is_reported() {
    let repo = Repo::new();
    let out = repo
        .hive_cmd(&repo.root, &["list"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_fails(&out, "cannot run git");
}
