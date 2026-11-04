#!/usr/bin/env bash
# Runs the macOS app from this checkout (TODO 17.2), beside an installed Hive without sharing its
# service:
#   - builds target/debug/hive and points the app's bridge at it (HIVE_BRIDGE); `tauri dev` also
#     watches crates/hive and rebuilds it before the app (scripts/mac-dev-runner.sh, TODO 18.14),
#     so a service edit restarts the app on the new service
#   - gives that service its own runtime folder (socket, lock) and Hive folder (settings, wrapper,
#     hooks, projects), through XDG_RUNTIME_DIR and HIVE_HOME (TODO 22.1)
#   - runs `bun run tauri dev`, which starts Vite on port 1420 (hot reload) and the debug app,
#     which runs as `com.lucas.hive.dev` (TODO 17.18): its own single instance and window state
# The service passes its environment to its terminals, so they see both variables too: `hive hook`
# needs XDG_RUNTIME_DIR to reach this service. The XDG data and config folders are left alone (git,
# gh and fish read theirs), and Hive's settings are the dev folder's own.
#   HIVE_DEV_DIR=<dir>   the Hive folder (default ~/.hive-dev); one per parallel run
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH=$HOME/.cargo/bin:$PATH
[ "$(uname -s)" = Darwin ] || { echo "mac-dev: macOS only (README: Development, Running)" >&2; exit 1; }

cargo build -p hive

data=${HIVE_DEV_DIR:-$HOME/.hive-dev}
mkdir -p -m 700 "$data"
data=$(cd "$data" && pwd -P)
# A short path under the per-user $TMPDIR: a Unix socket path is limited to 104 bytes. One per data
# folder, so two dev runs with different HIVE_DEV_DIR get two services. Never $TMPDIR/hive itself:
# that is the installed service's runtime folder (17.9).
runtime=${TMPDIR:-/tmp}
runtime=${runtime%/}/hive-dev-$(printf %s "$data" | cksum | cut -d' ' -f1)
mkdir -p -m 700 "$runtime"

export HIVE_BRIDGE="$PWD/target/debug/hive" HIVE_HOME="$data" XDG_RUNTIME_DIR="$runtime"
echo "mac-dev: Hive folder $HIVE_HOME, runtime $XDG_RUNTIME_DIR/hive"
exec bun run tauri dev --runner "$PWD/scripts/mac-dev-runner.sh" \
  --additional-watch-folders "$PWD/crates/hive"
