use super::*;
use crate::health::tests::{commit as commit_file, run};
use crate::procs::Source;

/// A followed repository with one commit on `main`, in a temporary folder.
struct Repo {
    tmp: tempfile::TempDir,
    projects: Projects,
    id: String,
}

impl Repo {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        run(&root, &["init", "-q", "-b", "main"]);
        commit_file(&root, "a");
        let data = tmp.path().join("data");
        let projects = Projects::load(data.join("spaces.json"), &data.join("projects.json"));
        let id = projects.add(&root.display().to_string()).unwrap().id;
        Self { tmp, projects, id }
    }

    fn root(&self) -> &Path {
        Path::new(&self.id)
    }

    fn project(&self) -> Project {
        self.projects.followed(&self.id).unwrap()
    }

    fn worktree(&self, path: &str) -> Worktree {
        let project = self.project();
        project
            .worktrees
            .into_iter()
            .find(|w| w.path == path)
            .unwrap()
    }

    /// `git <args>` in `dir`, its output trimmed.
    fn git(&self, dir: &str, args: &[&str]) -> String {
        text(Path::new(dir), args).unwrap()
    }

    /// The linked worktree `name` on its own branch `worktree-<name>`.
    fn linked(&self, name: &str) -> String {
        let (_, created) = self.projects.create_worktree(&self.id, name, None).unwrap();
        created.path.display().to_string()
    }

    /// Creates the review of `path` and answers its path.
    fn review(&self, path: &str) -> String {
        let (project, created) = create(&self.projects, path).unwrap();
        let review = created.path.display().to_string();
        let w = project.worktrees.iter().find(|w| w.path == review).unwrap();
        assert_eq!(w.review_of.as_deref(), Some(path));
        review
    }

    fn no_procs(&self) -> PathBuf {
        let dir = self.tmp.path().join("proc");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

/// "Bring back" of the review `path` as its plan shows now, with `tip` as the review's tip.
fn bring(repo: &Repo, path: &str, tip: &str) -> io::Result<(Project, u64, bool)> {
    let now = plan(&repo.projects, path).ok();
    let (into, base) = now.map(|p| (p.into, p.base)).unwrap_or_default();
    let shown = Shown {
        into: &into,
        base: &base,
        tip,
    };
    bring_back(&repo.projects, path, shown, &env())
}

/// The space's identity, and no global git config (a signing setting of the user's).
fn env() -> Vec<(&'static str, String)> {
    vec![
        ("GIT_AUTHOR_NAME", "Rev".into()),
        ("GIT_AUTHOR_EMAIL", "rev@x".into()),
        ("GIT_COMMITTER_NAME", "Rev".into()),
        ("GIT_COMMITTER_EMAIL", "rev@x".into()),
        ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM", "1".into()),
    ]
}

#[test]
fn a_reviews_name_is_a_worktree_and_branch_name() {
    assert_eq!(review_name("fix-login"), "review-fix-login");
    assert_eq!(
        review_name("Feature/Login.Page"),
        "review-feature-login-page"
    );
    assert_eq!(review_name("a--b_c"), "review-a-b_c");
    assert_eq!(review_name("x-"), "review-x");
    assert_eq!(review_name("é"), "review");
    let long = review_name(&"a".repeat(100));
    assert_eq!(long, format!("review-{}", "a".repeat(NAME_LIMIT)));
    for name in ["x", "Feature/Login.Page", "é", &"a".repeat(100)] {
        assert!(worktree::validate_name(&review_name(name)).is_ok());
    }
}

#[test]
fn a_log_is_parsed_with_subjects_cut_on_a_character() {
    let long = format!("a{}", "é".repeat(150));
    let out = format!("1111\x1ffix\x002222\x1f{long}\x00broken\x00");
    let commits = parse_log(out.as_bytes());
    assert_eq!(commits.len(), 2);
    assert_eq!(
        commits[0],
        ReviewCommit {
            id: "1111".into(),
            summary: "fix".into()
        }
    );
    assert_eq!(commits[1].id, "2222");
    assert_eq!(commits[1].summary.len(), SUMMARY_LIMIT - 1);
    assert!(long.starts_with(&commits[1].summary));
    let exact = format!("3\x1f{}", "b".repeat(SUMMARY_LIMIT + 5));
    assert_eq!(parse_log(exact.as_bytes())[0].summary.len(), SUMMARY_LIMIT);
}

fn worktree_at(path: &str) -> Worktree {
    Worktree {
        id: path.into(),
        name: path.into(),
        path: path.into(),
        branch: None,
        main: false,
        claude: true,
        status: None,
        review_of: None,
    }
}

#[test]
fn links_are_saved_privately_marked_pruned_and_renamed() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("data/reviews.json");
    let reviews = Reviews::load(file.clone());
    assert!(!reviews.full());
    reviews.link("/r", "/r/v", "/r/w").unwrap();
    // Linked again, the same review keeps one link.
    reviews.link("/r", "/r/v", "/r/w").unwrap();
    reviews.link("/q", "/q/v", "/q/w").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let reviews = Reviews::load(file.clone());
    assert_eq!(reviews.links().len(), 2);

    let mut project = Project {
        id: "/r".into(),
        name: "r".into(),
        path: "/r".into(),
        worktrees: ["/r", "/r/w", "/r/v"].map(worktree_at).to_vec(),
        error: None,
        group: false,
        folder: false,
        parent: None,
    };
    reviews.mark(&mut project);
    let marks: Vec<_> = project
        .worktrees
        .iter()
        .map(|w| w.review_of.clone())
        .collect();
    assert_eq!(marks, [None, None, Some("/r/w".into())]);
    // A reviewed worktree missing from the list marks nothing.
    project.worktrees.remove(1);
    reviews.mark(&mut project);
    assert_eq!(project.worktrees[1].review_of, None);

    reviews.rename("/r/w", "/r/x");
    reviews.rename("/r/nothing", "/r/y");
    assert_eq!(Reviews::load(file.clone()).links()[0].reviewed, "/r/x");
    // Another project's list prunes nothing of this one.
    reviews.prune("/r", &["/r/v", "/r/x"].map(worktree_at));
    reviews.prune("/q", &["/q/w"].map(worktree_at));
    let kept = Reviews::load(file.clone());
    assert_eq!(kept.links().len(), 1);
    assert_eq!(kept.links()[0].project, "/r");
    reviews.prune("/r", &["/r/x"].map(worktree_at));
    assert!(Reviews::load(file).links().is_empty());
}

