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
. "$here/lib.sh"
write_settings "$spike/settings-observe.json" "$hive" "$spike/records/observe.jsonl"
write_settings "$spike/settings-worktree-hooks.json" "$hive" "$spike/records/worktree-hooks.jsonl" "$spike/bin"

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
