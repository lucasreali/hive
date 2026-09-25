---
name: release
description: Cut a Hive release: check main is clean and green, bump every manifest and tag with scripts/release.sh. The human pushes.
disable-model-invocation: true
argument-hint: <major.minor.patch>
---

# Cut release v$ARGUMENTS

1. Check the state: on `main`, `git status` clean, no task branch left (`git branch`), no worktree besides the main checkout (`git worktree list`). Otherwise stop and say what is in the way.
2. If `$ARGUMENTS` is empty or not `major.minor.patch`, ask for it. Show the commits since the last tag (`git log --oneline $(git describe --tags --abbrev=0)..`); if there are none, stop.
3. Run the `gates` skill with `MUTANTS=0` on `main`. Any FAIL stops the release.
4. `scripts/release.sh $ARGUMENTS`. It bumps `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `crates/hive/Cargo.toml` and `Cargo.lock`, commits `chore(release): v$ARGUMENTS` and tags it.
5. Never push. End with the commits in the release and the exact command for the human: `git push origin main v$ARGUMENTS` (it starts `.github/workflows/release.yml`).
