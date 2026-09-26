#!/usr/bin/env python3
"""Record real `claude -p` stream-json sessions for the chat spike (TODO 7.3a, docs/spike/chat.md).

Run by the human, never by an agent. One session per scenario, in a scratch git repository
under /var/tmp (never this repository). Each scenario writes, in <out>/rec/:

    <name>.out.jsonl    stdout of claude, byte for byte (one JSON message per line)
    <name>.in.jsonl     every line this script wrote to claude's stdin
    <name>.timing.tsv   seconds since start, direction, type/subtype of each line
    <name>.stderr.txt   stderr of claude

Usage (from the repository root, any shell; no shell config is needed):

    python3 scripts/spike/record-chat.py                  # every scenario
    python3 scripts/spike/record-chat.py text permission  # only these
    python3 scripts/spike/record-chat.py --list

Options: --claude <path> (default: the first `claude` on PATH outside Hive's wrapper dir),
--model <alias> (default haiku), --out <dir> (default: a new /var/tmp/hive-chat-spike.* dir).

What it touches outside <out>: claude itself writes the sessions to
~/.claude/projects/<scratch repo path, / and . as -> as any session does; the script prints
that folder at the end so you can delete it. User settings are not loaded
(--setting-sources project,local) and no MCP server starts (--strict-mcp-config).
"""

import argparse
import json
import os
import queue
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time
import uuid
import zlib

SCENARIO_TIMEOUT = 600  # seconds for a whole scenario
EXIT_GRACE = 60  # seconds to exit after stdin closes


def png(width=32, height=32):
    """A tiny PNG: left half red, right half blue (for the image scenario)."""
    rows = b"".join(
        b"\x00" + b"".join(b"\xff\x00\x00" if x < width // 2 else b"\x00\x00\xff" for x in range(width))
        for _ in range(height)
    )

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def image_turn():
    import base64

    data = base64.b64encode(png()).decode()
    return [
        {"type": "text", "text": "Describe this image in one sentence. Do not use any tools."},
        {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data}},
    ]


# Each scenario: prompts (user turns, sent one after each `result`), extra CLI args, and how the
# script answers permission requests ("allow", "deny", "allow-then-deny", "answer-first").
SCENARIOS = {
    "text": {
        "turns": ["Reply with exactly two short sentences about git worktrees. Do not use any tools."],
    },
    "thinking": {
        "args": ["--effort", "high"],
        "turns": ["Think it through step by step before answering: what is 17 * 23 - 19? Do not use any tools."],
    },
    "tools": {
        "turns": ["Read notes.txt and list its lines. Then run the shell command `ls -la` and say how many entries it shows."],
    },
    "permission": {
        "answer": "allow-then-deny",
        "turns": [
            "Use the Write tool to create hello.txt containing 'hi'. Then run the shell command "
            "`touch made-by-bash.txt`. If a step is denied, say so and stop."
        ],
    },
    "question": {
        "answer": "answer-first",
        "turns": [
            "Before doing anything else, use the AskUserQuestion tool to ask me which language to greet "
            "me in, with the options English and Portuguese. Then greet me in that language. No other tools."
        ],
    },
    "plan": {
        "args": ["--permission-mode", "plan"],
        "answer": "allow",
        "turns": [
            "Plan how to add a CONTRIBUTING.md file to this repository with a two-line section about "
            "commit messages. Keep the plan short and present it for approval, then carry it out."
        ],
    },
    "subagent": {
        "args": ["--forward-subagent-text"],
        "answer": "allow",
        "turns": [
            "Use the Agent tool with the general-purpose subagent to count the lines of notes.txt and "
            "report the number back. Do not read the file yourself."
        ],
    },
    "compaction": {
        "turns": [
            "Remember the word 'pineapple'. Reply only with OK.",
            "/compact",
            "Which word did I ask you to remember?",
        ],
    },
    "error-model": {
        "model": "no-such-model-hive-spike",
        "env": {"CLAUDE_CODE_STARTUP_FAILURE_RESULTS": "1"},
        "turns": ["Say hi."],
    },
    "error-max-turns": {
        "args": ["--max-turns", "1"],
        "turns": ["Read notes.txt, then read README.md, then summarize both in one line."],
    },
    "image": {
        "turns": [image_turn()],
    },
    "interrupt": {
        "answer": "allow",
        "interrupt_after_tool": 5,  # seconds after the first tool_use starts
        "turns": [
            "Run the shell command `sleep 40` and then say done.",
            "What happened to the previous command?",
        ],
    },
    "resume": {
        "resume_from": "text",
        "turns": ["What did you tell me in your previous answer? One line."],
    },
}


