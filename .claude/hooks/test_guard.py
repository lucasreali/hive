#!/usr/bin/env python3
"""Checks guard.py against command lines it must block and must allow: python3 .claude/hooks/test_guard.py"""
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
]


def main() -> None:
    for command in BLOCKED:
        assert bash(command), f"should block: {command!r}"
    for command in ALLOWED:
        assert bash(command) is None, f"should allow: {command!r} ({bash(command)})"
    for path in ("docs/hive.md", "/r/docs/prototype/a.html", "Cargo.lock", "/r/bun.lock"):
        assert edit(path), f"should block editing {path}"
    for path in ("docs/architecture.md", "TODO.md", "/r/docs/hive.md.bak"):
        assert edit(path) is None, f"should allow editing {path}"
    assert blocked({"tool_name": "Read", "tool_input": {"file_path": "docs/hive.md"}}) is None
    print(f"guard: {len(BLOCKED)} blocked and {len(ALLOWED)} allowed command lines ok")


if __name__ == "__main__":
    main()
