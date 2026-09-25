#!/usr/bin/env python3
"""PreToolUse guard for the CLAUDE.md hard rules: exit 2 blocks the tool call."""
import json
import re
import sys

READ_ONLY = re.compile(r"(^|/)(docs/hive\.md|docs/prototype/.*|Cargo\.lock|bun\.lock)$")
BASH_RULES = [
    (re.compile(r"(^|[;&|(]\s*)(npm|npx|pnpm|yarn)\b"), "frontend packages go through bun only (rule 3)"),
    (re.compile(r"\bgit\s+push\b"), "never push; pushing is the human's (rule 4)"),
    (re.compile(r"\bgit\s+rebase\b"), "never rebase (rule 4)"),
    (re.compile(r"\bgit\b[^;&|]*\s(--force\b|--force-with-lease\b)"), "never force (rule 4)"),
    (re.compile(r"\bgit\s+config\b[^;&|]*--global"), "never change the global git config (rule 4)"),
]


def main() -> int:
    event = json.load(sys.stdin)
    tool = event.get("tool_name", "")
    args = event.get("tool_input", {})
    if tool in ("Edit", "Write", "NotebookEdit"):
        path = args.get("file_path") or args.get("notebook_path") or ""
        if READ_ONLY.search(path):
            print(f"blocked: {path} is read-only for agents (CLAUDE.md rules 1 and 3)", file=sys.stderr)
            return 2
    elif tool == "Bash":
        command = args.get("command", "")
        for pattern, reason in BASH_RULES:
            if pattern.search(command):
                print(f"blocked: {reason}", file=sys.stderr)
                return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