#[test]
fn a_links_file_that_cannot_be_read_or_saved_is_no_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("reviews.json");
    std::fs::write(&file, "not json").unwrap();
    assert!(Reviews::load(file.clone()).links().is_empty());
    std::fs::write(&file, vec![b' '; FILE_LIMIT as usize + 1]).unwrap();
    assert!(Reviews::load(file).links().is_empty());

    // Its folder is a file: nothing saves, the links are kept all the same.
    std::fs::write(tmp.path().join("plain"), "").unwrap();
    let reviews = Reviews::load(tmp.path().join("plain/reviews.json"));
    assert!(reviews.link("/r", "/r/v", "/r/w").is_err());
    reviews.rename("/r/w", "/r/x");
    assert_eq!(reviews.links()[0].reviewed, "/r/x");
    reviews.prune("/r", &[]);
    assert!(reviews.links().is_empty());
}

#[test]
fn links_are_capped() {
    let tmp = tempfile::tempdir().unwrap();
    let reviews = Reviews::load(tmp.path().join("reviews.json"));
    for i in 0..LINKS_LIMIT {
        reviews.link("/r", &format!("/r/{i}"), "/r").unwrap();
    }
    assert!(reviews.full());
    let err = reviews.link("/r", "/r/x", "/r").unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("Hive keeps at most {LINKS_LIMIT} reviews: discard some first")
    );
    // The same review linked again is no new one.
    reviews.link("/r", "/r/0", "/r/1").unwrap();
}

