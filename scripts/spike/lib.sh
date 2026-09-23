# Shared by the spike scripts (sourced, not run).

# Observation events recorded by the spike: every event Hive maps today plus the
# neighbours worth cataloguing.
SPIKE_EVENTS="SessionStart UserPromptSubmit PreToolUse PostToolUse PostToolUseFailure PostToolBatch
PermissionRequest PermissionDenied Notification Stop StopFailure SubagentStart SubagentStop
SessionEnd PreCompact PostCompact CwdChanged"

# write_settings <out.json> <hive binary> <records.jsonl> [worktree hooks bin dir]
# Every event runs `hive hook <Event> --record <records.jsonl>` (exec form, no shell).
# With a bin dir, WorktreeCreate/WorktreeRemove run <dir>/worktree-create and worktree-remove.
write_settings() {
    out=$1 hive=$2 records=$3 worktree_bin=${4:-}
    printf '{\n  "hooks": {\n' > "$out"
    sep=""
    for e in $SPIKE_EVENTS; do
        printf '%s    "%s": [{"hooks": [{"type": "command", "command": "%s", "args": ["hook", "%s", "--record", "%s"], "timeout": 5}]}]' \
            "$sep" "$e" "$hive" "$e" "$records" >> "$out"
        sep=",
"
    done
    if [ -n "$worktree_bin" ]; then
        printf ',\n    "WorktreeCreate": [{"hooks": [{"type": "command", "command": "%s", "timeout": 30}]}]' "$worktree_bin/worktree-create" >> "$out"
        printf ',\n    "WorktreeRemove": [{"hooks": [{"type": "command", "command": "%s", "timeout": 30}]}]' "$worktree_bin/worktree-remove" >> "$out"
    fi
    printf '\n  }\n}\n' >> "$out"
}
