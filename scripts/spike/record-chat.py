#!/usr/bin/env python3
"""Record real `claude -p` stream-json sessions for the chat spike (TODO 7.3a, docs/spike/chat.md).

Run by the human, never by an agent. One session per scenario, in a scratch git repository
under /var/tmp (never this repository). Each scenario writes, in <out>/rec/:

    <name>.out.jsonl         stdout of claude, byte for byte (one JSON message per line)
    <name>.in.jsonl          every line this script wrote to claude's stdin
    <name>.timing.tsv        seconds since start, direction, type/subtype of each line
    <name>.stderr.txt        stderr of claude
    <name>.transcript.jsonl  a copy of the session's transcript from ~/.claude/projects
    <name>.hooks.jsonl       hook payloads, one per line (scenarios with "hooks")
    <name>.settings.json     what --settings got (scenarios with "hooks" or "settings")

Usage (from the repository root, any shell; no shell config is needed):

    python3 scripts/spike/record-chat.py                  # every scenario
    python3 scripts/spike/record-chat.py text permission  # only these
    python3 scripts/spike/record-chat.py stage8           # the Stage 8 scenarios
    python3 scripts/spike/record-chat.py --list

Options: --claude <path> (default: the first `claude` on PATH outside Hive's wrapper dir),
--model <alias> (default haiku), --out <dir> (default: a new /var/tmp/hive-chat-spike.* dir),
--model-1m / --model-200k / --thinking-model / --switch-model (see --help).

What it touches outside <out>: claude itself writes the sessions to
~/.claude/projects/<scratch repo path, every non-alphanumeric as -> as any session does; the
script prints that folder at the end so you can delete it. User settings are not loaded
(--setting-sources project,local) and no MCP server starts (--strict-mcp-config). Hooks and
settings a scenario needs come from --settings <rec>/<name>.settings.json; the hooks only append
their stdin to <rec>/<name>.hooks.jsonl.
"""

import argparse
import glob
import json
import os
import queue
import re
import shutil
import shlex
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


READY = "Reply with one word: ready. Do not use any tools."
LS = "Run the shell command `ls` and say how many entries it shows."
PRIMES = (
    "Think carefully before answering: how many primes are there between 100 and 150? "
    "Answer with the count only. Do not use any tools."
)

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
    # Stage 8. A turn is a user message, or {"control": request}: the control request is sent and
    # the next turn waits for its control_response. "model" may be a function of the options.
    # "report" names the facts the final summary prints (see FACTS).
    "context-1m": {  # 8.1: where the 1M window shows up
        "model": lambda a: a.model_1m,
        "hooks": True,
        "turns": [READY],
        "report": ["init", "window", "transcript_model", "hook_model"],
    },
    "context-opus": {  # 8.1: plain `opus` (1M natively on current models), no [1m] suffix
        "model": "opus",
        "hooks": True,
        "turns": [READY],
        "report": ["init", "window", "transcript_model", "hook_model"],
    },
    "context-200k": {  # 8.1: a 200k model
        "model": lambda a: a.model_200k,
        "hooks": True,
        "turns": [READY],
        "report": ["init", "window", "transcript_model", "hook_model"],
    },
    "auto-start": {  # 8.4: started in auto mode
        "model": lambda a: a.thinking_model,
        "args": ["--permission-mode", "auto"],
        "turns": [LS],
        "report": ["init", "asked", "stderr"],
    },
    "auto-switch": {  # 8.4: switched to auto mode by the control request
        "model": lambda a: a.thinking_model,
        "turns": [{"control": {"subtype": "set_permission_mode", "mode": "auto"}}, LS],
        "report": ["controls", "init", "asked"],
    },
    "auto-switch-haiku": {  # 8.4: the same on haiku (a model that may not offer auto mode)
        "model": "haiku",
        "turns": [{"control": {"subtype": "set_permission_mode", "mode": "auto"}}, LS],
        "report": ["controls", "init", "asked"],
    },
    "thinking-sonnet": {  # 8.7: a thinking-capable model, default settings
        "model": lambda a: a.thinking_model,
        "args": ["--effort", "high"],
        "turns": [PRIMES],
        "report": ["thinking"],
    },
    "thinking-summaries": {  # 8.7: the same with showThinkingSummaries (settings reference)
        "model": lambda a: a.thinking_model,
        "args": ["--effort", "high"],
        "settings": {"showThinkingSummaries": True},
        "turns": [PRIMES],
        "report": ["thinking"],
    },
    "model-switch": {  # 8.9: initialize's models, set_model to an alias, then to a bad name
        "turns": [
            "Say hi in one word. Do not use any tools.",
            {"control": {"subtype": "set_model", "model": lambda a: a.switch_model}},
            "Which model are you? One line. Do not use any tools.",
            {"control": {"subtype": "set_model", "model": "no-such-model-hive-spike"}},
            "Say bye in one word. Do not use any tools.",
        ],
        "report": ["models", "controls", "init", "result_models"],
    },
    "title": {  # 8.11: does a stream-json session get an ai-title? does /rename work?
        "turns": [
            "Explain in one sentence what a git worktree is. Do not use any tools.",
            "Now in one sentence: how do I remove one? Do not use any tools.",
            "/rename hive spike title",
        ],
        "report": ["title"],
    },
    "mention": {  # 8.12: does claude expand @file and @folder/ itself?
        "turns": [
            "Without using any tools, tell me the second line of @notes.txt and the files in "
            "@subdir/. If you cannot see them, say so."
        ],
        "report": ["mention"],
    },
}
STAGE8 = [n for n, spec in SCENARIOS.items() if "report" in spec]


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
    os.makedirs(os.path.join(repo, "subdir"))
    with open(os.path.join(repo, "subdir", "inner.txt"), "w") as f:
        f.write("inner file\n")
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