def find_claude(explicit):
    if explicit:
        return explicit
    hive_bin = os.path.realpath(os.path.expanduser("~/.local/share/hive/bin"))
    for d in os.environ.get("PATH", "").split(os.pathsep):
        if not d or os.path.realpath(d) == hive_bin:
            continue  # Hive's wrapper adds Hive's hooks; record the plain CLI
        p = os.path.join(d, "claude")
        if os.path.isfile(p) and os.access(p, os.X_OK):
            return p
    sys.exit("record-chat: no `claude` on PATH; pass --claude <path>")


def git(repo, *args):
    subprocess.run(
        ["git", "-C", repo, "-c", "user.name=spike", "-c", "user.email=spike@example.invalid", *args],
        check=True,
        stdout=subprocess.DEVNULL,
    )


def make_repo(repo):
    os.makedirs(repo)
    with open(os.path.join(repo, "notes.txt"), "w") as f:
        f.write("first line\nsecond line\nthird line\n")
    with open(os.path.join(repo, "README.md"), "w") as f:
        f.write("# Scratch\n\nA scratch repository for the Hive chat spike.\n")
    with open(os.path.join(repo, "picture.png"), "wb") as f:
        f.write(png())
    git(repo, "init", "-q", "-b", "main")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "scratch")


def label(line):
    try:
        m = json.loads(line)
    except ValueError:
        return "not-json"
    kind = m.get("type", "?")
    sub = m.get("subtype") or (m.get("request") or {}).get("subtype") or (m.get("response") or {}).get("subtype")
    if kind == "stream_event":
        sub = (m.get("event") or {}).get("type")
    return f"{kind}/{sub}" if sub else kind


class Session:
    def __init__(self, name, cmd, cwd, env, rec):
        self.name, self.start = name, time.monotonic()
        base = os.path.join(rec, name)
        self.out = open(base + ".out.jsonl", "w")
        self.inp = open(base + ".in.jsonl", "w")
        self.timing = open(base + ".timing.tsv", "w")
        self.err = open(base + ".stderr.txt", "w")
        self.proc = subprocess.Popen(
            cmd, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.err,
            text=True, bufsize=1, start_new_session=True,
        )
        self.lines = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def _log(self, direction, line):
        self.timing.write(f"{time.monotonic() - self.start:.3f}\t{direction}\t{label(line)}\n")
        self.timing.flush()

    def send(self, message):
        line = json.dumps(message)
        self.inp.write(line + "\n")
        self.inp.flush()
        self._log("in", line)
        try:
            self.proc.stdin.write(line + "\n")
            self.proc.stdin.flush()
        except (BrokenPipeError, ValueError):
            print(f"  [{self.name}] stdin closed, could not send {label(line)}")

    def next(self, timeout):
        try:
            line = self.lines.get(timeout=timeout)
        except queue.Empty:
            return "", None
        if line is None:
            return None, None
        self.out.write(line if line.endswith("\n") else line + "\n")
        self.out.flush()
        self._log("out", line)
        try:
            return line, json.loads(line)
        except ValueError:
            return line, None

    def close_stdin(self):
        if not self.proc.stdin.closed:
            self.proc.stdin.close()
            self._log("in", '{"type":"eof"}')

    def stop(self):
        """Wait for claude to exit; escalate SIGINT, SIGTERM, SIGKILL to its process group."""
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGKILL, None):
            try:
                self.proc.wait(timeout=10)
                break
            except subprocess.TimeoutExpired:
                if sig is None:
                    break
                print(f"  [{self.name}] sending {sig.name}")
                try:
                    os.killpg(self.proc.pid, sig)
                except ProcessLookupError:
                    pass
        for f in (self.out, self.inp, self.timing, self.err):
            f.close()
        return self.proc.returncode


