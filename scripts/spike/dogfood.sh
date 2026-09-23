#!/bin/sh
# Passive part of the hooks spike: start claude in this repository as usual, with every
# observation hook recorded (no worktree hooks, so real work is never at risk).
#   records: target/spike/records.jsonl   terminal output + timing: target/spike/logs/
# Arguments are passed to claude, e.g. `dogfood.sh --resume`.
# Nothing outside target/spike is written; ~/.claude/settings.json is not touched
# (--settings merges with your own settings).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
dir="$root/target/spike"
mkdir -p "$dir/bin" "$dir/logs"
cargo build --release -q -p hive --manifest-path "$root/Cargo.toml"
# A private copy, so rebuilding hive during the session never breaks the hooks.
cp "$root/target/release/hive" "$dir/bin/hive.new" && mv "$dir/bin/hive.new" "$dir/bin/hive"
. "$here/lib.sh"
write_settings "$dir/settings.json" "$dir/bin/hive" "$dir/records.jsonl"

cmd="claude --settings '$dir/settings.json'"
for arg in "$@"; do
    cmd="$cmd '$(printf '%s' "$arg" | sed "s/'/'\\\\''/g")'"
done
stamp=$(date +%Y%m%d-%H%M%S)
exec script -q -O "$dir/logs/$stamp.out" -T "$dir/logs/$stamp.timing" -c "$cmd"