def settings_args(name, spec, rec):
    """--settings for this scenario: its settings, plus hooks appending stdin to <name>.hooks.jsonl."""
    settings = dict(spec.get("settings", {}))
    if spec.get("hooks"):
        log = shlex.quote(os.path.join(rec, name + ".hooks.jsonl"))
        hook = [{"hooks": [{"type": "command", "command": f"{{ cat; echo; }} >> {log}"}]}]
        events = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "SessionEnd"]
        settings["hooks"] = {event: hook for event in events}
    if not settings:
        return []
    path = os.path.join(rec, name + ".settings.json")
    with open(path, "w") as f:
        json.dump(settings, f, indent=1)
    return ["--settings", path]


def config_dir():
    return os.environ.get("CLAUDE_CONFIG_DIR") or os.path.expanduser("~/.claude")


def copy_transcript(name, sid, rec):
    """Copy the session's transcript (found by its id) to <name>.transcript.jsonl."""
    if not sid or not re.fullmatch(r"[0-9a-fA-F-]{36}", sid):
        return
    for path in glob.glob(os.path.join(config_dir(), "projects", "*", sid + ".jsonl")):
        shutil.copyfile(path, os.path.join(rec, name + ".transcript.jsonl"))
        return
    print(f"  [{name}] no transcript found for session {sid}")


def resolve(value, opts):
    """A scenario value that may depend on the options (a function of them)."""
    return value(opts) if callable(value) else value