def user(content):
    return {
        "type": "user",
        "uuid": str(uuid.uuid4()),
        "message": {"role": "user", "content": content},
        "parent_tool_use_id": None,
        "session_id": "default",
    }


def answer(policy, request, count):
    """The control_response body for a can_use_tool request."""
    tool, data = request.get("tool_name"), request.get("input") or {}
    if tool == "AskUserQuestion":
        answers = {q["question"]: q["options"][0]["label"] for q in data.get("questions", []) if q.get("options")}
        return {"behavior": "allow", "updatedInput": {**data, "answers": answers}}
    if policy == "deny" or (policy == "allow-then-deny" and count > 1):
        return {"behavior": "deny", "message": "Denied by the recording script."}
    return {"behavior": "allow", "updatedInput": data}


def run(name, spec, claude, model, repo, rec, sessions):
    cmd = [
        claude, "-p",
        "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
        "--include-partial-messages", "--include-hook-events", "--replay-user-messages",
        "--permission-prompt-tool", "stdio",
        "--model", spec.get("model", model),
        "--strict-mcp-config", "--setting-sources", "project,local",
        *spec.get("args", []),
    ]
    if "resume_from" in spec:
        sid = sessions.get(spec["resume_from"])
        if not sid:
            print(f"  [{name}] skipped: scenario {spec['resume_from']!r} gave no session id")
            return
        cmd += ["--resume", sid]
    # Drop Hive's variables and the markers of a surrounding Claude Code session; keep the rest
    # (e.g. CLAUDE_CODE_OAUTH_TOKEN) so claude logs in as usual.
    env = {
        k: v
        for k, v in os.environ.items()
        if not k.startswith("HIVE_") and k not in ("CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT")
    }
    env["CLAUDE_CODE_SDK_READS_SESSION_STATE"] = "1"  # undocumented: session_state_changed frames
    env.update(spec.get("env", {}))
    with open(os.path.join(rec, name + ".cmd.json"), "w") as f:
        json.dump(cmd, f)

    s = Session(name, cmd, repo, env, rec)
    turns = list(spec["turns"])
    policy, asked = spec.get("answer", "allow"), 0
    interrupt_at, interrupted = None, False
    s.send({"type": "control_request", "request_id": "rec-init", "request": {"subtype": "initialize", "hooks": None}})
    initialized = False
    init_deadline = time.monotonic() + 60
    deadline = time.monotonic() + SCENARIO_TIMEOUT
    while time.monotonic() < deadline:
        if not initialized and time.monotonic() > init_deadline:
            print(f"  [{name}] no initialize response in 60 s; sending the prompt anyway")
            initialized = True
            s.send(user(turns.pop(0)))
        if interrupt_at and not interrupted and time.monotonic() >= interrupt_at:
            interrupted = True
            s.send({"type": "control_request", "request_id": "rec-interrupt", "request": {"subtype": "interrupt"}})
        line, msg = s.next(timeout=0.5)
        if line is None:
            break  # claude exited
        if not msg:
            continue
        kind = msg.get("type")
        if kind == "control_response" and (msg.get("response") or {}).get("request_id") == "rec-init":
            if not initialized:
                initialized = True
                s.send(user(turns.pop(0)))
        elif kind == "control_request":
            req = msg.get("request") or {}
            if req.get("subtype") == "can_use_tool":
                asked += 1
                body = answer(policy, req, asked)
                print(f"  [{name}] permission for {req.get('tool_name')}: {body['behavior']}")
                s.send({"type": "control_response", "response": {"subtype": "success", "request_id": msg.get("request_id"), "response": body}})
            else:
                s.send({"type": "control_response", "response": {"subtype": "error", "request_id": msg.get("request_id"), "error": "not supported by the recording script"}})
        elif kind == "system" and msg.get("subtype") == "init":
            sessions[name] = msg.get("session_id")
        elif kind == "assistant" and spec.get("interrupt_after_tool") and interrupt_at is None:
            if any(b.get("type") == "tool_use" for b in (msg.get("message") or {}).get("content", [])):
                interrupt_at = time.monotonic() + spec["interrupt_after_tool"]
        elif kind == "result":
            if turns:
                s.send(user(turns.pop(0)))
            else:
                s.close_stdin()
                deadline = min(deadline, time.monotonic() + EXIT_GRACE)
    else:
        print(f"  [{name}] timed out")
    code = s.stop()
    print(f"  [{name}] exit {code}")