#[test]
fn a_review_starts_from_the_reviewed_branchs_tip() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    commit_file(Path::new(&w), "b");
    // Uncommitted work stays where it is.
    std::fs::write(Path::new(&w).join("c"), "c").unwrap();
    let review = repo.review(&w);
    let folder = worktree::worktrees_dir(repo.root()).join("review-fix");
    assert_eq!(review, folder.display().to_string());
    let v = repo.worktree(&review);
    assert_eq!(v.branch.as_deref(), Some("review-fix"));
    assert!(v.claude);
    assert_eq!(
        repo.git(&review, &["rev-parse", "HEAD"]),
        repo.git(&w, &["rev-parse", "HEAD"])
    );
    assert!(!Path::new(&review).join("c").exists());

    // A second review gets the next name; so does one whose branch alone is taken.
    let again = repo.review(&w);
    assert!(again.ends_with("review-fix-2"), "{again}");
    run(repo.root(), &["branch", "review-fix-3"]);
    assert!(repo.review(&w).ends_with("review-fix-4"));

    // The main worktree is reviewed by its branch's name.
    let main = repo.review(&repo.id);
    assert!(main.ends_with("review-main"), "{main}");
    // The links are kept in Hive's folder.
    let saved = std::fs::read_to_string(repo.tmp.path().join("data/reviews.json")).unwrap();
    assert!(saved.contains(&main));
}

#[test]
fn a_review_needs_a_branch_a_followed_worktree_a_free_name_and_room() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let refused = |path: &str| create(&repo.projects, path).unwrap_err().to_string();
    assert_eq!(
        refused("/nowhere"),
        "/nowhere is not a worktree of a followed project"
    );
    run(Path::new(&w), &["checkout", "-q", "--detach"]);
    repo.projects.forget();
    assert_eq!(refused(&w), "fix is detached: there is no branch to review");
    run(Path::new(&w), &["checkout", "-q", "worktree-fix"]);
    repo.projects.forget();

    run(repo.root(), &["branch", "review-fix"]);
    for i in 2..=TRIES {
        run(repo.root(), &["branch", &format!("review-fix-{i}")]);
    }
    assert_eq!(
        refused(&w),
        format!("review-fix and {TRIES} more names are taken")
    );

    for i in 0..LINKS_LIMIT {
        let _ = repo.projects.reviews().link("/r", &format!("/r/{i}"), "/r");
    }
    assert_eq!(
        refused(&repo.id),
        format!("Hive keeps at most {LINKS_LIMIT} reviews: discard some first")
    );
}

#[test]
fn a_review_whose_link_cannot_be_saved_says_so() {
    let repo = Repo::new();
    // The links' file is a folder: nothing can replace it.
    std::fs::create_dir_all(repo.tmp.path().join("data/reviews.json/x")).unwrap();
    let (project, created) = create(&repo.projects, &repo.id).unwrap();
    assert_eq!(created.notes.len(), 1);
    assert!(
        created.notes[0].starts_with("the review link could not be saved: "),
        "{:?}",
        created.notes
    );
    // Kept for this run all the same.
    let review = created.path.display().to_string();
    let w = project.worktrees.iter().find(|w| w.path == review).unwrap();
    assert_eq!(w.review_of.as_deref(), Some(repo.id.as_str()));
}