def run(name, spec, claude, opts, repo, rec, sessions):
    cmd = [
        claude, "-p",
        "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
        "--include-partial-messages", "--include-hook-events", "--replay-user-messages",
        "--permission-prompt-tool", "stdio",
        "--model", resolve(spec.get("model", opts.model), opts),
        "--strict-mcp-config", "--setting-sources", "project,local",
        *settings_args(name, spec, rec),
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
    pending, pending_until = None, 0  # the control request the next turn waits for (at most 60 s)

    def advance():
        """Send the next turn: a user message, or a control request to wait for."""
        nonlocal pending, pending_until
        turn = turns.pop(0)
        if isinstance(turn, dict) and "control" in turn:
            pending, pending_until = f"rec-ctl-{len(turns)}", time.monotonic() + 60
            request = {k: resolve(v, opts) for k, v in turn["control"].items()}
            s.send({"type": "control_request", "request_id": pending, "request": request})
        else:
            pending = None
            s.send(user(turn))

    interrupt_at, interrupted = None, False
    s.send({"type": "control_request", "request_id": "rec-init", "request": {"subtype": "initialize", "hooks": None}})
    initialized = False
    init_deadline = time.monotonic() + 60
    deadline = time.monotonic() + SCENARIO_TIMEOUT
    while time.monotonic() < deadline:
        if not initialized and time.monotonic() > init_deadline:
            print(f"  [{name}] no initialize response in 60 s; sending the prompt anyway")
            initialized = True
            advance()
        if pending and time.monotonic() > pending_until:
            print(f"  [{name}] no reply to {pending} in 60 s; going on")
            advance()
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
                advance()
        elif kind == "control_response" and pending and (msg.get("response") or {}).get("request_id") == pending:
            reply = msg.get("response") or {}
            print(f"  [{name}] control reply: {reply.get('subtype')} {reply.get('error') or ''}".rstrip())
            advance()
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
            if turns and not pending:
                advance()
            else:
                s.close_stdin()
                deadline = min(deadline, time.monotonic() + EXIT_GRACE)
    else:
        print(f"  [{name}] timed out")
    code = s.stop()
    print(f"  [{name}] exit {code}")
    copy_transcript(name, sessions.get(name), rec)


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


def load(path):
    """The JSON objects of a .jsonl file (missing file or bad lines: skipped)."""
    if not os.path.exists(path):
        return []
    out = []
    with open(path, errors="replace") as f:
        for line in f:
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if isinstance(value, dict):
                out.append(value)
    return out


def ordered(values):
    return list(dict.fromkeys(v for v in values if v is not None))


def blocks(messages, kind):
    """Content blocks of type `kind` in the `assistant` messages."""
    for m in messages:
        if m.get("type") == "assistant":
            content = (m.get("message") or {}).get("content")
            if isinstance(content, list):
                yield from (b for b in content if isinstance(b, dict) and b.get("type") == kind)


def fact_init(r):
    inits = [m for m in r["out"] if m.get("type") == "system" and m.get("subtype") == "init"]
    seq = ordered(f"model={m.get('model')} permissionMode={m.get('permissionMode')}" for m in inits)
    return "system/init: " + (" -> ".join(seq) or "none")


def fact_window(r):
    usage = [m.get("modelUsage") or {} for m in r["out"] if m.get("type") == "result"]
    windows = {k: v.get("contextWindow") for u in usage for k, v in u.items() if isinstance(v, dict)}
    return f"result.modelUsage contextWindow: {windows or 'none'}"


def fact_transcript_model(r):
    models = ordered((e.get("message") or {}).get("model") for e in r["transcript"] if e.get("type") == "assistant")
    found = "found" if r["transcript"] else "MISSING"
    return f"transcript ({found}) message.model: {models or 'none'}"


def fact_hook_model(r):
    if not r["hooks"]:
        return "hooks: no payload recorded"
    seen = []
    for h in r["hooks"]:
        fields = {k: v for k, v in h.items() if "model" in k.lower() or "context" in k.lower()}
        seen.append(f"{h.get('hook_event_name')}: {fields or '-'}")
    keys = sorted({k for h in r["hooks"] for k in h})
    return "hooks: " + "; ".join(ordered(seen)) + f"\n    hook payload keys: {keys}"


def fact_asked(r):
    asked = [
        (m.get("request") or {}).get("tool_name")
        for m in r["out"]
        if m.get("type") == "control_request" and (m.get("request") or {}).get("subtype") == "can_use_tool"
    ]
    tools = [b.get("name") for b in blocks(r["out"], "tool_use")]
    results = [m.get("subtype") for m in r["out"] if m.get("type") == "result"]
    return f"tools used: {tools}; permission asked for: {asked}; results: {results}"


def fact_stderr(r):
    text = r["stderr"].strip()
    return f"stderr: {text[:300]!r}" if text else "stderr: empty"


def fact_controls(r):
    replies = {
        (m.get("response") or {}).get("request_id"): m.get("response") or {}
        for m in r["out"]
        if m.get("type") == "control_response"
    }
    lines = []
    for m in r["in"]:
        rid = m.get("request_id") or ""
        if m.get("type") == "control_request" and rid.startswith("rec-ctl-"):
            req = m.get("request") or {}
            args = {k: v for k, v in req.items() if k != "subtype"}
            reply = replies.get(rid)
            got = "no reply" if reply is None else f"{reply.get('subtype')}: {reply.get('error') or reply.get('response')}"
            lines.append(f"{req.get('subtype')} {args} -> {got}")
    return "controls: " + ("\n    ".join(lines) or "none")


def fact_models(r):
    for m in r["out"]:
        resp = m.get("response") or {}
        if m.get("type") == "control_response" and resp.get("request_id") == "rec-init":
            body = resp.get("response") or {}
            models = body.get("models") or []
            names = [
                f"{x.get('value')}{' (auto)' if x.get('supportsAutoMode') else ''}" if isinstance(x, dict) else x
                for x in models
            ]
            return f"initialize models: {names or 'none'}\n    initialize keys: {sorted(body)}"
    return "initialize: no reply"


def fact_result_models(r):
    msgs = ordered((m.get("message") or {}).get("model") for m in r["out"] if m.get("type") == "assistant")
    usage = [sorted((m.get("modelUsage") or {}).keys()) for m in r["out"] if m.get("type") == "result"]
    return f"assistant message.model: {msgs}; modelUsage per result: {usage}"


def fact_thinking(r):
    thinking = list(blocks(r["out"], "thinking"))
    texts = [b for b in thinking if str(b.get("thinking") or "").strip()]
    redacted = list(blocks(r["out"], "redacted_thinking"))
    deltas = [
        (m.get("event") or {}).get("delta") or {}
        for m in r["out"]
        if m.get("type") == "stream_event" and (m.get("event") or {}).get("type") == "content_block_delta"
    ]
    think = [d for d in deltas if d.get("type") == "thinking_delta"]
    think_text = [d for d in think if str(d.get("thinking") or "").strip()]
    sig = [d for d in deltas if d.get("type") == "signature_delta"]
    empty = "yes" if thinking and not texts and not think_text else "no" if thinking or think_text else "no thinking at all"
    return (
        f"thinking text empty: {empty} (thinking blocks {len(thinking)}, with text {len(texts)}; "
        f"redacted {len(redacted)}; thinking_delta {len(think)}, with text {len(think_text)}; signature_delta {len(sig)})"
    )


def fact_title(r):
    ai = [e.get("aiTitle") for e in r["transcript"] if e.get("type") == "ai-title"]
    custom = [e.get("customTitle") for e in r["transcript"] if e.get("type") == "custom-title"]
    results = [str(m.get("result") or "")[:80] for m in r["out"] if m.get("type") == "result"]
    found = "found" if r["transcript"] else "MISSING"
    return (
        f"transcript {found}; ai-title found: {'yes ' + repr(ai) if ai else 'no'}; "
        f"custom-title found: {'yes ' + repr(custom) if custom else 'no'}\n    result of each turn: {results}"
    )


def fact_mention(r):
    def mentions(entries):
        text = "\n".join(json.dumps(e) for e in entries if e.get("type") != "assistant")
        return "third line" in text, "inner.txt" in text

    replay = [m for m in r["out"] if m.get("type") == "user"]
    notes, folder = mentions(r["transcript"] + replay)
    tools = [b.get("name") for b in blocks(r["out"] + r["transcript"], "tool_use")]
    types = sorted({e.get("type") for e in r["transcript"]} - {None})
    expanded = "yes" if notes or folder else "no"
    return (
        f"@path expanded: {expanded} (file content @notes.txt: {notes}, listing @subdir/: {folder}; "
        f"tools used: {tools}; transcript entry types: {types})"
    )


FACTS = {
    "init": fact_init,
    "window": fact_window,
    "transcript_model": fact_transcript_model,
    "hook_model": fact_hook_model,
    "asked": fact_asked,
    "stderr": fact_stderr,
    "controls": fact_controls,
    "models": fact_models,
    "result_models": fact_result_models,
    "thinking": fact_thinking,
    "title": fact_title,
    "mention": fact_mention,
}


def findings(rec, names):
    """The answers the Stage 8 scenarios look for, one block per scenario (to paste back)."""
    for name in names:
        report = SCENARIOS[name].get("report")
        if not report:
            continue
        base = os.path.join(rec, name)
        stderr = open(base + ".stderr.txt", errors="replace").read() if os.path.exists(base + ".stderr.txt") else ""
        r = {
            "out": load(base + ".out.jsonl"),
            "in": load(base + ".in.jsonl"),
            "transcript": load(base + ".transcript.jsonl"),
            "hooks": load(base + ".hooks.jsonl"),
            "stderr": stderr,
        }
        print(f"[{name}]")
        for fact in report:
            print("    " + FACTS[fact](r))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("scenarios", nargs="*", help="scenarios to run (default: all)")
    ap.add_argument("--list", action="store_true", help="list the scenarios and exit")
    ap.add_argument("--claude", help="path of the claude binary")
    ap.add_argument("--model", default="haiku", help="model alias (default haiku)")
    ap.add_argument("--out", help="output directory (default: a new /var/tmp/hive-chat-spike.* dir)")
    ap.add_argument("--model-1m", default="opus[1m]", help="1M model of context-1m (default opus[1m])")
    ap.add_argument("--model-200k", default="haiku", help="200k model of context-200k (default haiku)")
    ap.add_argument(
        "--thinking-model", default="sonnet", help="model of thinking-* and auto-start/auto-switch (default sonnet)"
    )
    ap.add_argument("--switch-model", default="sonnet", help="alias model-switch switches to (default sonnet)")
    a = ap.parse_args()
    sys.stdout.reconfigure(line_buffering=True)  # progress shows up before du's output
    if a.list:
        print("\n".join(SCENARIOS) + "\nstage8 = " + " ".join(STAGE8))
        return
    names = [n for arg in a.scenarios or SCENARIOS for n in (STAGE8 if arg == "stage8" else [arg])]
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
        run(name, SCENARIOS[name], claude, a, repo, rec, sessions)
    print()
    summary(rec, names)
    if any("report" in SCENARIOS[n] for n in names):
        print("\n== Findings (paste this back to the agent)")
        print(f"claude {version}")
        findings(rec, names)
    projects = os.path.join(config_dir(), "projects", re.sub(r"[^A-Za-z0-9]", "-", os.path.realpath(repo)))
    print(f"\nDone. Recordings: {rec}")
    print(f"claude saved these sessions under {projects} (delete it when done).")
    print(f"To share: tar -C {base} -czf /var/tmp/hive-chat-spike.tar.gz rec")
    if shutil.which("du"):
        subprocess.run(["du", "-sh", rec])


if __name__ == "__main__":
    main()
