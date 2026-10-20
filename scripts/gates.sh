#!/usr/bin/env bash
# CI is the authoritative gate run (.github/workflows/ci.yml and macos.yml, TODO 5.11): a task
# merges only with green CI on its branch tip. This script is for optional local use, on Linux or
# macOS (TODO 17.2). On Linux, llvm-cov and the diff mutants run as ci.yml / mutants.yml do. On
# macOS (Darwin), llvm-cov uses macos.yml's exclusion regex instead of ci.yml's (macos.rs is counted),
# the mutants of macos.rs run as macos.yml does, and the diff mutants cannot test Linux-only `cfg`
# code (it is not compiled there): CI's mutants.yml on Linux stays the authority for those.
# Runs every Rust quality gate from CLAUDE.md, prints one line per gate, exits non-zero when any
# gate failed.
#   BASE=<rev>              diff base for cargo mutants (default: the merge base with main)
#   MUTANTS=0               skip mutation testing
#   MUTANTS_JOBS=<n>        mutants tested at a time (default 1)
#   CARGO_BUILD_JOBS=<n>    build jobs (default: every core)
#   GATES_MEM_MIN_KIB=<n>   stop the mutants below this much available memory (default 1500000)
# Logs go to target/gates-*/<gate>.log.
cd "$(dirname "$0")/.." || exit 1
export PATH=$HOME/.cargo/bin:$PATH
os=$(uname -s)
if [ "$os" = Darwin ]; then cores=$(sysctl -n hw.ncpu); else cores=$(nproc); fi
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-$cores}
mkdir -p target
out=$(mktemp -d -p target gates-XXXXXX)
source scripts/gates-lib.sh  # run, mutants: one line per gate, failures counted
run fmt cargo fmt --all --check
run clippy cargo clippy --workspace --all-targets -- -D warnings
run test cargo test --workspace
run check-locked cargo check --workspace --locked
run deny cargo deny check
run machete cargo machete
run guard python3 .claude/hooks/test_guard.py
# Exclusions: COVERAGE_EXCLUSIONS.md. The same regex as ci.yml (Linux) and macos.yml (macOS, where
# macos.rs is compiled and counted, and the windows files are not compiled).
if [ "$os" = Darwin ]; then
  ignore='src-tauri/src/main\.rs'
else
  ignore='src-tauri/src/main\.rs|crates/hive/src/macos\.rs|crates/hive/src/windows[^/]*\.rs'
fi
run llvm-cov cargo llvm-cov --workspace --fail-under-lines 100 --ignore-filename-regex "$ignore"
grep -E '^\S+\.rs|^TOTAL' "$out/llvm-cov.log" | awk '$10 != "100.00%" || /TOTAL/ {print "      " $1, "lines missed:", $9, $10}'

if [ "${MUTANTS:-1}" = 1 ]; then
  git add -N . 2>/dev/null  # untracked files must show up in the diff
  # The mutant trees must live outside the repository (worktree tests would otherwise find this
  # repo by walking up).
  mkdir -p /var/tmp/hive-mutants
  # As mutants.yml: the diff since the branch left `main` (its merge base), so commits that reached
  # `main` meanwhile are not mutated.
  git diff "${BASE:-$(git merge-base main HEAD)}" -- '*.rs' > "$out/diff.patch"
  # Same setup as CI (.cargo/mutants.toml: nextest, `mutants` profile; needs cargo-nextest) in
  # one shard, with the baseline run here: it sets the automatic test timeout.
  mutants mutants --in-diff "$out/diff.patch"
  # As macos.yml: `--no-config`, since `.cargo/mutants.toml` excludes this file for Linux.
  if [ "$os" = Darwin ]; then
    mutants mutants-macos --no-config -f crates/hive/src/macos.rs -f src-tauri/src/macos.rs --timeout 120
  fi
fi
[ "$failures" = 0 ] || { echo "gates: $failures failed"; exit 1; }
