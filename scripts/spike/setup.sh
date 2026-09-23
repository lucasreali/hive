#!/bin/sh
# Hooks spike (TODO 0.12): builds hive and prepares a scratch repo under /tmp/hive-spike.
# Touches nothing outside /tmp/hive-spike: no ~/.claude/settings.json, no shell config.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
spike=/tmp/hive-spike
rm -rf "$spike"
mkdir -p "$spike/bin" "$spike/records" "$spike/logs"

cargo build --release -p hive --manifest-path "$root/Cargo.toml"
cp "$root/target/release/hive" "$spike/bin/hive"
hive="$spike/bin/hive"

# WorktreeCreate/WorktreeRemove: record the call, then let hive handle it.
for event in create remove; do
    name=$(printf '%s' "$event" | sed 's/^./\U&/')
    cat > "$spike/bin/worktree-$event" <<SCRIPT
#!/bin/sh
input=\$(cat)
printf '%s' "\$input" | "$hive" hook Worktree$name --record "$spike/records/worktree-hooks.jsonl"
printf '%s' "\$input" | "$hive" worktree hook-$event
SCRIPT
    chmod +x "$spike/bin/worktree-$event"
done

# Settings: every hook event records to records/<name>.jsonl.
settings() { # $1 = name, $2 = include worktree hooks (yes/no)
    out="$spike/settings-$1.json"
    printf '{\n  "hooks": {\n' > "$out"
    sep=""
    for e in SessionStart UserPromptSubmit PreToolUse PostToolUse PostToolUseFailure PostToolBatch \
             PermissionRequest PermissionDenied Notification Stop StopFailure SubagentStart SubagentStop \
             SessionEnd PreCompact PostCompact CwdChanged; do
        printf '%s    "%s": [{"hooks": [{"type": "command", "command": "%s", "args": ["hook", "%s", "--record", "%s"], "timeout": 5}]}]' \
            "$sep" "$e" "$hive" "$e" "$spike/records/$1.jsonl" >> "$out"
        sep=",
"
    done
    if [ "$2" = yes ]; then
        for e in create remove; do
            name=$(printf '%s' "$e" | sed 's/^./\U&/')
            printf ',\n    "Worktree%s": [{"hooks": [{"type": "command", "command": "%s", "timeout": 30}]}]' \
                "$name" "$spike/bin/worktree-$e" >> "$out"
        done
    fi
    printf '\n  }\n}\n' >> "$out"
}
settings observe no
settings worktree-hooks yes

# Scratch repository with a gitignored .env, a .worktreeinclude and a worktree-isolated agent.
repo="$spike/repo"
mkdir -p "$repo/.claude/agents"
git -C "$repo" init -q -b main
printf '.env\n' > "$repo/.gitignore"
printf '.env\n' > "$repo/.worktreeinclude"
printf 'SECRET=spike\n' > "$repo/.env"
printf 'first line\n' > "$repo/notes.txt"
cat > "$repo/.claude/agents/isolated.md" <<'AGENT'
---
name: isolated
description: Makes a small file change in its own git worktree. Use when asked for the isolated agent.
isolation: worktree
tools: Bash, Read, Write, Edit
---
Do exactly the file change you are asked for, in your working directory, then report what you did.
AGENT
git -C "$repo" add -A
git -C "$repo" -c user.name=spike -c user.email=spike@example.com commit -q -m "spike: initial"

echo "Ready: $spike"
echo "Next: $here/run.sh observe   (see $here/README.md)"
