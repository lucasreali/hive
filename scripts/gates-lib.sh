# Gate accounting for scripts/gates.sh, sourced (and tested by scripts/test-scripts.sh with a fake
# `cargo`). Needs $out (the log folder). Each gate prints one line; `failures` counts the failed ones.
failures=0

# run <name> <command…>: one gate, logged to $out/<name>.log.
run() {
  local name=$1; shift
  if "$@" >"$out/$name.log" 2>&1; then
    echo "ok    $name"
  else
    echo "FAIL  $name ($out/$name.log)"
    failures=$((failures + 1))
  fi
}

# mutants_verdict <name> <cargo-mutants status> <memwatch status>: its line, and the count.
# cargo-mutants exits 2 for missed mutants, 3 for timeouts, 4 when the unmutated tree fails;
# memwatch exits 3 when it stopped cargo-mutants (low memory), 1 when it could not read memory.
mutants_verdict() {
  local name=$1 status=$2 watch=$3 reason
  if [ "$watch" != 0 ]; then
    reason="stopped by the memory watchdog"
  else
    case $status in
      0) echo "ok    $name"; return 0 ;;
      2) reason="missed mutants" ;;
      3) reason="timeouts" ;;
      4) reason="baseline failed" ;;
      *) reason="cargo-mutants exited $status" ;;
    esac
  fi
  echo "FAIL  $name ($reason, $out/$name.log)"
  failures=$((failures + 1))
}

# mutants <name> <cargo mutants args…>: one cargo-mutants run, logged to $out/<name>.log, with the
# memory watchdog (scripts/memwatch.sh): a runaway mutant must not take the machine down.
mutants() {
  local name=$1 log=$out/$1.log watch=$out/$1-memwatch.log status watcher watch_status; shift
  # Its own log: cargo-mutants writes at its own offset and would overwrite a line appended to its.
  scripts/memwatch.sh cargo-mutants "${GATES_MEM_MIN_KIB:-1500000}" "$watch" &
  watcher=$!
  TMPDIR=/var/tmp/hive-mutants cargo mutants --gitignore true -j "${MUTANTS_JOBS:-1}" "$@" >"$log" 2>&1
  status=$?
  wait "$watcher"
  watch_status=$?
  mutants_verdict "$name" "$status" "$watch_status"
  sed 's/^/      /' "$watch" 2>/dev/null
  grep -E '^(MISSED|TIMEOUT|ERROR)|mutants tested' "$log" | sed 's/^/      /'
}
