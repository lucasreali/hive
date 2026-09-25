#!/usr/bin/env bash
# PostToolUse: format the file Claude just edited, so the fmt/biome gates stay green.
# Runs from the file's own checkout, so worktrees use their own config. Never fails the edit.
export PATH=$HOME/.cargo/bin:$HOME/.bun/bin:$PATH
f=$(python3 -c 'import json,sys; print(json.load(sys.stdin).get("tool_input", {}).get("file_path", ""))')
[ -f "$f" ] || exit 0
cd "$(dirname "$f")" || exit 0
case "$f" in
  *.rs) cargo fmt --all >/dev/null 2>&1 ;;  # cargo picks each crate's edition
  *.ts|*.tsx|*.json|*.css)
    root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
    biome="$root/node_modules/.bin/biome"
    [ -x "$biome" ] || biome="$CLAUDE_PROJECT_DIR/node_modules/.bin/biome"
    (cd "$root" && "$biome" check --write --no-errors-on-unmatched "$f" >/dev/null 2>&1) ;;
esac
exit 0
