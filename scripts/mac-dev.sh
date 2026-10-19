#!/usr/bin/env bash
# Runs the macOS app from this checkout (TODO 17.2), beside an installed Hive without sharing its
# service:
#   - builds target/debug/hive and points the app's bridge at it (HIVE_BRIDGE)
#   - gives that service its own runtime folder (socket, lock) and data folder (wrapper, hooks,
#     projects), through XDG_RUNTIME_DIR and XDG_DATA_HOME
#   - runs `bun run tauri dev`, which starts Vite on port 1420 (hot reload) and the debug app
# The service passes its environment to its terminals, so they see both variables too: `hive hook`
# needs XDG_RUNTIME_DIR to reach this service, and XDG-aware tools in those terminals (e.g. fish's
# history) keep their data under the dev folder. XDG_CONFIG_HOME is left alone (git, gh and fish
# read their config there), so Hive's settings are the installed app's.
#   HIVE_DEV_DIR=<dir>   the data folder (default ~/.local/share/hive-dev); one per parallel run
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH=$HOME/.cargo/bin:$PATH
[ "$(uname -s)" = Darwin ] || { echo "mac-dev: macOS only (from WSL: scripts/win-dev.sh)" >&2; exit 1; }

cargo build -p hive

data=${HIVE_DEV_DIR:-$HOME/.local/share/hive-dev}
mkdir -p "$data"
data=$(cd "$data" && pwd -P)
# A short path under the per-user $TMPDIR: a Unix socket path is limited to 104 bytes. One per data
# folder, so two dev runs with different HIVE_DEV_DIR get two services.
runtime=${TMPDIR:-/tmp}
runtime=${runtime%/}/hive-dev-$(printf %s "$data" | cksum | cut -d' ' -f1)
mkdir -p -m 700 "$runtime"

export HIVE_BRIDGE="$PWD/target/debug/hive" XDG_DATA_HOME="$data" XDG_RUNTIME_DIR="$runtime"
echo "mac-dev: data $XDG_DATA_HOME/hive, runtime $XDG_RUNTIME_DIR/hive"
exec bun run tauri dev
