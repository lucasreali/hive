#!/usr/bin/env bash
# Runs every Rust quality gate from CLAUDE.md and prints one line per gate.
#   BASE=<rev>   diff base for cargo mutants (default: main)
#   MUTANTS=0    skip mutation testing
# Logs go to target/gates-*/<gate>.log.
cd "$(dirname "$0")/.." || exit 1
export PATH=$HOME/.cargo/bin:$PATH
out=$(mktemp -d -p target gates-XXXXXX)
run() { local name=$1; shift; if "$@" >"$out/$name.log" 2>&1; then echo "ok    $name"; else echo "FAIL  $name ($out/$name.log)"; fi; }
run fmt cargo fmt --all --check
run clippy cargo clippy --workspace --all-targets -- -D warnings
run test cargo test --workspace
run check-locked cargo check --workspace --locked
run deny cargo deny check
run machete cargo machete
run llvm-cov cargo llvm-cov --workspace --fail-under-lines 100 --ignore-filename-regex 'src-tauri/src/main\.rs'
grep -E '^\S+\.rs|^TOTAL' "$out/llvm-cov.log" | awk '$10 != "100.00%" || /TOTAL/ {print "      " $1, "lines missed:", $9, $10}'
if [ "${MUTANTS:-1}" = 1 ]; then
  git add -N . 2>/dev/null  # untracked files must show up in the diff
  # /tmp is a small tmpfs and the mutant trees must live outside the repository
  # (worktree tests would otherwise find this repo by walking up).
  mkdir -p /var/tmp/hive-mutants
  git diff "${BASE:-main}" -- '*.rs' > "$out/diff.patch"
  TMPDIR=/var/tmp/hive-mutants cargo mutants --gitignore true --in-diff "$out/diff.patch" >"$out/mutants.log" 2>&1
  grep -E '^(MISSED|TIMEOUT|ERROR)|mutants tested' "$out/mutants.log" | sed 's/^/      /'
fi
