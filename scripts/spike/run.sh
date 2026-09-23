#!/bin/sh
# Starts claude in the scratch repo with the spike's hooks, recording terminal output with timing.
# Usage: run.sh observe | worktree-hooks
set -eu
spike=/tmp/hive-spike
mode=${1:?usage: run.sh observe|worktree-hooks}
[ -f "$spike/settings-$mode.json" ] || { echo "run setup.sh first" >&2; exit 1; }
stamp=$(date +%H%M%S)
cd "$spike/repo"
exec script -q -O "$spike/logs/$mode-$stamp.out" -T "$spike/logs/$mode-$stamp.timing" -c \
    "claude --settings '$spike/settings-$mode.json' --strict-mcp-config --model haiku"
