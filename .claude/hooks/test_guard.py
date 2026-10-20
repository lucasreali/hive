#!/usr/bin/env python3
"""Checks guard.py against command lines it must block and must allow: python3 .claude/hooks/test_guard.py"""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from guard import blocked  # noqa: E402


def bash(command: str) -> str | None:
    return blocked({"tool_name": "Bash", "tool_input": {"command": command}})


def edit(path: str) -> str | None:
    return blocked({"tool_name": "Edit", "tool_input": {"file_path": path}})


BLOCKED = [
    # push: anything but a task branch, whatever git options come first
    "git push",
    "git push origin main",
    "git push origin v0.4.0",
    "git push --tags",
    "git push -u origin task/x main",
    "git -C . push origin main",
    "git -C /home/lucas/dev/projects/hive push",
    'git -C "a dir" push origin main',
    "git -c push.default=current push",
    "git -c x=y -C . push origin main",
    "git --git-dir=.git push origin main",
    "git --git-dir .git push origin main",
    "git --no-pager push origin main",
    "/usr/bin/git push origin main",
    "cd x && git push origin main",
    "git 'push' origin main",
    # read-only docs written from the shell
    "echo x > docs/hive.md",
    "echo x >docs/hive.md",
    "echo x >> docs/hive.md",
    "echo x >| docs/hive.md",
    "echo x 2> ./docs/hive.md",
    "echo x > /home/lucas/dev/projects/hive/docs/hive.md",
    "echo x > docs/prototype/HiveApp.dc.html",
    'echo x > "docs/prototype/Hive Protótipo.dc.html"',
    "echo x | tee docs/hive.md",
    "echo x | tee -a docs/prototype/a.html > /dev/null",
    "sed -i 's/a/b/' docs/hive.md",
    "sed -Ei 's/a/b/' docs/hive.md",
    "sed --in-place 's/a/b/' docs/prototype/a.html",
    "perl -pi -e 's/a/b/' docs/hive.md",
    "cp /tmp/x docs/hive.md",
    "cp -r /tmp/proto docs/prototype",
    "cp /tmp/x docs/prototype/",
    'cp /tmp/x "docs/prototype/Hive Protótipo.dc.html"',
    "cp -t docs/prototype /tmp/x",
    "mv /tmp/x docs/hive.md && ls",
    "mv docs/hive.md /tmp/x",
    "ln -sf /tmp/x docs/hive.md",
    # APFS is case-insensitive, and `.` / `//` reach the same file
    "echo x > docs/HIVE.md",
    "echo x > DOCS/Hive.md",
    "echo x > docs//hive.md",
    "echo x > docs/./hive.md",
    "cp a docs/Hive.md",
    "cp a docs/Prototype/x.html",
    "SED -i 's/a/b/' docs/Hive.md",
    # package managers, whatever their case and whatever runs them
    "NPM install",
    "Npx foo",
    "env npm i",
    "env -i FOO=1 npm i",
    "FOO=1 npm i",
    "command npx x",
    "command -p npx x",
    "bunx npm i",
    "bunx --bun yarn add x",
    "bun x pnpm i",
    "exec npm i",
    "sudo npm i -g x",
    "/usr/local/bin/npm i",
    "echo $(npm bin)",
    "echo `npm bin`",
    "cargo test\nnpm i",
    "GIT push origin main",
    "Git Rebase main",
    # the rules that were already there
    "npm install",
    "bun install; npx foo",
    "git rebase main",
    "git push --force origin task/x",
    "git config --global user.name x",
]

ALLOWED = [
    "scripts/ci.sh push",
    "./scripts/ci.sh push",
    "git push origin task/9.6-supply-chain",
    "git push -u origin task/9.6-supply-chain",
    "git -C . push -u origin task/9.6-supply-chain",
    "git stash push -u -m tag",
    "git log --grep push",
    "git status && git log --oneline -3",
    "git diff main -- docs/hive.md",
    "git show main:docs/hive.md > /tmp/hive.md",
    "cat docs/hive.md",
    "grep -n 'x' docs/hive.md docs/prototype/*.html",
    "sed -n '1,20p' docs/hive.md",
    "sed 's/a/b/' docs/hive.md > /tmp/out",
    "cp docs/hive.md /tmp/hive.md",
    'cp "docs/prototype/Hive Protótipo.dc.html" /tmp/',
    "ls docs/prototype/ > /tmp/list",
    "cat docs/hive.md | tee /tmp/copy",
    "echo x > docs/hive.md.bak",
    "echo x > mydocs/hive.md",
    "echo x > docs/prototype-notes.md",
    "cp a b\ncat docs/hive.md",
    "bun install --frozen-lockfile",
    "cargo add -p hive serde",
    "bunx playwright install webkit chromium",
    "bunx biome check .",
    "env FOO=1 cargo test",
    "command -v cargo",
    "cat npm-debug.log",
    "grep -rn npm docs/",
    "echo x > docs/history/stage-17.md",
    "echo x > docs/HISTORY/stage-17.md",
    "echo x > docs/hive-notes.md",
    "cp a docs/Hive.md.bak",
    "echo x > TODO.md",
]

EDIT_BLOCKED = [
    "docs/hive.md",
    "/r/docs/prototype/a.html",
    "Cargo.lock",
    "/r/bun.lock",
    # APFS is case-insensitive: these are the protected files
    "DOCS/HIVE.md",
    "docs/Hive.md",
    "/r/Docs/Hive.MD",
    "docs//hive.md",
    "docs/./hive.md",
    "/r/docs/x/../hive.md",
    "Cargo.LOCK",
    "BUN.lock",
    "docs/Prototype/x.html",
]

EDIT_ALLOWED = [
    "docs/architecture.md",
    "TODO.md",
    "/r/TODO.md",
    "/r/docs/hive.md.bak",
    "docs/hive-something.md",
    "docs/history/stage-17.md",
    "/r/docs/History/Stage-17.md",
    "docs/prototype-notes.md",
    "/r/crates/hive/Cargo.toml",
    "/r/docs/hive/x.md",
    "",
]


def main() -> None:
    for command in BLOCKED:
        assert bash(command), f"should block: {command!r}"
    for command in ALLOWED:
        assert bash(command) is None, f"should allow: {command!r} ({bash(command)})"
    for path in EDIT_BLOCKED:
        assert edit(path), f"should block editing {path!r}"
    for path in EDIT_ALLOWED:
        assert edit(path) is None, f"should allow editing {path!r} ({edit(path)})"
    notebook = {"tool_name": "NotebookEdit", "tool_input": {"notebook_path": "DOCS/Prototype/a.ipynb"}}
    assert blocked(notebook), "should block a notebook edit under docs/prototype"
    assert blocked({"tool_name": "Write", "tool_input": {"file_path": "Docs/Hive.md"}})
    assert blocked({"tool_name": "Read", "tool_input": {"file_path": "docs/hive.md"}}) is None
    print(
        f"guard: {len(BLOCKED)} blocked and {len(ALLOWED)} allowed command lines,"
        f" {len(EDIT_BLOCKED)} blocked and {len(EDIT_ALLOWED)} allowed edits ok"
    )


if __name__ == "__main__":
    main()
