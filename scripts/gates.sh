#!/usr/bin/env bash
# CI is the authoritative gate run (.github/workflows/ci.yml and macos.yml, TODO 5.11): a task
# merges only with green CI on its branch tip. This script is for optional local use, on Linux or
# macOS; each runs what its CI workflow runs (TODO 17.2): on Linux ci.yml's coverage and diff
# mutants, on macOS also macos.yml's (coverage with macos.rs counted, mutants of macos.rs).
# Runs every Rust quality gate from CLAUDE.md and prints one line per gate.
#   BASE=<rev>              diff base for cargo mutants (default: main)
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
run() { local name=$1; shift; if "$@" >"$out/$name.log" 2>&1; then echo "ok    $name"; else echo "FAIL  $name ($out/$name.log)"; fi; }
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

# One cargo-mutants run, logged to $out/<log>.log, with the memory watchdog (scripts/memwatch.sh):
# a runaway mutant must not take the machine (and every Hive terminal) down.
mutants() {
  local log=$out/$1.log watch=$out/$1-memwatch.log; shift
  # Its own log: cargo-mutants writes at its own offset and would overwrite a line appended to its.
  scripts/memwatch.sh cargo-mutants "${GATES_MEM_MIN_KIB:-1500000}" "$watch" &
  TMPDIR=/var/tmp/hive-mutants cargo mutants --gitignore true -j "${MUTANTS_JOBS:-1}" "$@" >"$log" 2>&1
  wait
  sed 's/^/      /' "$watch" 2>/dev/null
  grep -E '^(MISSED|TIMEOUT|ERROR)|mutants tested' "$log" | sed 's/^/      /'
}
if [ "${MUTANTS:-1}" = 1 ]; then
  git add -N . 2>/dev/null  # untracked files must show up in the diff
  # The mutant trees must live outside the repository (worktree tests would otherwise find this
  # repo by walking up).
  mkdir -p /var/tmp/hive-mutants
  git diff "${BASE:-main}" -- '*.rs' > "$out/diff.patch"
  # Same setup as CI (.cargo/mutants.toml: nextest, `mutants` profile; needs cargo-nextest) in
  # one shard, with the baseline run here: it sets the automatic test timeout.
  mutants mutants --in-diff "$out/diff.patch"
  # As macos.yml: `--no-config`, since `.cargo/mutants.toml` excludes this file for Linux.
  [ "$os" = Darwin ] && mutants mutants-macos --no-config -f crates/hive/src/macos.rs --timeout 120
fi
exit 0
