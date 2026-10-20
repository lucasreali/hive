#!/bin/sh
# The cargo `tauri dev` runs for scripts/mac-dev.sh (`--runner`, TODO 18.14): rebuilds the service
# (target/debug/hive, the app's HIVE_BRIDGE) before each build of the app, so an edit under
# crates/hive (watched through `--additional-watch-folders`) restarts the app on a new service.
# The old service ends with the app's connection; the new app's bridge starts the rebuilt one.
set -eu
cargo build -p hive
exec cargo "$@"
