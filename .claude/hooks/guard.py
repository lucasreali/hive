#!/usr/bin/env python3
"""PreToolUse guard for the CLAUDE.md hard rules: exit 2 blocks the tool call.

Tests: python3 .claude/hooks/test_guard.py
"""
import json
import re
import sys

READ_ONLY = re.compile(r"(^|/)(docs/hive\.md|docs/prototype/.*|Cargo\.lock|bun\.lock)$")
BASH_RULES = [
    (re.compile(r"(^|[;&|(]\s*)(npm|npx|pnpm|yarn)\b"), "frontend packages go through bun only (rule 3)"),
    (re.compile(r"\bgit\s+rebase\b"), "never rebase (rule 4)"),
    (re.compile(r"\bgit\b[^;&|]*\s(--force\b|--force-with-lease\b)"), "never force (rule 4)"),
    (re.compile(r"\bgit\s+config\b[^;&|]*--global"), "never change the global git config (rule 4)"),
]
# Agents push only task branches, so CI runs the heavy gates (rule 4); main and tags are the human's.
# Any git command whose subcommand is push, after git's own options (`-C <dir>`, `-c <k=v>`, `--git-dir=…`).
WORD = r"""(?:"[^"]*"|'[^']*'|\S+)"""
GIT_OPTIONS = rf"(?:\s+(?:-[Cc]\s*{WORD}|--(?:git-dir|work-tree|namespace|exec-path|config-env)\s+{WORD}|-{{1,2}}[\w-]+(?:=\S*)?))*"
PUSH = re.compile(rf"""\bgit{GIT_OPTIONS}\s+["']?push["']?(?![\w-])([^;&|]*)""")
TASK_PUSH = re.compile(r"^\s+(-u\s+)?origin\s+task/[A-Za-z0-9._/-]+\s*$")

# Bash commands that write the read-only docs (rule 1), as Edit/Write on them are blocked above.
# ponytail: covers the usual shell writers (redirection, tee, sed/perl -i, cp/mv/install/ln); a
# script that opens the file itself (python -c, dd of=) is not seen.
DOC = r"docs/(?:hive\.md|prototype(?:/{rest})?)"
TARGET = (
    r"""(?<![^\s"'>=])(?:"""
    + r'"(?:[^"]*/)?' + DOC.format(rest=r'[^"]*') + r'"'
    + r"|'(?:[^']*/)?" + DOC.format(rest=r"[^']*") + r"'"
    + r"|(?:[^\s;&|'\"<>]*/)?" + DOC.format(rest=r"[^\s;&|'\"<>]*")
    + r""")(?=$|[\s;&|)<>])"""
)
SEGMENT = r"[^;&|\n]*"
END = r"\s*(?=$|[;&|)\n])"
DOC_WRITES = [
    re.compile(rf">\|?\s*{TARGET}"),
    re.compile(rf"\btee\b{SEGMENT}{TARGET}"),
    re.compile(rf"\b(?:sed|perl)\b{SEGMENT}\s(?:-[a-zA-Z]*i|--in-place){SEGMENT}{TARGET}"),
    re.compile(rf"\bmv\b{SEGMENT}{TARGET}"),
    re.compile(rf"\b(?:cp|mv|install|ln)\b{SEGMENT}\s{TARGET}{END}"),
    re.compile(rf"\b(?:cp|mv|install|ln)\b{SEGMENT}\s(?:-t\s*|--target-directory[=\s]){TARGET}"),
]


def blocked(event: dict) -> str | None:
    """The reason the tool call breaks a hard rule, or None when it may run."""
    tool = event.get("tool_name", "")
    args = event.get("tool_input", {})
    if tool in ("Edit", "Write", "NotebookEdit"):
        path = args.get("file_path") or args.get("notebook_path") or ""
        if READ_ONLY.search(path):
            return f"{path} is read-only for agents (CLAUDE.md rules 1 and 3)"
    elif tool == "Bash":
        command = args.get("command", "")
        for push in PUSH.finditer(command):
            if not TASK_PUSH.match(push.group(1)):
                return "agents push only task branches: git push [-u] origin task/<id>-<slug> (rule 4)"
        if any(write.search(command) for write in DOC_WRITES):
            return "docs/hive.md and docs/prototype/ are read-only for agents (CLAUDE.md rule 1)"
        for pattern, reason in BASH_RULES:
            if pattern.search(command):
                return reason
    return None


def main() -> int:
    reason = blocked(json.load(sys.stdin))
    if reason:
        print(f"blocked: {reason}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