#[test]
fn a_plan_lists_the_commits_that_would_go() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    let refused = |path: &str| plan(&repo.projects, path).unwrap_err().to_string();
    assert_eq!(refused(&w), "fix is not a review");

    let empty = plan(&repo.projects, &review).unwrap();
    assert_eq!((empty.total, empty.commits.len()), (0, 0));
    assert!(empty.fast_forward);

    commit_file(Path::new(&review), "b");
    commit_file(Path::new(&review), "c");
    let tip = repo.git(&review, &["rev-parse", "HEAD"]);
    let plan_now = plan(&repo.projects, &review).unwrap();
    assert_eq!(
        plan_now,
        ReviewPlan {
            reviewed: w.clone(),
            into: "worktree-fix".into(),
            base: repo.git(&w, &["rev-parse", "HEAD"]),
            branch: "review-fix".into(),
            tip: tip.clone(),
            commits: vec![
                ReviewCommit {
                    id: tip,
                    summary: "c".into()
                },
                ReviewCommit {
                    id: repo.git(&review, &["rev-parse", "HEAD~"]),
                    summary: "b".into()
                },
            ],
            total: 2,
            fast_forward: true,
        }
    );
    commit_file(Path::new(&w), "d");
    assert!(!plan(&repo.projects, &review).unwrap().fast_forward);

    // Only so many are listed; all are counted.
    for i in 0..BRING_BACK_SHOWN {
        run(
            Path::new(&review),
            &["commit", "-q", "--allow-empty", "-m", &format!("e{i}")],
        );
    }
    let many = plan(&repo.projects, &review).unwrap();
    assert_eq!(many.commits.len(), BRING_BACK_SHOWN);
    assert_eq!(many.total, BRING_BACK_SHOWN as u64 + 2);

    run(Path::new(&review), &["checkout", "-q", "--detach"]);
    repo.projects.forget();
    assert_eq!(
        refused(&review),
        "review-fix is detached: it is on no branch"
    );
}

#[test]
fn changes_come_back_as_a_fast_forward_when_the_branch_did_not_move() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    commit_file(Path::new(&review), "b");
    let tip = repo.git(&review, &["rev-parse", "HEAD"]);
    let (project, commits, fast_forward) = bring(&repo, &review, &tip).unwrap();
    assert_eq!((commits, fast_forward), (1, true));
    assert_eq!(repo.git(&w, &["rev-parse", "HEAD"]), tip);
    assert!(Path::new(&w).join("b").exists());
    assert!(project.worktrees.iter().any(|x| x.path == review));
    // Brought back, there is nothing more.
    let again = bring(&repo, &review, &tip).unwrap_err();
    assert_eq!(
        again.to_string(),
        "nothing to bring back: every commit of review-fix is on worktree-fix"
    );
}

#[test]
fn changes_come_back_as_a_merge_commit_when_both_moved() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    commit_file(Path::new(&review), "b");
    commit_file(Path::new(&w), "c");
    let tip = repo.git(&review, &["rev-parse", "HEAD"]);
    let before = repo.git(&w, &["rev-parse", "HEAD"]);
    let (_, commits, fast_forward) = bring(&repo, &review, &tip).unwrap();
    assert_eq!((commits, fast_forward), (1, false));
    let parents = repo.git(&w, &["log", "-1", "--format=%P"]);
    assert_eq!(parents, format!("{before} {tip}"));
    assert_eq!(
        repo.git(&w, &["log", "-1", "--format=%s|%an|%ce"]),
        "Merge review review-fix into worktree-fix|Rev|rev@x"
    );
}

#[test]
fn a_conflict_is_aborted_with_gits_message() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    for (dir, text) in [(&review, "theirs"), (&w, "ours")] {
        std::fs::write(Path::new(dir).join("a"), text).unwrap();
        run(Path::new(dir), &["commit", "-q", "-am", text]);
    }
    let tip = repo.git(&review, &["rev-parse", "HEAD"]);
    let before = repo.git(&w, &["rev-parse", "HEAD"]);
    let err = bring(&repo, &review, &tip).unwrap_err();
    let err = err.to_string();
    assert!(
        err.starts_with("git merge stopped; nothing was changed:\n"),
        "{err}"
    );
    assert!(err.contains("CONFLICT"), "{err}");
    assert_eq!(repo.git(&w, &["rev-parse", "HEAD"]), before);
    assert_eq!(repo.git(&w, &["status", "--porcelain"]), "");
    assert_eq!(
        std::fs::read_to_string(Path::new(&w).join("a")).unwrap(),
        "ours"
    );
}

