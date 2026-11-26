use super::*;
use crate::health::tests::{commit, run};
use std::collections::BTreeMap;
use std::time::SystemTime;

/// A repository with one commit, an ignored folder and Hive's folder apart.
fn repo() -> (tempfile::TempDir, PathBuf, Snapshots) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    run(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join(".gitignore"), "ignored/\n*.log\n").unwrap();
    run(&root, &["add", ".gitignore"]);
    commit(&root, "a");
    let snapshots = Snapshots::new(tmp.path().join("hive/snapshots"));
    (tmp, root, snapshots)
}

fn id(dir: &Path) -> String {
    dir.display().to_string()
}

fn take(snapshots: &Snapshots, dir: &Path, session: &str, now: u64) -> Turn {
    snapshots.take(dir, &id(dir), session, now).unwrap()
}

/// The paths of the turn's tree, and the content of `path` there.
fn tree_files(since: &Since) -> Vec<String> {
    let index = since.store.index(&since.turn.worktree);
    let ls = ["ls-tree", "-r", "--name-only", since.turn.tree.as_str()];
    let out = since
        .store
        .work(&since.worktree, &index, &args(&ls))
        .unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn content(since: &Since, path: &str) -> Option<String> {
    let out = since.ls_tree(path).unwrap();
    let (oid, _) = crate::file::parse_ls_tree(&out)?;
    Some(String::from_utf8(since.blob(&oid).unwrap()).unwrap())
}

/// Every file under `dir` with its bytes and modification time; with `objects`, the objects'
/// times are left out (git refreshes those, which the human accepted).
fn files(dir: &Path) -> BTreeMap<PathBuf, (Vec<u8>, Option<SystemTime>)> {
    let mut found = BTreeMap::new();
    let mut folders = vec![dir.to_owned()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let object = path.strip_prefix(dir).unwrap().starts_with("objects");
            let time = std::fs::metadata(&path).unwrap().modified().ok();
            let time = time.filter(|_| !object);
            found.insert(path.clone(), (std::fs::read(&path).unwrap(), time));
        }
    }
    found
}

#[test]
fn a_snapshot_holds_the_worktree_with_untracked_files_and_without_ignored_ones() {
    let (_tmp, root, snapshots) = repo();
    std::fs::write(root.join("a"), "a2").unwrap();
    std::fs::write(root.join("new"), "n").unwrap();
    std::fs::create_dir(root.join("ignored")).unwrap();
    std::fs::write(root.join("ignored/x"), "x").unwrap();
    std::fs::write(root.join("x.log"), "x").unwrap();
    run(&root, &["add", "new"]);
    let turn = take(&snapshots, &root, "s-1", 1000);
    assert_eq!((turn.n, turn.late, turn.started_ms), (1, false, 1000));
    let since = snapshots.since(&root, &id(&root), "s-1").unwrap();
    assert_eq!(since.turn, turn);
    // The same base found again is equal; another turn's is not.
    let mut other = snapshots.since(&root, &id(&root), "s-1").unwrap();
    assert_eq!(other, since);
    other.turn.n = 9;
    assert_ne!(other, since);
    assert_eq!(tree_files(&since), [".gitignore", "a", "new"]);
    assert_eq!(content(&since, "a").as_deref(), Some("a2"));
    assert_eq!(content(&since, "gone"), None);
    // The repository's own objects do not hold the tree.
    let cat = ["cat-file", "-e", turn.tree.as_str()];
    assert!(git::output(&root, &cat, &[0]).is_err());
    // A second turn of the same session is the next one, and both are kept.
    std::fs::write(root.join("b"), "b").unwrap();
    let second = take(&snapshots, &root, "s-1", 2000);
    assert_eq!(second.n, 2);
    assert_ne!(second.tree, turn.tree);
    assert_eq!(
        snapshots.turns(&root, &id(&root)),
        std::slice::from_ref(&second)
    );
    // Since the second turn: the new file `c`, `b` changed, `a` gone.
    std::fs::write(root.join("c"), "c").unwrap();
    std::fs::write(root.join("b"), "b2\n").unwrap();
    std::fs::remove_file(root.join("a")).unwrap();
    let since = snapshots.since(&root, &id(&root), "s-1").unwrap();
    let (raw, numstat) = since.list().unwrap();
    let raw = crate::changes::parse_raw(&raw);
    let names: Vec<_> = raw.iter().map(|(p, s, _)| (lossy(p), *s)).collect();
    use hive_protocol::FileStatus::*;
    assert_eq!(
        names,
        [
            ("a".into(), Deleted),
            ("b".into(), Modified),
            ("c".into(), Added)
        ]
    );
    let counts = crate::changes::parse_numstat(&numstat);
    assert_eq!(counts[&b"b".to_vec()], (Some(1), Some(1)));
}