def summary(rec, names):
    for name in names:
        path = os.path.join(rec, name + ".timing.tsv")
        if not os.path.exists(path):
            continue
        counts = {}
        with open(path) as f:
            for row in f:
                _, direction, kind = row.rstrip("\n").split("\t")
                if direction == "out":
                    counts[kind] = counts.get(kind, 0) + 1
        print(f"{name}: " + ", ".join(f"{k}×{v}" for k, v in sorted(counts.items())))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("scenarios", nargs="*", help="scenarios to run (default: all)")
    ap.add_argument("--list", action="store_true", help="list the scenarios and exit")
    ap.add_argument("--claude", help="path of the claude binary")
    ap.add_argument("--model", default="haiku", help="model alias (default haiku)")
    ap.add_argument("--out", help="output directory (default: a new /var/tmp/hive-chat-spike.* dir)")
    a = ap.parse_args()
    sys.stdout.reconfigure(line_buffering=True)  # progress shows up before du's output
    if a.list:
        print("\n".join(SCENARIOS))
        return
    names = a.scenarios or list(SCENARIOS)
    unknown = [n for n in names if n not in SCENARIOS]
    if unknown:
        sys.exit(f"record-chat: unknown scenario(s): {', '.join(unknown)} (see --list)")
    if "resume" in names and "text" not in names:
        names.insert(names.index("resume"), "text")
    claude = find_claude(a.claude)
    base = tempfile.mkdtemp(prefix="hive-chat-spike.", dir="/var/tmp") if not a.out else a.out
    repo, rec = os.path.join(base, "repo"), os.path.join(base, "rec")
    os.makedirs(rec, exist_ok=True)
    if not os.path.exists(repo):
        make_repo(repo)
    version = subprocess.run([claude, "--version"], capture_output=True, text=True).stdout.strip()
    with open(os.path.join(rec, "version.txt"), "w") as f:
        f.write(version + "\n")
    print(f"claude: {claude} ({version})\nrecordings: {rec}\n")
    sessions = {}
    for name in names:
        print(f"== {name}")
        run(name, SCENARIOS[name], claude, a.model, repo, rec, sessions)
    print()
    summary(rec, names)
    projects = os.path.expanduser("~/.claude/projects/") + os.path.realpath(repo).replace("/", "-").replace(".", "-")
    print(f"\nDone. Recordings: {rec}")
    print(f"claude saved these sessions under {projects} (delete it when done).")
    print(f"To share: tar -C {base} -czf /var/tmp/hive-chat-spike.tar.gz rec")
    if shutil.which("du"):
        subprocess.run(["du", "-sh", rec])


if __name__ == "__main__":
    main()
