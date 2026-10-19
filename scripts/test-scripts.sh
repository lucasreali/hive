#!/usr/bin/env bash
# Tests the portable scripts on this OS (TODO 17.2; ci.yml runs it on Linux, the Mac by hand):
#   - release.sh in a scratch repository holding a copy of the manifests (a fake `cargo`)
#   - memwatch.sh reads the available memory, stops a process below its threshold, and leaves it
#     alone above
# Never touches the real environment: a temporary HOME, a scratch repository outside this one.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
repo=$PWD
tmp=$(mktemp -d)
probe=hive-memprobe
trap 'pkill -x $probe 2>/dev/null; rm -rf "$tmp"' EXIT
failures=0
check() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; failures=$((failures + 1)); fi; }

# --- release.sh ---------------------------------------------------------------------------------
export HOME=$tmp/home GIT_CONFIG_NOSYSTEM=1
unset XDG_CONFIG_HOME
mkdir -p "$HOME/.cargo/bin" "$tmp/rel/scripts" "$tmp/rel/src-tauri" "$tmp/rel/crates/hive"
printf '#!/bin/sh\nexit 0\n' >"$HOME/.cargo/bin/cargo"
chmod +x "$HOME/.cargo/bin/cargo"
manifests="src-tauri/tauri.conf.json package.json src-tauri/Cargo.toml crates/hive/Cargo.toml"
for f in $manifests scripts/release.sh; do cp "$repo/$f" "$tmp/rel/$f"; done
git -C "$tmp/rel" init -q
git -C "$tmp/rel" config user.name test
git -C "$tmp/rel" config user.email test@example.invalid
git -C "$tmp/rel" add -A
git -C "$tmp/rel" commit -qm base
check "release: refuses a bad version" '! "$tmp/rel/scripts/release.sh" 1.2 2>/dev/null'
"$tmp/rel/scripts/release.sh" 9.8.7 >/dev/null
check "release: tagged v9.8.7" '[ "$(git -C "$tmp/rel" tag)" = v9.8.7 ]'
check "release: commit message" '[ "$(git -C "$tmp/rel" log -1 --format=%s)" = "chore(release): v9.8.7" ]'
# One line per manifest, its own version: never a dependency's.
check "release: one line per manifest" \
  '[ "$(git -C "$tmp/rel" diff --numstat HEAD~1 | awk "\$1 == 1 && \$2 == 1" | wc -l | tr -d " ")" = 4 ]'
check "release: json versions" \
  '[ "$(grep -h "^  \"version\": \"9.8.7\",$" "$tmp/rel/package.json" "$tmp/rel/src-tauri/tauri.conf.json" | wc -l | tr -d " ")" = 2 ]'
check "release: toml versions" \
  '[ "$(grep -h "^version = \"9.8.7\"$" "$tmp/rel/src-tauri/Cargo.toml" "$tmp/rel/crates/hive/Cargo.toml" | wc -l | tr -d " ")" = 2 ]'

# --- memwatch.sh --------------------------------------------------------------------------------
check "memwatch: available memory" '[[ $(scripts/memwatch.sh --available) =~ ^[1-9][0-9]*$ ]]'
# A process with a name of its own, so pkill hits only it. Built with cc: macOS kills a copy of a
# system binary (/bin/dash, /bin/sleep) run from another folder.
printf '#include <unistd.h>\nint main(void) { sleep(30); return 0; }\n' | cc -x c -o "$tmp/$probe" -
export MEMWATCH_INTERVAL=0.2
# Below the threshold (more than any machine has): stopped, and the log says why.
"$tmp/$probe" &
low=$!
scripts/memwatch.sh $probe 999999999999 "$tmp/low.log"
wait $low 2>/dev/null
low_status=$?
check "memwatch: stops below the threshold" '[ $low_status = 143 ]'  # 128 + SIGTERM
check "memwatch: logs the stop" 'grep -q "memory low" "$tmp/low.log"'
# Above it: left running; the watchdog exits once the process is gone.
"$tmp/$probe" &
high=$!
scripts/memwatch.sh $probe 0 "$tmp/high.log" &
watch=$!
sleep 1
check "memwatch: leaves it above the threshold" 'kill -0 $high 2>/dev/null && kill -0 $watch 2>/dev/null'
kill $high
wait $high 2>/dev/null
wait $watch
watch_status=$?
check "memwatch: exits with the process" '[ $watch_status = 0 ] && [ ! -s "$tmp/high.log" ]'

[ $failures = 0 ]