fn lossy(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

#[test]
fn the_users_index_and_git_folder_are_untouched() {
    let (_tmp, root, snapshots) = repo();
    std::fs::write(root.join("a"), "a2").unwrap();
    std::fs::write(root.join("staged"), "s").unwrap();
    run(&root, &["add", "staged"]);
    std::fs::write(root.join("untracked"), "u").unwrap();
    // A file put back to its committed text: git meets its object again.
    std::fs::write(root.join(".gitignore"), "ignored/\n*.log\n").unwrap();
    let before = files(&root.join(".git"));
    take(&snapshots, &root, "s", 1);
    std::fs::write(root.join("untracked"), "u2").unwrap();
    take(&snapshots, &root, "s", 2);
    snapshots
        .since(&root, &id(&root), "s")
        .unwrap()
        .list()
        .unwrap();
    assert_eq!(files(&root.join(".git")), before);
}

#[test]
fn a_linked_worktree_has_its_own_index_and_shares_the_store() {
    let (tmp, root, snapshots) = repo();
    let linked = tmp.path().join("linked");
    run(
        &root,
        &["worktree", "add", "-q", "-b", "w", linked.to_str().unwrap()],
    );
    std::fs::write(linked.join("w"), "w").unwrap();
    let main = take(&snapshots, &root, "m", 1);
    let other = take(&snapshots, &linked, "l", 2);
    assert_ne!(main.tree, other.tree);
    assert_eq!(snapshots.turns(&root, &id(&root)), [main]);
    assert_eq!(snapshots.turns(&linked, &id(&linked)), [other]);
    assert_eq!(std::fs::read_dir(&snapshots.root).unwrap().count(), 1);
    // A session whose last turn is of another worktree has no base here.
    assert!(snapshots.since(&root, &id(&root), "l").is_err());
}

#[test]
fn no_filter_of_a_nested_repository_runs() {
    let (tmp, root, snapshots) = repo();
    let spy = tmp.path().join("spy");
    // A repository inside the worktree (a submodule's checkout is one too), with one inside it.
    let inner = root.join("inner");
    let deeper = inner.join("deeper");
    for dir in [&deeper, &inner] {
        std::fs::create_dir_all(dir).unwrap();
        run(dir, &["init", "-q"]);
        commit(dir, "f");
    }
    // `deeper` is a gitlink of `inner`, and so is `gone`, not checked out.
    run(&inner, &["add", "deeper"]);
    let sha = git::output(&deeper, &["rev-parse", "HEAD"], &[0]).unwrap();
    let link = format!("160000,{},gone", String::from_utf8_lossy(sha.trim_ascii()));
    run(&inner, &["update-index", "--add", "--cacheinfo", &link]);
    take(&snapshots, &root, "s", 1);
    let filter = format!("echo ran >> '{}'; cat", spy.display());
    for (dir, name) in [(&inner, "spy"), (&deeper, "deep")] {
        run(dir, &["config", &format!("filter.{name}.clean"), &filter]);
        std::fs::write(dir.join(".gitattributes"), format!("* filter={name}\n")).unwrap();
        // The same size: only its content, read through the filter, tells.
        std::fs::write(dir.join("f"), "g").unwrap();
    }
    take(&snapshots, &root, "s", 2);
    snapshots
        .since(&root, &id(&root), "s")
        .unwrap()
        .list()
        .unwrap();
    assert!(!spy.exists(), "a nested repository's filter ran");
}

#[test]
fn no_hook_or_filter_of_the_repository_runs() {
    let (tmp, root, snapshots) = repo();
    let spy = tmp.path().join("spy");
    let hooks = root.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let script = format!("#!/bin/sh\necho ran >> '{}'\ncat\n", spy.display());
    for hook in ["post-index-change", "reference-transaction"] {
        let hook = hooks.join(hook);
        std::fs::write(&hook, &script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let filter = format!("echo ran >> '{}'; cat", spy.display());
    run(&root, &["config", "filter.spy.clean", &filter]);
    run(&root, &["config", "filter.spy.required", "true"]);
    run(&root, &["config", "filter.other.smudge", "cat"]);
    // A key without a driver's name names no driver.
    run(&root, &["config", "filter.lonely", "x"]);
    std::fs::write(root.join(".gitattributes"), "* filter=spy\n").unwrap();
    std::fs::write(root.join("a"), "changed").unwrap();
    let turn = take(&snapshots, &root, "s", 1);
    snapshots
        .since(&root, &id(&root), "s")
        .unwrap()
        .list()
        .unwrap();
    assert!(!spy.exists(), "a repository program ran");
    // The raw content went in.
    let since = snapshots.since(&root, &id(&root), "s").unwrap();
    assert_eq!(since.turn, turn);
    assert_eq!(content(&since, "a").as_deref(), Some("changed"));
}

#[test]
fn a_filter_that_cannot_be_turned_off_stops_the_snapshot() {
    let (_tmp, root, snapshots) = repo();
    run(&root, &["config", "filter.a=b.clean", "cat"]);
    let err = snapshots.take(&root, &id(&root), "s", 1).unwrap_err();
    assert_eq!(err.to_string(), "cannot turn off the filter a=b");
}

#[test]
fn only_session_ids_that_fit_a_ref_get_a_snapshot() {
    let (_tmp, root, snapshots) = repo();
    let long = "x".repeat(65);
    for bad in ["", "a/b", "../x", "a b", long.as_str()] {
        let err = snapshots.take(&root, &id(&root), bad, 1).unwrap_err();
        assert_eq!(err.to_string(), "not a session id Hive keeps snapshots for");
        assert!(snapshots.since(&root, &id(&root), bad).is_err());
    }
    take(&snapshots, &root, &"x".repeat(64), 1);
    take(&snapshots, &root, "A-z_0", 1);
}

#[test]
fn outside_a_repository_there_is_no_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let snapshots = Snapshots::new(tmp.path().join("snapshots"));
    let plain = tmp.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    assert!(snapshots.take(&plain, "p", "s", 1).is_err());
    assert_eq!(snapshots.turns(&plain, "p"), []);
    let err = snapshots.since(&plain, "p", "s").unwrap_err();
    assert_eq!(
        err.to_string(),
        "No snapshot of this agent's last turn in this worktree"
    );
}

#[test]
fn a_session_keeps_its_last_ten_turns_and_pruning_drops_the_others() {
    let (_tmp, root, snapshots) = repo();
    let snapshots = snapshots.with_limits(SIZE_CAP, Duration::from_secs(3600));
    let mut trees = Vec::new();
    for n in 1..=11u64 {
        std::fs::write(root.join("f"), format!("turn {n}")).unwrap();
        trees.push(take(&snapshots, &root, "s", n).tree);
    }
    let (store, _) = snapshots.store(&Repo::of(&root).unwrap());
    let kept: Vec<u64> = store.read().turns.iter().map(|t| t.n).collect();
    assert_eq!(kept, (2..=11).rev().collect::<Vec<_>>());
    // The first turn's tree and its own blob are gone; the kept ones are whole.
    let exists = |object: &str| {
        let index = store.index(&id(&root));
        store
            .work(&root, &index, &args(&["cat-file", "-e", object]))
            .is_ok()
    };
    assert!(!exists(&trees[0]));
    let since = snapshots.since(&root, &id(&root), "s").unwrap();
    assert_eq!(content(&since, "f").as_deref(), Some("turn 11"));
    for tree in &trees[1..] {
        assert!(exists(tree));
    }
    let refs = store.git(&args(&["for-each-ref", "--format=%(refname)"]), &[]);
    let refs = String::from_utf8(refs.unwrap()).unwrap();
    assert_eq!(refs.lines().count(), 10);
    assert!(!refs.contains("refs/turns/s/1\n"));
}

#[test]
fn past_the_size_cap_the_oldest_snapshots_go() {
    let (_tmp, root, snapshots) = repo();
    // Random-looking bytes do not compress.
    let noise = |seed: u64| -> Vec<u8> {
        let mut x = seed;
        (0..40_000)
            .map(|_| {
                x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                (x >> 56) as u8
            })
            .collect()
    };
    let probe = Snapshots::new(snapshots.root.join("probe"));
    std::fs::write(root.join("big"), noise(1)).unwrap();
    take(&probe, &root, "p", 1);
    let (store, _) = probe.store(&Repo::of(&root).unwrap());
    let one = store.size();
    // Room for two snapshots of a big file.
    let snapshots = snapshots.with_limits(one * 5 / 2, Duration::ZERO);
    take(&snapshots, &root, "a", 1);
    std::fs::write(root.join("big"), noise(2)).unwrap();
    take(&snapshots, &root, "b", 2);
    std::fs::write(root.join("big"), noise(3)).unwrap();
    let third = take(&snapshots, &root, "c", 3);
    let (store, _) = snapshots.store(&Repo::of(&root).unwrap());
    let kept: Vec<String> = store.read().turns.into_iter().map(|t| t.session).collect();
    assert_eq!(kept, ["c", "b"]);
    assert!(store.size() <= one * 5 / 2);
    assert_eq!(third.session, "c");
    // At the cap exactly, nothing goes (a snapshot of nothing new adds no object).
    let exact = Snapshots::new(snapshots.root.clone()).with_limits(store.size(), Duration::MAX);
    take(&exact, &root, "e", 4);
    let kept: Vec<String> = store.read().turns.into_iter().map(|t| t.session).collect();
    assert_eq!(kept, ["e", "c", "b"]);
    // A snapshot bigger than the cap alone is not kept.
    let tiny = Snapshots::new(snapshots.root.clone()).with_limits(1, Duration::ZERO);
    let err = tiny.take(&root, &id(&root), "d", 4).unwrap_err();
    assert_eq!(
        err.to_string(),
        "this turn's snapshot was too large to keep"
    );
    assert_eq!(store.read().turns, []);
}

#[test]
fn a_store_is_pruned_at_most_once_in_a_while() {
    let (_tmp, root, snapshots) = repo();
    // What a listing hashed and no snapshot keeps: a loose object of the store.
    let listed = |snapshots: &Snapshots, text: &str| {
        take(snapshots, &root, "s", 1);
        std::fs::write(root.join("a"), text).unwrap();
        let since = snapshots.since(&root, &id(&root), "s").unwrap();
        since.list().unwrap();
        let oid = git::output(&root, &["hash-object", "a"], &[0]).unwrap();
        std::fs::write(root.join("a"), "a").unwrap();
        take(snapshots, &root, "s", 2);
        let index = since.store.index(&id(&root));
        let oid = String::from_utf8_lossy(oid.trim_ascii()).into_owned();
        since
            .store
            .work(&root, &index, &args(&["cat-file", "-e", &oid]))
            .is_ok()
    };
    let rarely = Snapshots::new(snapshots.root.clone()).with_limits(SIZE_CAP, Duration::MAX);
    assert!(listed(&rarely, "listed once"));
    let always = Snapshots::new(snapshots.root.clone()).with_limits(SIZE_CAP, Duration::ZERO);
    assert!(!listed(&always, "listed twice"));
}

#[test]
fn a_restarted_service_finds_the_turns_and_a_session_end_drops_them() {
    let (_tmp, root, snapshots) = repo();
    let a = take(&snapshots, &root, "a", 1);
    let b = take(&snapshots, &root, "b", 2);
    let again = Snapshots::new(snapshots.root.clone());
    assert_eq!(again.turns(&root, &id(&root)), [b.clone(), a]);
    again.end("a");
    assert_eq!(again.turns(&root, &id(&root)), [b]);
    // The last session gone, so is the store.
    again.end("b");
    assert_eq!(std::fs::read_dir(&snapshots.root).unwrap().count(), 0);
    // Nothing to end without the folder.
    let none = Snapshots::new(snapshots.root.join("none"));
    none.end("a");
    none.sweep(0);
}

#[test]
fn a_sweep_drops_old_turns_gone_worktrees_and_gone_repositories() {
    let (tmp, root, snapshots) = repo();
    let snapshots = snapshots.with_limits(SIZE_CAP, Duration::from_secs(3600));
    let linked = tmp.path().join("linked");
    run(
        &root,
        &["worktree", "add", "-q", "-b", "w", linked.to_str().unwrap()],
    );
    let old = take(&snapshots, &root, "old", 1);
    let new = take(&snapshots, &root, "new", AGE_MS + 1);
    take(&snapshots, &linked, "linked", AGE_MS + 1);
    run(&root, &["worktree", "remove", linked.to_str().unwrap()]);
    snapshots.sweep(AGE_MS + 2);
    assert_eq!(
        snapshots.turns(&root, &id(&root)),
        std::slice::from_ref(&new)
    );
    let (store, _) = snapshots.store(&Repo::of(&root).unwrap());
    assert!(!store.read().turns.contains(&old));
    // Nothing more to drop: the store stays as it is.
    snapshots.sweep(AGE_MS + 2);
    assert_eq!(snapshots.turns(&root, &id(&root)), [new]);
    // Garbage in the folder, and a store whose repository moved, go.
    std::fs::create_dir(snapshots.root.join("junk")).unwrap();
    std::fs::write(snapshots.root.join("file"), "x").unwrap();
    std::fs::rename(&root, tmp.path().join("moved")).unwrap();
    snapshots.sweep(AGE_MS + 2);
    let left: Vec<_> = std::fs::read_dir(&snapshots.root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, [OsString::from("file")]);
}

#[test]
fn an_edit_before_the_snapshot_is_done_makes_it_late() {
    let (_tmp, root, snapshots) = repo();
    snapshots.started("s");
    snapshots.edited("s");
    snapshots.edited("other");
    assert!(take(&snapshots, &root, "s", 1).late);
    // Not pending any more: a later edit changes nothing.
    snapshots.edited("s");
    snapshots.started("s");
    assert!(!take(&snapshots, &root, "s", 2).late);
    // A failed snapshot is not left pending either.
    snapshots.started("bad id");
    assert!(snapshots.take(&root, "x", "bad id", 3).is_err());
    assert!(lock(&snapshots.pending).is_empty());
}

#[test]
fn a_broken_snapshot_index_is_seeded_again() {
    let (_tmp, root, snapshots) = repo();
    take(&snapshots, &root, "s", 1);
    let (store, _) = snapshots.store(&Repo::of(&root).unwrap());
    let index = store.index(&id(&root));
    std::fs::write(&index, "not an index").unwrap();
    assert!(snapshots.take(&root, &id(&root), "s", 2).is_err());
    assert!(!index.exists());
    assert_eq!(take(&snapshots, &root, "s", 3).n, 2);
    // Without a user index (nothing staged ever), the snapshot starts empty.
    std::fs::remove_file(root.join(".git/index")).unwrap();
    std::fs::remove_file(&index).unwrap();
    let since = snapshots.since(&root, &id(&root), "s").unwrap();
    take(&snapshots, &root, "s", 4);
    assert_eq!(tree_files(&since), [".gitignore", "a"]);
}

#[test]
fn a_record_hive_could_not_have_written_is_not_trusted() {
    let (_tmp, root, snapshots) = repo();
    let first = take(&snapshots, &root, "s", 1);
    let (store, _) = snapshots.store(&Repo::of(&root).unwrap());
    // Turns edited by hand: a session that fits no ref, a tree that is no id.
    let mut record = store.read();
    let odd = |session: &str, tree: &str| Turn {
        session: session.into(),
        tree: tree.into(),
        ..first.clone()
    };
    record.turns.push(odd("a/b", &first.tree));
    record.turns.push(odd("t", "--output=x"));
    record.turns.push(odd("u", &"0".repeat(39)));
    store.write(&record).unwrap();
    assert_eq!(store.read().turns, std::slice::from_ref(&first));
    // A record that cannot be read at all: the store starts again, its old refs gone.
    std::fs::write(store.record(), "not json").unwrap();
    let again = take(&snapshots, &root, "s", 2);
    assert_eq!((again.n, store.read().turns.len()), (1, 1));
    let refs = store.git(&args(&["for-each-ref", "--format=%(refname)"]), &[]);
    assert_eq!(
        String::from_utf8(refs.unwrap()).unwrap(),
        "refs/turns/s/1\n"
    );
}

#[test]
fn a_listing_past_the_size_cap_prunes_what_it_hashed() {
    let (_tmp, root, snapshots) = repo();
    take(&snapshots, &root, "s", 1);
    let (store, _) = snapshots.store(&Repo::of(&root).unwrap());
    let kept = store.size();
    std::fs::write(root.join("a"), "x".repeat(10_000)).unwrap();
    let roomy = Snapshots::new(snapshots.root.clone()).with_limits(SIZE_CAP, Duration::MAX);
    roomy.since(&root, &id(&root), "s").unwrap().list().unwrap();
    assert!(store.size() > kept);
    let tight = Snapshots::new(snapshots.root.clone()).with_limits(kept, Duration::MAX);
    tight.since(&root, &id(&root), "s").unwrap().list().unwrap();
    assert_eq!(store.size(), kept);
}

#[test]
fn a_snapshot_whose_objects_are_gone_cannot_be_listed() {
    let (_tmp, root, snapshots) = repo();
    std::fs::write(root.join("a"), "only here").unwrap();
    take(&snapshots, &root, "s", 1);
    let (store, _) = snapshots.store(&Repo::of(&root).unwrap());
    std::fs::remove_dir_all(store.objects()).unwrap();
    std::fs::create_dir_all(store.objects().join("info")).unwrap();
    let since = snapshots.since(&root, &id(&root), "s").unwrap();
    assert!(since.list().is_err());
}

#[test]
fn turns_are_kept_per_session_and_by_age() {
    let turn = |session: &str, n: u64, started_ms: u64, worktree: &str| Turn {
        session: session.into(),
        n,
        tree: "t".into(),
        worktree: worktree.into(),
        started_ms,
        late: false,
    };
    let mut turns: Vec<Turn> = (1..=12).map(|n| turn("a", n, n, "/w")).collect();
    turns.push(turn("b", 1, 5, "/w"));
    turns.push(turn("c", 1, 5, "/gone"));
    let dropped = trim(&mut turns, AGE_MS + 3, |w| w == "/w");
    let n = |turns: &[Turn]| {
        turns
            .iter()
            .map(|t| (t.session.clone(), t.n))
            .collect::<Vec<_>>()
    };
    let a = |ns: &[u64]| ns.iter().map(|&n| ("a".to_owned(), n)).collect::<Vec<_>>();
    let mut kept = a(&[12, 11, 10, 9, 8, 7, 6, 5]);
    kept.insert(8, ("b".into(), 1));
    kept.extend(a(&[4, 3]));
    assert_eq!(n(&turns), kept);
    // `c`'s worktree is gone; `a` 2 and 1 are past ten, and over the age at `AGE_MS + 3`.
    assert_eq!(
        n(&dropped),
        [("c".into(), 1), ("a".into(), 2), ("a".into(), 1)]
    );
}

#[test]
fn a_snapshot_looks_into_a_bounded_number_of_nested_repositories() {
    assert!(nested(NESTED).is_ok());
    let err = nested(NESTED + 1).unwrap_err();
    assert_eq!(err.to_string(), "too many nested repositories to snapshot");
}

#[test]
fn names_stay_the_same_across_runs() {
    assert_eq!(fnv(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv(b"a"), 0xaf63_dc4c_8601_ec8c);
    let store = Store { dir: "/h".into() };
    assert_eq!(store.index("a"), Path::new("/h/af63dc4c8601ec8c.index"));
}