#[test]
fn a_merge_git_refuses_is_its_message() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    commit_file(Path::new(&review), "b");
    commit_file(Path::new(&w), "c");
    let mut shown = plan(&repo.projects, &review).unwrap();
    // A fast-forward that is none: git refuses it, and nothing is merging.
    shown.fast_forward = true;
    let err = merge(Path::new(&w), &shown, &env(), MERGE_TIME)
        .unwrap_err()
        .to_string();
    assert!(
        err.starts_with("git merge stopped; nothing was changed:\n"),
        "{err}"
    );
    assert!(err.contains("fast-forward"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_merge_that_takes_too_long_is_stopped_and_aborted() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    commit_file(Path::new(&review), "b");
    commit_file(Path::new(&w), "c");
    let hook = repo.root().join(".git/hooks/pre-merge-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\nsleep 5\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let before = repo.git(&w, &["rev-parse", "HEAD"]);
    let shown = plan(&repo.projects, &review).unwrap();
    let err = merge(Path::new(&w), &shown, &env(), Duration::from_millis(500)).unwrap_err();
    assert!(err.to_string().contains("took longer than"), "{err}");
    assert_eq!(repo.git(&w, &["rev-parse", "HEAD"]), before);
    assert_eq!(repo.git(&w, &["status", "--porcelain"]), "");

    // Stopped by a hook after the merge, it is done.
    std::fs::remove_file(&hook).unwrap();
    let after = hook.with_file_name("post-merge");
    std::fs::write(&after, "#!/bin/sh\nsleep 5\n").unwrap();
    std::fs::set_permissions(&after, std::fs::Permissions::from_mode(0o755)).unwrap();
    merge(Path::new(&w), &shown, &env(), Duration::from_millis(500)).unwrap();
    assert_eq!(repo.git(&w, &["rev-parse", "HEAD^2"]), shown.tip);
}

#[test]
fn a_merge_git_refuses_leaves_what_was_staged_meanwhile() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    std::fs::write(Path::new(&review).join("a"), "theirs").unwrap();
    run(Path::new(&review), &["commit", "-q", "-am", "theirs"]);
    commit_file(Path::new(&w), "c");
    let shown = plan(&repo.projects, &review).unwrap();
    // Staged after the check for changes: git refuses to merge over it.
    std::fs::write(Path::new(&w).join("a"), "staged").unwrap();
    run(Path::new(&w), &["add", "a"]);
    let err = merge(Path::new(&w), &shown, &env(), MERGE_TIME).unwrap_err();
    assert!(err.to_string().contains("overwritten"), "{err}");
    assert_eq!(repo.git(&w, &["diff", "--cached", "--name-only"]), "a");
    assert_eq!(
        std::fs::read_to_string(Path::new(&w).join("a")).unwrap(),
        "staged"
    );
}

#[test]
fn only_the_end_of_what_git_said_is_kept() {
    assert_eq!(tail("abc", 2), "bc");
    assert_eq!(tail("abc", 5), "abc");
    // Never inside a character.
    assert_eq!(tail("aéb", 2), "b");
}

#[test]
fn an_untracked_file_in_the_way_stops_the_merge_with_gits_word() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    commit_file(Path::new(&review), "u");
    commit_file(Path::new(&w), "c");
    std::fs::write(Path::new(&w).join("u"), "mine").unwrap();
    let tip = repo.git(&review, &["rev-parse", "HEAD"]);
    let before = repo.git(&w, &["rev-parse", "HEAD"]);
    let err = bring(&repo, &review, &tip).unwrap_err();
    assert!(err.to_string().contains("untracked"), "{err}");
    assert_eq!(repo.git(&w, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        std::fs::read_to_string(Path::new(&w).join("u")).unwrap(),
        "mine"
    );
}

#[test]
fn nothing_comes_back_with_changes_on_either_side_or_a_moved_review() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    commit_file(Path::new(&review), "b");
    let tip = repo.git(&review, &["rev-parse", "HEAD"]);
    let refused = |tip: &str| bring(&repo, &review, tip).unwrap_err().to_string();

    assert_eq!(
        refused("0000"),
        "review-fix changed since the commits were shown: look again"
    );
    // The reviewed branch moved, or is another one, since the plan was shown.
    let shown = plan(&repo.projects, &review).unwrap();
    let with = |into: &str, base: &str| {
        let shown = Shown {
            into,
            base,
            tip: &tip,
        };
        bring_back(&repo.projects, &review, shown, &env())
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        with(&shown.into, "0000"),
        "worktree-fix changed since the commits were shown: look again"
    );
    assert_eq!(
        with("other", &shown.base),
        "fix is now on worktree-fix since the commits were shown: look again"
    );
    std::fs::write(Path::new(&review).join("b"), "edited").unwrap();
    assert_eq!(
        refused(&tip),
        "review-fix has uncommitted changes: commit or discard them first"
    );
    run(Path::new(&review), &["checkout", "--", "b"]);
    // An untracked file is no change.
    std::fs::write(Path::new(&review).join("new"), "x").unwrap();
    std::fs::write(Path::new(&w).join("a"), "edited").unwrap();
    assert_eq!(
        refused(&tip),
        "fix has uncommitted changes: commit or discard them first"
    );
    assert_eq!(
        bring(&repo, &w, &tip).unwrap_err().to_string(),
        "fix is not a review"
    );
}

