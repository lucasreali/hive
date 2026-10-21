#!/usr/bin/env bash
# A second launch of the app ends at once and leaves the first running (TODO 17.18): the
# single-instance plugin hands it to the running app, which brings its window up. macOS CI only
# (macos.yml): it opens the app's window, and its webview keeps caches under the user's Library.
#   scripts/second-launch.sh [app]   app: a development build (default target/debug/hive-app)
set -euo pipefail
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] || { echo "second-launch: macOS only" >&2; exit 1; }
app=${1:-target/debug/hive-app}

# The plugin's socket on macOS: /tmp/<identifier, '.' and '-' as '_'>_si.sock, where a
# development build's identifier ends in ".dev" (hive_lib::instance_identifier).
identifier=$(sed -n 's/^  "identifier": "\(.*\)",$/\1/p' src-tauri/tauri.conf.json)
[ -n "$identifier" ] || { echo "second-launch: no identifier in tauri.conf.json" >&2; exit 1; }
socket=/tmp/$(printf %s "$identifier.dev" | tr .- __)_si.sock
[ ! -e "$socket" ] || { echo "second-launch: $socket exists: a development Hive is running" >&2; exit 1; }

fail() { echo "second-launch: $*" >&2; exit 1; }
home=$(mktemp -d "${TMPDIR:-/tmp}/hive-second-launch.XXXXXX")
first= second=
cleanup() {
  for pid in $first $second; do kill "$pid" 2>/dev/null || true; done
  rm -rf "$home" "$socket"
}
trap cleanup EXIT

# A temporary HOME: the window-state file goes under it.
HOME=$home "$app" &
first=$!
for _ in $(seq 150); do
  [ -S "$socket" ] && break
  kill -0 "$first" 2>/dev/null || fail "the first launch ended"
  sleep 0.2
done
[ -S "$socket" ] || fail "the first launch opened no $socket in 30 s"

HOME=$home "$app" &
second=$!
for _ in $(seq 100); do
  kill -0 "$second" 2>/dev/null || break
  sleep 0.2
done
kill -0 "$second" 2>/dev/null && fail "the second launch still runs after 20 s"
status=0
wait "$second" || status=$?
second=
[ "$status" = 0 ] || fail "the second launch exited with $status"
kill -0 "$first" 2>/dev/null || fail "the first launch ended with the second"
echo "second-launch: the second launch ended and the first runs on"
