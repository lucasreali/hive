#!/usr/bin/env bash
# Runs every Rust quality gate from CLAUDE.md and prints one line per gate.
#   BASE=<rev>   diff base for cargo mutants (default: main)
#   MUTANTS=0    skip mutation testing
# Logs go to target/gates-*/<gate>.log.
cd "$(dirname "$0")/.." || exit 1
export PATH=$HOME/.cargo/bin:$PATH
# The WSL VM has ~10 GB: instrumented builds on every core, or a runaway mutant, can take
# the whole VM down (and every Hive terminal with it). Keep builds narrow, one mutant at a
# time, and stop mutants when memory runs low.
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}
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
  ( while sleep 2; do
      pgrep -f 'cargo-mutants|cargo mutants' >/dev/null || break
      if [ "$(awk '/MemAvailable/{print $2}' /proc/meminfo)" -lt 1500000 ]; then
        echo "gates: memory low, mutants stopped" >>"$out/mutants.log"
        pkill -f 'cargo-mutants|cargo mutants'
        break
      fi
    done ) &
  TMPDIR=/var/tmp/hive-mutants cargo mutants --gitignore true -j 1 --timeout 60 --in-diff "$out/diff.patch" >"$out/mutants.log" 2>&1
  grep -E '^(MISSED|TIMEOUT|ERROR)|mutants tested|memory low' "$out/mutants.log" | sed 's/^/      /'
fi