#[test]
fn a_discarded_review_takes_its_branch_only_when_brought_back() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let procs = repo.no_procs();
    let discard = |path: &str, force: bool| {
        discard(&repo.projects, path, force, Source::Dir(&procs), |_| Ok(()))
    };
    let has = |branch: &str| worktree::has_branch(repo.root(), branch);

    // No commit of its own: the branch goes.
    let review = repo.review(&w);
    let project = discard(&review, false).unwrap();
    assert!(!project.worktrees.iter().any(|x| x.path == review));
    assert!(!Path::new(&review).exists());
    assert!(!has("review-fix"));

    // A commit not brought back: the branch stays.
    let review = repo.review(&w);
    commit_file(Path::new(&review), "x");
    discard(&review, false).unwrap();
    assert!(has("review-fix"));
    run(repo.root(), &["branch", "-D", "review-fix"]);

    // Brought back: it goes.
    let review = repo.review(&w);
    commit_file(Path::new(&review), "b");
    let tip = repo.git(&review, &["rev-parse", "HEAD"]);
    bring(&repo, &review, &tip).unwrap();
    discard(&review, false).unwrap();
    assert!(!has("review-fix"));

    // With changes, 4.4's check keeps it unless forced.
    let review = repo.review(&w);
    commit_file(Path::new(&review), "c");
    std::fs::write(Path::new(&review).join("c"), "edited").unwrap();
    let err = discard(&review, false).unwrap_err().to_string();
    assert!(err.contains("git worktree remove"), "{err}");
    discard(&review, true).unwrap();
    assert!(has("review-fix"), "its commit is not on worktree-fix");
    run(repo.root(), &["branch", "-D", "review-fix"]);

    // Moved to a branch of the user's: that branch is never Hive's to delete.
    let review = repo.review(&w);
    run(Path::new(&review), &["switch", "-q", "-c", "mine"]);
    repo.projects.forget();
    discard(&review, false).unwrap();
    assert!(has("mine"));

    assert_eq!(
        discard(&w, false).unwrap_err().to_string(),
        "fix is not a review"
    );
    // Its link went with it.
    assert!(repo.projects.reviews().links().is_empty());
}

#[test]
fn a_review_of_a_worktree_that_went_is_a_worktree_like_any_other() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    let procs = repo.no_procs();
    let removed = repo
        .projects
        .remove_worktree(&w, false, Source::Dir(&procs), |_| Ok(()));
    removed.unwrap();
    assert_eq!(repo.worktree(&review).review_of, None);
    assert!(repo.projects.reviews().links().is_empty());
}

#[test]
fn a_renamed_worktree_keeps_its_review() {
    let repo = Repo::new();
    let w = repo.linked("fix");
    let review = repo.review(&w);
    let procs = repo.no_procs();
    let (_, to) = repo
        .projects
        .rename_worktree(&w, "fixed", Source::Dir(&procs))
        .unwrap();
    assert_eq!(repo.worktree(&review).review_of, Some(to));
}
