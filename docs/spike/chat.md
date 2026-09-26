# Chat with Claude inside the app — design spike (TODO 7.3a)

Status: **proposal for the human.** No product code. Written on 2026-09-25 from the official
documentation and the open-source Agent SDK; **no real session was recorded yet**. Every example
below is the documented shape and is marked *to be recorded* until
`scripts/spike/record-chat.py` (section 11) has been run and the examples replaced by real lines.

Sources (fetched 2026-09-25):

- Headless mode: <https://code.claude.com/docs/en/headless>
- CLI reference: <https://code.claude.com/docs/en/cli-reference>
- Agent SDK TypeScript reference (message types): <https://code.claude.com/docs/en/agent-sdk/typescript>
- Approvals and user input: <https://code.claude.com/docs/en/agent-sdk/user-input>
- Streaming output: <https://code.claude.com/docs/en/agent-sdk/streaming-output>
- Streaming input: <https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode>
- Hooks (`PreToolUse`, `ExitPlanMode`, `AskUserQuestion`): <https://code.claude.com/docs/en/hooks>
- Legal and compliance: <https://code.claude.com/docs/en/legal-and-compliance>
- Authentication: <https://code.claude.com/docs/en/authentication>
- Agent SDK and your Claude plan (Help Center, updated 2026-06-16):
  <https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan>
- Python Agent SDK source (wire protocol, the part the docs do not spell out):
  `anthropics/claude-agent-sdk-python`, `src/claude_agent_sdk/_internal/transport/subprocess_cli.py`,
  `_internal/query.py`, `types.py`, `client.py` (branch `main`).

---

## 1. Summary and recommendation

- **It is possible without the SDK package and without Node.** The chat is the official
  `claude` binary in headless mode, `claude -p --input-format stream-json --output-format stream-json`,
  spawned by the Rust service with pipes (no PTY). The Agent SDK itself is only a wrapper that
  spawns this same command and speaks this same newline-delimited JSON on stdin/stdout.
- **Hive becomes the permission prompt.** With `--permission-prompt-tool stdio`, every tool
  approval, every `AskUserQuestion` and every plan approval (`ExitPlanMode`) arrives as a
  `control_request` that the service must answer. Nothing runs until the human clicks.
- **This reverses the core concept** of `docs/hive.md` (section 2). Terminals can stay exactly
  as they are (observed only); the chat is a new kind of tab next to them.
- **Billing (section 8):** today `claude -p` with the human's own login draws from the Pro/Max
  subscription limits, like the terminal. Anthropic announced, then **paused** (2026-06-15), a
  split that would move `claude -p` and SDK usage to a separate monthly credit. If it comes back,
  chat usage would be billed differently from terminal usage: that is the same risk that got the
  SDK revoked. **The human must accept this risk before 7.3b.**
- **The wire protocol for permissions and interrupts is not in the CLI reference.** It is the
  SDK's protocol, read from the open-source SDK. It is stable enough for the SDK to depend on it,
  but it can change with a Claude Code release (section 10).

Recommendation: approve the chat as an **optional** tab, keep the terminals observe-only, run
the recording script, then implement 7.3b–7.3i (section 14). If the billing split returns, the
fallback is option B (section 12): answer permissions and questions of the **interactive**
`claude` from the app through hooks, which keeps interactive billing.

---

## 2. What this changes in `docs/hive.md`

| Where in `docs/hive.md` | What it says today | Conflict |
|---|---|---|
| 🧭 Conceito central, 1st paragraph | "O Hive **observa** os agentes… Ele **não controla e não conversa** com os agentes: toda interação com o agente acontece no terminal." | The chat talks to Claude and answers its permission prompts. |
| Conceito central, consequences | "**Sem Claude Agent SDK.** O uso é o `claude` normal, dentro dos limites da assinatura." | No SDK package is used, but the chat uses the SDK's wire protocol and falls under the SDK billing note (section 8). |
| Conceito central, consequences | "O Hive **pode editar arquivos** (Etapa 3b), mas nunca age sobre o agente." | The chat acts on the agent. |
| Decisão #1 | "Claude Code interativo em terminal embutido… sem dependência nem cobrança do SDK" | The chat is headless, not interactive. |
| Decisão #6 | "Serviço no WSL 100% Rust — Sem SDK, o Node deixou de ser necessário" | Still true: the chat needs no Node (the service spawns `claude` directly). |
| Decisões revogadas, "SDK (antiga #1)" | "Rodar agentes via Claude Agent SDK — Hive virou observador…; elimina o risco de cobrança do SDK" | Rule 7 of `hive.md`: a revoked decision needs a new reason. New reason: the human asked for an in-app chat (2026-09-25), and it uses the unmodified `claude` binary, not the SDK package. The billing risk is **not** eliminated (section 8). |
| Decisões revogadas, "Node (antiga #6)" | "Mini serviço Node para o SDK" | Stays revoked. |
| Fase 2, item 2 | "**Interações ricas**: responder permissões pelo app…, perguntas de múltipla escolha como botões" | The chat brings these into v0.3, for chat tabs only. |
| Adições manuais, "Orquestração de agentes" | "**Descartado**: conflita com o Hive como observador" | Not affected: the chat is one conversation driven by the human, not orchestration. |
| Estados, regra 2 | Silence of the PTY turns 🔵/🟡 into 🟠 | A chat has no PTY; its state comes from the stream (section 5.4). |
| #18, #20, #21 | Terminals die with the app; live state only from Hive terminals; hooks injected through the wrapper's `--settings` | All carry over to chats (sections 5.1, 5.4). |

---

## 3. The transport

### 3.1 Command

Built by the service as an argument vector (never a shell string):

```
<real claude> -p
  --input-format stream-json --output-format stream-json --verbose
  --include-partial-messages          # stream_event deltas (live text)
  --replay-user-messages              # echo of what we sent, with uuid
  --permission-prompt-tool stdio      # permission prompts as control_request on stdout
  --permission-mode <default|acceptEdits|plan|auto>
  --settings <hive hooks settings>    # the same file the wrapper passes (#21)
  [--model <alias>] [--resume <session id>] [--forward-subagent-text]
```

- `<real claude>` is what `wrapper::real_claude` already finds (the first `claude` on `PATH`
  outside Hive's bin dir). The chat does not go through the `sh` wrapper, so it passes
  `--settings` itself, with `HIVE_WRAPPED=1` in the environment.
- `--verbose` is required with `stream-json` output (headless docs, "Stream responses").
- **Never `--bare`**: bare mode never reads the OAuth login, so it would need an API key
  (headless docs, "Start faster with bare mode").
- `--permission-prompt-tool stdio` is what the Python SDK passes when a `can_use_tool` callback is
  set (`types.py`, `_configure_can_use_tool`). The CLI reference documents the flag only for an MCP
  tool name; the `stdio` value is SDK protocol (section 10). *To be recorded.*
- `--forward-subagent-text` adds subagents' text and thinking (v2.1.211+); without it the stream
  carries only their tool calls.

### 3.2 stdin (service → claude), one JSON object per line

```jsonc
// First line, as the SDK does: registers nothing, returns commands/models/account (section 4.1).
{"type":"control_request","request_id":"hive-1","request":{"subtype":"initialize","hooks":null}}
// A user turn (text, or content blocks with images: section 4.11).
{"type":"user","uuid":"<uuid v4>","message":{"role":"user","content":"Fix the failing test"},
 "parent_tool_use_id":null,"session_id":"default"}
// Answer to a permission request (section 4.5).
{"type":"control_response","response":{"subtype":"success","request_id":"<from the request>",
 "response":{"behavior":"allow","updatedInput":{...}}}}
// Stop the turn (section 4.13).
{"type":"control_request","request_id":"hive-2","request":{"subtype":"interrupt"}}
// Switch permission mode / model while running.
{"type":"control_request","request_id":"hive-3","request":{"subtype":"set_permission_mode","mode":"acceptEdits"}}
{"type":"control_request","request_id":"hive-4","request":{"subtype":"set_model","model":"sonnet"}}
```

Messages sent while a turn runs are queued and may be merged into the next turn
(`user_message_uuids` on the result). Closing stdin ends the session after the current turn.

### 3.3 stdout (claude → service)

One JSON object per line. Every message has `type`, and most have `uuid` and `session_id`.
Besides the conversation (section 4) there are:

- `control_response` — answers to our `control_request`s, matched by `response.request_id`
  (`subtype: "success"` with `response`, or `"error"` with `error`).
- `control_request` — requests **to us**: `can_use_tool` (permissions, questions, plans);
  `hook_callback` and `mcp_message` only if we register SDK hooks or SDK MCP servers (we do not;
  answer them with an error response).
- `control_cancel_request` — claude abandoned a request it sent (e.g. after an interrupt): drop
  the card, do not answer it.

### 3.4 Exit

Exit 0 after stdin closes and the last result is written. Non-zero on failure; a failure inside
the run is still printed as a `result` on stdout. SIGTERM → exit 143, turn left unfinished;
SIGINT ends the turn first (headless docs, "Stop a run with SIGTERM").

---

## 4. Catalogue of what the stream returns

Each entry: shape (documented, *to be recorded*), what the service does, what the UI shows.
Unknown `type`/`subtype` values are ignored by the service (the list is open and grows with every
Claude Code release).

### 4.1 Session start — `system/init` and the `initialize` response

```jsonc
// to be recorded
{"type":"system","subtype":"init","uuid":"…","session_id":"9f1c…","cwd":"/home/u/proj",
 "model":"claude-…","permissionMode":"default","apiKeySource":"none",
 "tools":["Bash","Read","Edit",…],"mcp_servers":[{"name":"…","status":"connected"}],
 "slash_commands":["compact","clear",…],"agents":["general-purpose",…],"skills":[…],
 "plugins":[{"name":"…","path":"…"}],"output_style":"default","claude_code_version":"2.1.…",
 "capabilities":["interrupt_receipt_v1","interrupt_cancel_queued_v1"]}
```

The `initialize` `control_response` carries `commands`, `agents`, `models`, `account`,
`output_style`, `fast_mode_state` and `pending_permission_requests` (v2.1.268+).
Hook events of `SessionStart` may come before `init` (`system/hook_started|hook_progress|hook_response`).

- **Service:** keeps `session_id` (for resume), `model`, `permissionMode`, `apiKeySource`,
  `claude_code_version`, `capabilities`, `slash_commands`. Checks `apiKeySource` (section 8).
  Refuses to go on with a clear error if `capabilities` lacks what it relies on (section 10).
- **UI:** header shows model and permission mode; `slash_commands` feed the composer's `/` list.
  Nothing else is shown.

### 4.2 Assistant text — `assistant`

```jsonc
// to be recorded
{"type":"assistant","uuid":"…","session_id":"…","parent_tool_use_id":null,
 "message":{"id":"msg_…","role":"assistant","model":"…","stop_reason":null,
            "content":[{"type":"text","text":"The test fails because…"}],
            "usage":{"input_tokens":…,"output_tokens":…,"cache_read_input_tokens":…}}}
```

One `assistant` message **per content block**; blocks of one API response share `message.id`
(streaming-output docs, "Message flow"). `error` (`rate_limit`, `billing_error`, `model_not_found`, …)
and `aborted: true` (cut by an interrupt) are optional fields.

- **Service:** one entry per block: `{kind: text, role: assistant, text}`. Text is cut at a cap
  (section 5.5). Usage is taken from here like 6.9 does from transcripts.
- **UI:** a message bubble; Markdown if a renderer is approved (question Q5), plain text otherwise.

### 4.3 Thinking — `assistant` with a `thinking` block, `system/thinking_tokens`

```jsonc
// to be recorded
{"type":"assistant", …, "message":{…,"content":[{"type":"thinking","thinking":"Let me check…","signature":"…"}]}}
{"type":"system","subtype":"thinking_tokens","estimated_tokens":812,"estimated_tokens_delta":64,"uuid":"…","session_id":"…"}
```

`redacted_thinking` blocks carry no readable text. Whether thinking text is summarized or empty
depends on the model and version: *to be recorded* (scenario `thinking`).

- **Service:** entry `{kind: thinking, text}` (cut at the cap); `signature` dropped.
  `thinking_tokens` becomes a progress counter, not an entry.
- **UI:** a collapsed "Thinking…" row (muted, expandable), with the running token estimate while
  it streams. 6.10 left thinking out of subagent transcripts; the chat shows it collapsed.

### 4.4 Tool calls and results — `tool_use` / `tool_result`, `tool_progress`

```jsonc
// to be recorded
{"type":"assistant", …, "message":{…,"content":[{"type":"tool_use","id":"toolu_1","name":"Bash",
  "input":{"command":"cargo test -p hive","description":"Run tests"}}]}}
{"type":"user","parent_tool_use_id":null,"message":{"role":"user","content":[{"type":"tool_result",
  "tool_use_id":"toolu_1","content":"test result: ok…","is_error":false}]},
  "tool_use_result":{"stdout":"…","stderr":"","interrupted":false}}
{"type":"tool_progress","tool_use_id":"toolu_1","tool_name":"Bash","parent_tool_use_id":null,
 "elapsed_time_seconds":30,"heartbeat":true,"uuid":"…","session_id":"…"}
```

`tool_use_result` is the tool's structured output (shape per tool, SDK reference "Tool Output
Types"); `content` is what the model saw. `tool_progress` heartbeats come every 30 s during a
long call. `tool_use_summary` may summarize several calls.

- **Service:** entry `{kind: tool, tool, summary, id}` where `summary` is a one-line rendering of
  the input (the `command` of Bash, `file_path` of Read/Edit/Write, `pattern` of Grep…; the rest as
  compact JSON, cut). The result is attached to the same entry by `tool_use_id`: `{status: ok|error,
  output}` (cut; images in results: section 4.11). Heartbeats update an "elapsed" field, no entry.
- **UI:** the 6.10 tool row (name + summary), expandable to the output; red when `is_error`; a
  spinner with elapsed seconds while running.

### 4.5 Permission requests — `control_request` `can_use_tool`

```jsonc
// to be recorded
{"type":"control_request","request_id":"req_7",
 "request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf target"},
            "tool_use_id":"toolu_2","agent_id":null,"blocked_path":null,
            "decision_reason":"…","title":"…","display_name":"…","description":"…",
            "permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"rm -rf target"}],
                                        "behavior":"allow","destination":"localSettings"}]}}
```

Answer (only one per `request_id`; permission prompts never time out):

```jsonc
{"type":"control_response","response":{"subtype":"success","request_id":"req_7",
 "response":{"behavior":"allow","updatedInput":{"command":"rm -rf target"}}}}
{"type":"control_response","response":{"subtype":"success","request_id":"req_7",
 "response":{"behavior":"deny","message":"The user denied this.","interrupt":false}}}
```

`updatedPermissions` (from `permission_suggestions`) makes an "always allow" rule; with the
`localSettings` destination it writes `.claude/settings.local.json` in the project.
Only calls that would prompt reach us: allow rules and the permission mode decide first.
Denials decided without us arrive as `system/permission_denied` — except that, per the SDK reference,
this event is **not emitted at all** when an MCP prompt tool is set; whether `stdio` counts as one
is *to be recorded*. The result's `permission_denials` is the authoritative list.

- **Service:** keeps a table of pending requests `{request_id → chat, tool_use_id, kind}`; sends
  the app a card; forwards the app's answer **only** for a pending `request_id` of that chat, once;
  `control_cancel_request` or the chat ending removes it. `updatedInput` is always the input
  claude sent (the app never edits tool input). "Always allow" only if the human approves it (Q3).
  The agent's state becomes 🟡 while any request is pending.
- **UI:** a card inline in the conversation: tool name, the **full** command or path (never
  truncated silently), `decision_reason`, buttons **Allow** / **Deny** (optional message) and, if
  approved, **Always allow** showing the exact rule. Keyboard accessible; focus moves to the card.

### 4.6 Multiple-choice questions — `can_use_tool` for `AskUserQuestion`

```jsonc
// to be recorded
{"type":"control_request","request_id":"req_8","request":{"subtype":"can_use_tool",
 "tool_name":"AskUserQuestion","tool_use_id":"toolu_3","input":{"questions":[
   {"question":"Which language should I greet you in?","header":"Language","multiSelect":false,
    "options":[{"label":"English","description":"…"},{"label":"Portuguese","description":"…"}]}]}}}
```

1–4 questions, 2–4 options each; `header` ≤ 12 chars. Answer by echoing the questions with
`answers` (question text → label, or an array of labels for `multiSelect`; free text for "Other"):

```jsonc
{"type":"control_response","response":{"subtype":"success","request_id":"req_8","response":{
 "behavior":"allow","updatedInput":{"questions":[…same…],
   "answers":{"Which language should I greet you in?":"Portuguese"}}}}}
```

`AskUserQuestion` is offered in `-p` only when there is a permission host (hooks docs); it is not
available inside subagents. Option `preview` (HTML/Markdown) only appears if the host opts in; Hive
does not (no HTML from the model is ever rendered).

- **Service:** validates the answer against the pending question (labels must exist, one label
  unless `multiSelect`, free text ≤ 4 KiB), builds `updatedInput`. State 🟡 while pending — this
  fixes, for chats, the 🟠 confusion that 7.6 studies for terminals.
- **UI:** a card with each question's `header`, text, options as buttons (checkboxes for
  `multiSelect`), an "Other…" text field, **Send**. Dismissing = deny.

### 4.7 Plan approval — `can_use_tool` for `ExitPlanMode`

```jsonc
// to be recorded
{"type":"control_request","request_id":"req_9","request":{"subtype":"can_use_tool",
 "tool_name":"ExitPlanMode","tool_use_id":"toolu_4",
 "input":{"plan":"## Add CONTRIBUTING.md\n1. …","planFilePath":"/home/u/.claude/plans/….md"}}}
```

Claude Code injects `plan` and `planFilePath` into the input (hooks docs, "ExitPlanMode"); the
model's own input is usually empty. **Allow** = approve (leaves plan mode); **deny** with a
message = keep planning with that feedback. Which permission mode follows an approval (the
interactive CLI offers "auto-accept edits" vs "manually approve") and whether a
`status`/`permissionMode` change or `conversation_reset` (`trigger: "plan_mode_exit"`) is emitted:
*to be recorded* (scenario `plan`).

- **Service:** entry `{kind: plan, text}` (cap 256 KiB) + pending request; approving may be
  followed by `set_permission_mode` if the human picks "accept edits" (Q3).
- **UI:** the plan rendered as a document card, buttons **Approve** / **Approve and accept
  edits** / **Keep planning** (with a feedback field).

### 4.8 Subagents — `parent_tool_use_id`, `system/task_*`

Messages of a subagent are ordinary `assistant`/`user` messages whose `parent_tool_use_id` is the
`Agent` tool call that started it (nested subagents point to their own parent call, v2.1.219+). The
first one is a `user` message with the subagent's prompt. Only tool calls are forwarded unless
`--forward-subagent-text` is set. Stream events (`stream_event`) are main-thread only.
Lifecycle: `system/task_started` (`task_type: "local_agent"`, `is_backgrounded`, `spawn_depth`),
`system/task_progress`, `system/task_updated`, `system/task_notification` (`completed|failed|stopped`,
`summary`, `usage`), `system/background_tasks_changed` (the full live set). The `Agent` tool's
`tool_use_result` holds the subagent's report. *To be recorded* (scenario `subagent`).

- **Service:** groups entries by `parent_tool_use_id` under the `Agent` tool entry; the subagent
  list for the sidebar comes from the hooks as today (`SubagentStart/Stop`, 5.10) and is
  cross-checked with `background_tasks_changed` (which is authoritative for background work).
- **UI:** the `Agent` call is a collapsible group showing the subagent type and state; expanded,
  it is the 6.10 transcript look inline. The sidebar is unchanged (🟣 with subagents).

### 4.9 Compaction — `system/status`, `system/compact_boundary`

```jsonc
// to be recorded
{"type":"system","subtype":"status","status":"compacting","uuid":"…","session_id":"…"}
{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"manual","pre_tokens":150231},"uuid":"…","session_id":"…"}
{"type":"system","subtype":"status","status":null, …}
```

`/compact` sent as a user turn triggers it manually; auto-compaction triggers it near the limit.
`/clear` emits `conversation_reset` with `new_conversation_id`.

- **Service:** entry `{kind: divider, text: "Conversation compacted (150k tokens)"}`; context
  tokens (6.9) reset from the next usage. `conversation_reset`: divider "Conversation cleared" and
  the new session id is kept for resume.
- **UI:** a horizontal divider with the label; "Compacting…" in the header while `status` is set.

### 4.10 Errors — `result` error subtypes, `assistant.error`, `system/api_retry`, exit code

```jsonc
// to be recorded
{"type":"system","subtype":"api_retry","attempt":1,"max_retries":10,"retry_delay_ms":2000,
 "error_status":529,"error":"overloaded","uuid":"…","session_id":"…"}
{"type":"result","subtype":"error_max_turns","is_error":true,"errors":["…"], …}
{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["…"],
 "startup_failure_reason":"cwd_unavailable", …}
{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1790000000,
 "errorCode":"credits_required"}, …}
```

Error subtypes: `error_max_turns`, `error_during_execution`, `error_max_budget_usd`,
`error_max_structured_output_retries`; a success result can still carry `is_error: true`
(API failure). `terminal_reason` says why the loop ended. A startup failure may leave only stderr
and a non-zero exit. `system/informational` carries warnings and hook messages
(`level: info|notice|suggestion|warning`).

- **Service:** error result or `assistant.error` → entry `{kind: error, text}` and state 🔴
  (like `StopFailure`); `api_retry` → a transient status ("Retrying (2/10)…"), no entry; process
  exit without a result → 🔴 with the last 4 KiB of stderr; `rate_limit_event` `rejected` →
  error entry with the reset time. `informational` → a muted note entry.
- **UI:** a red error row with the text; the composer stays usable (a new turn retries).

### 4.11 Images — input and output

Input, a user turn with content blocks (streaming-input docs):

```jsonc
{"type":"user","message":{"role":"user","content":[
  {"type":"text","text":"What is wrong in this screenshot?"},
  {"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0…"}}]},
 "parent_tool_use_id":null,"session_id":"default"}
```

Output: images come back inside `tool_result` content (e.g. `Read` of a `.png`) as the same
`image` blocks, base64. *To be recorded* (scenario `image`).

- **Service:** the app sends the image bytes (paste/drag) in the chat request; the service checks
  the media type **by content** (PNG/JPEG/GIF/WebP magic bytes), size ≤ 5 MiB decoded (limit to be
  confirmed against the API's), at most 10 per turn, and base64-encodes it. Images in tool results
  are forwarded only if ≤ 3 MiB encoded (the frame limit is 4 MiB, `hive_protocol::MAX_PAYLOAD`);
  bigger ones become "image (N KiB) not shown".
- **UI:** thumbnails in the composer before sending and in the bubble; click to enlarge. Shown
  from a `data:` URL built by the app (`src-tauri/tauri.conf.json` sets no CSP today, so nothing
  blocks it; if a CSP is added later it needs `img-src data:`).

### 4.12 Result and usage — `result`

```jsonc
// to be recorded
{"type":"result","subtype":"success","is_error":false,"result":"Done.","num_turns":3,
 "duration_ms":8123,"duration_api_ms":7011,"stop_reason":"end_turn","terminal_reason":"completed",
 "total_cost_usd":0.0123,"usage":{…},"modelUsage":{"claude-…":{…}},"permission_denials":[],
 "session_id":"…","uuid":"…","user_message_uuid":"…"}
```

Ends every turn (in streaming-input mode, one per turn). `modelUsage` and `total_cost_usd` are
cumulative across turns and include subagents; `usage` is the main loop only.
`total_cost_usd` is a client-side estimate even on a subscription.

- **Service:** turn done → state 🟠 (the stream replaces the PTY-silence rule for chats);
  `agent_usage` from `modelUsage`/`usage` (same message as 6.9). If
  `CLAUDE_CODE_SDK_READS_SESSION_STATE=1` is set (undocumented; the SDK sets it),
  `system/session_state_changed {state: idle|running|requires_action}` gives the same states
  directly: *to be recorded*.
- **UI:** a muted footer under the turn: duration, output tokens, context %. **No dollar value**
  on a subscription login (it would read like a bill; Q2).

### 4.13 Interruption — `interrupt` control request

Send `{"type":"control_request","request_id":"…","request":{"subtype":"interrupt"}}`; the
`control_response` is a receipt `{still_queued: [uuid…], cancelled?: […]}` (capability
`interrupt_receipt_v1`), then the turn's `result` follows (`terminal_reason:
"aborted_streaming"|"aborted_tools"`); a cut assistant message has `aborted: true`. With
`cancel_queued: true` (capability `interrupt_cancel_queued_v1`) queued messages are dropped too.
A running Bash command is killed. *To be recorded* (scenario `interrupt`).

- **Service:** Stop button → `interrupt` with `cancel_queued: true`; pending permission cards
  are removed on their `control_cancel_request`.
- **UI:** the composer's Send becomes **Stop** while a turn runs (Esc in the composer also stops,
  like the terminal).

### 4.14 Stream events / partial messages — `stream_event`

```jsonc
// to be recorded
{"type":"stream_event","event":{"type":"content_block_delta","index":0,
 "delta":{"type":"text_delta","text":"The te"}},"parent_tool_use_id":null,"uuid":"…","session_id":"…"}
```

Raw Messages API events: `message_start`, `content_block_start`, `content_block_delta`
(`text_delta`, `thinking_delta`, `input_json_delta`), `content_block_stop`, `message_delta`,
`message_stop`. The complete `assistant` message for a block arrives before its
`content_block_stop`. Main thread only.

- **Service:** accumulates `text_delta`/`thinking_delta` per block and sends the app the growing
  text at most every 50 ms (coalesced, one message per chat), then the final entry replaces it.
  `input_json_delta` is ignored (the tool row appears with the complete `assistant`).
- **UI:** the last bubble grows as text arrives; no per-token re-render of the list (the store
  updates one entry).

### 4.15 Everything else in the union

`user` replays of our own messages (`isReplay`, from `--replay-user-messages`) confirm a turn was
queued: the service marks the composer's message as sent. `system/hook_started|hook_progress|hook_response`
(Hive's own hooks run too), `system/plugin_install`, `system/files_persisted`,
`system/commands_changed` (refresh the `/` list), `system/worker_shutting_down`,
`system/notification`, `system/memory_recall`, `system/elicitation_complete`, `system/mirror_error`,
`auth_status`, `prompt_suggestion`, `tool_use_summary`: ignored in the first version, except
`commands_changed`. Shapes of the undocumented ones: *to be recorded*.

---

## 5. Service design (Rust)

### 5.1 Process ownership

- New module `hive::chat`: one `Chat` per chat tab, owning a child `claude` process spawned with
  `std::process::Command` (argument vector; cwd = the worktree; `setsid` so the whole group can be
  killed, like terminals, #18). Environment: the service's, plus `HIVE_TERMINAL_ID=<chat channel>`
  and `HIVE_WRAPPED=1`; minus `CLAUDECODE`/`CLAUDE_CODE_ENTRYPOINT` if present.
- **Pipes, not a PTY**: stdin (JSON lines in), stdout (JSON lines out), stderr (a 64 KiB ring
  buffer for error reports). No terminal emulation, no escape sequences.
- **Chat ids share the terminal channel space**, so `HIVE_TERMINAL_ID` binds hook events to the
  chat exactly like to a terminal (#19): the sidebar, pending counter, notifications and 6.9
  usage work unchanged.
- Dies with the app like terminals (#18): closing the app kills every chat's process group;
  `open-sessions.json` (4.12) records the chat so it comes back (5.3).

### 5.2 Reading and writing

- A reader thread per chat: `BufRead::read_until(b'\n')` on a `Take` bounded at **16 MiB per
  line**; a longer line is skipped to the next newline (bounded by the same cap per step) and
  becomes an entry "message too large, not shown". Each line: `serde_json::from_slice::<Value>`,
  then a match on `type`/`subtype` into typed entries; anything unknown is dropped (debug log with
  the type only).
- Writes go through one writer per chat (a channel), so answers and interrupts never interleave.
  A write error (claude gone) ends the chat.

### 5.3 Session resume

- `session_id` from `system/init` (and `conversation_reset.new_conversation_id`) is kept on the
  chat. Reopening = `--resume <id>`; the transcript `.jsonl` is the same one interactive sessions
  write, so the Sessions panel (4.11) lists chat sessions too, and they can also be resumed in a
  terminal.
- On app restart, `open-sessions.json` gains `kind: "terminal"|"chat"`; chats come back as chats.
- History of a resumed chat: `--resume` is not documented to replay old messages on stdout
  (*to be recorded*, scenario `resume`), so the service fills the view from the transcript file with the 6.10 reader (`hive::transcript`, same root
  checks and 8 MiB bound).

### 5.4 States

| Chat situation | Source | State |
|---|---|---|
| Process started, no turn yet | `system/init` | 🟢 idle |
| Turn running | our `user` message / `stream_event` / hooks `UserPromptSubmit`, `PreToolUse` | 🔵 working |
| Permission, question or plan pending | `control_request can_use_tool` (and hook `PermissionRequest`, *to be recorded*) | 🟡 waiting for permission/answer |
| Turn done | `result` success | 🟠 waiting for you |
| Error | `result` error / `assistant.error` / exit without result | 🔴 error |
| Subagents running | hooks `SubagentStart/Stop`, `background_tasks_changed` | 🟣 |
| Process ended | exit | ⚫ ended |

The PTY-silence rule (state rule 2) does not apply to chats: an interrupt produces a `result`.

### 5.5 Size limits and untrusted input

Everything on stdout is untrusted (model output, tool output, file contents, MCP servers):

- per stdout line 16 MiB; per entry text sent to the app 64 KiB (cut with "… (N KiB more)");
  plan 256 KiB; tool summary 500 chars; ≤ 200 entries per app message so a frame stays < 4 MiB;
  the app store keeps ≤ 2000 entries per chat (older ones reloaded from the transcript on scroll —
  or simply dropped with "Earlier messages are left out", as 6.10).
- per user turn: text ≤ 1 MiB, ≤ 10 images ≤ 5 MiB each (checked by content). Answers: labels
  must exist in the pending question; free text ≤ 4 KiB; deny message ≤ 4 KiB.
- ids (`request_id`, `tool_use_id`, `session_id`) ≤ 128 bytes, printable ASCII; `session_id`
  must be a UUID before it is ever passed to `--resume`.
- No prompt, tool input or output in logs at default level (CLAUDE.md baseline).
- The app renders every string as text (or sanitized Markdown, Q5): never HTML, never as a
  terminal.

### 5.6 Cancellation and shutdown

| Action | What the service does |
|---|---|
| Stop (button / Esc) | `interrupt` with `cancel_queued: true` |
| Close the chat tab | deny every pending request, close stdin, wait ≤ 5 s, then SIGINT to the group, ≤ 5 s, SIGTERM, ≤ 5 s, SIGKILL |
| App closes / service ends | same as terminals (#18): kill the group; the chat is in `open-sessions.json` |
| claude exits by itself | chat ⚫ (or 🔴 with stderr if no result); the tab stays with a "Resume" button |

A background subagent keeps `claude -p` alive up to 10 minutes after stdin closes
(`CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS`); Hive does not wait for it when the tab closes (the
human closed it), but the Stop/close confirmation says background work will be lost (as #18).

---

## 6. UI design

- **Reuse 6.10**: `src/shell/TranscriptView.tsx` (TanStack Virtual list, header with state,
  "keep to the bottom") becomes the base of a `ConversationView` used by both the subagent view
  (read-only) and the chat (with a composer). `TranscriptEntry` gains a `kind`
  (`text | thinking | tool | subagent | permission | question | plan | divider | error | note | image`)
  and optional fields; the 6.10 roles stay compatible.
- **Where:** a chat is a tab in the terminal tab bar (7.5's "+" → **Agent**), with its own icon;
  the sidebar lists it like any agent (Project → Worktree → Agent). F8 and notifications work
  unchanged (states come from the service).
- **Composer** at the bottom: multi-line textarea (Enter sends, Shift+Enter newline), paste/drag
  images (thumbnails), `/` opens the slash command list from `system/init`, `@` could reuse the
  3.x "selection → prompt" reference format later. Send ↔ **Stop** while a turn runs.
  Permission mode selector (Default / Accept edits / Plan) in the composer bar.
- **Cards** (permission, question, plan) are entries in the list and also pin to the bottom above
  the composer while pending, so they cannot scroll away; focus moves to them; keyboard: Enter =
  primary, Esc = deny.
- **Styling:** tokens from `docs/ui-reference.md`; icons from Phosphor (7.7).
- **Mock transport** serves a scripted chat built from the recordings, so Playwright covers every
  entry kind and every card without claude.

---

## 7. Where the frontend stops (#37)

The app never parses stream-json. The service turns it into typed Hive messages; the app only
renders entries, keeps the draft and UI state, and sends user actions (`chat_send`, `chat_answer`,
`chat_interrupt`, `chat_set_mode`). Validation of answers and images is the service's.

Draft protocol (names for 7.3b, to be refined there and recorded in `docs/architecture.md`):

| Message | Direction | Fields |
|---|---|---|
| `open_chat` | app → service | `worktree`, `resume?` (session id), `mode?`, `model?` |
| `chat_opened` | service → app | `chat` (channel), `session?`, `model`, `mode`, `commands[]` |
| `chat_send` | app → service | `chat`, `text`, `images[] {media_type, data}` |
| `chat_entries` | service → app | `chat`, `entries[]`, `replace_from?` (for the growing last entry) |
| `chat_request` | service → app | `chat`, `request` (id), `kind: permission|question|plan`, fields per kind |
| `chat_answer` | app → service | `chat`, `request`, `allow|deny{message}|answers{…}`, `always?` |
| `chat_request_gone` | service → app | `chat`, `request` |
| `chat_interrupt` / `chat_set_mode` / `chat_set_model` | app → service | `chat`, … |
| `close_chat` / `chat_closed` | both | `chat`, `exit?`, `error?` |

---

## 8. Subscription and billing

1. **Does headless `-p` use the Pro/Max subscription?** Yes, today. Without `--bare`, `claude -p`
   uses the same login as the interactive CLI (headless docs: bare mode "doesn't use your
   subscription login", i.e. the normal mode does). The Help Center article (updated 2026-06-16)
   says: "For now, nothing has changed: Claude Agent SDK, `claude -p`, and third-party app usage
   still draw from your subscription's usage limits."
2. **The paused change.** The same article announced monthly "Agent SDK credits" ($20 Pro,
   $100/$200 Max) that would have replaced subscription limits for SDK and `claude -p` usage from
   2026-06-15. It was paused on that day; Anthropic says it is "working to update the plan" and
   will announce changes before they apply. **If it comes back, the chat's usage would move off the
   subscription limits while the terminals stay on them.** This is the reason SDK (antiga #1) was
   revoked. Interactive Claude Code was not part of the change.
3. **Is Hive allowed to do this?** The legal page restricts *developers* from offering claude.ai
   login in their own products or routing Pro/Max credentials on behalf of their users, and says it
   does not prevent "an end user from signing in to the unmodified Claude Code binary with their own
   Claude subscription". Hive runs the unmodified binary, on the human's machine, with the human's
   own login, never reads or stores credentials, and is not offered to others as a service. Usage
   limits "assume ordinary, individual usage of Claude Code and the Agent SDK". Reading: allowed for
   personal use; **not a legal opinion** — the human decides (Q2).
4. **What the service does:** reads `apiKeySource` from `system/init` and shows a warning in the
   chat header when the chat runs on an API key (`ANTHROPIC_API_KEY`, `apiKeyHelper`) instead of
   the subscription login (the value for a claude.ai login is *to be recorded*); never passes
   `--bare`; never shows `total_cost_usd` as a charge.

---

## 9. Security

- **The chat runs tools on the machine** with the human's user rights, exactly like `claude` in a
  terminal. What is new: **Hive is the permission prompt.** A Hive bug (answer to the wrong
  request, stale card, double click, auto-allow) runs a command without consent. Rules:
  - answers only for a pending `request_id` of that chat, once; unknown ids refused;
  - no auto-allow in Hive: the permission mode and Claude Code's own rules decide what is
    auto-approved; Hive only relays clicks;
  - default is deny: closing the tab, the app, a cancel or an error denies;
  - the card shows the full command/path; `updatedInput` is always claude's own input.
- **Permission modes:** `default` (ask) as the starting mode; `acceptEdits` and `plan` from the
  selector; `auto` (classifier) only if the human wants it (Q3); **`bypassPermissions` never
  offered** in the first version. "Always allow" writes `.claude/settings.local.json` in the
  project (Claude Code does it, not Hive) — only if approved (Q3).
- **No trust dialog in `-p`:** headless docs: "Without `--bare`, a `-p` session runs the hooks in a
  project's `.claude/settings.json` and connects the servers in its `.mcp.json`, even in a folder
  you've never trusted. A `-p` session shows no workspace trust dialog". The terminal asks; the
  chat would not. Proposal: chats only in projects the human added to Hive, and the first chat in
  a project whose folder Claude has not trusted yet shows Hive's own confirmation (Q4).
- **Untrusted output:** model and tool text may carry escape sequences, HTML, huge lines or
  prompt-injected instructions. Rendered as text, bounded (5.5); AskUserQuestion previews off.
- **Process:** argument vector only; the session id passed to `--resume` must be a UUID; the
  cwd is a worktree the service knows; the `security-reviewer` agent reviews 7.3b and 7.3c.

---

## 10. Risks specific to this design

1. **Undocumented protocol pieces**: `--permission-prompt-tool stdio`, the `control_request`
   subtypes (`initialize`, `can_use_tool`, `interrupt`, `set_permission_mode`, `set_model`) and
   `CLAUDE_CODE_SDK_READS_SESSION_STATE` come from the open-source Python SDK, not from the CLI
   reference. Mitigation: feature-detect with `system/init.capabilities`; tests built on the
   recorded fixtures; a service check that refuses a Claude Code version it cannot drive, with a
   clear error; re-record on each Claude Code update (as risk 2 of `hive.md` for the wrapper).
2. **Billing split returning** (section 8).
3. **Scope**: this is the largest feature since Stage 3 (a conversation UI with cards, images,
   streaming). Section 14 splits it.

---

## 11. Recording real sessions (for the human)

`scripts/spike/record-chat.py` (Python 3 standard library only; no shell config needed).

```sh
cd ~/dev/projects/hive
python3 scripts/spike/record-chat.py --list     # the scenarios
python3 scripts/spike/record-chat.py            # all of them (~15–25 min with haiku)
```

- It creates `/var/tmp/hive-chat-spike.XXXX/` with a scratch git repo (`repo/`) and the
  recordings (`rec/`): per scenario `<name>.out.jsonl` (claude's stdout, exact),
  `<name>.in.jsonl` (what the script sent), `<name>.timing.tsv`, `<name>.stderr.txt`,
  `<name>.cmd.json`; plus `version.txt`.
- Scenarios: `text`, `thinking`, `tools`, `permission` (first request allowed, the second
  denied), `question` (answers the first option), `plan` (approves), `subagent`, `compaction`
  (`/compact` between two turns), `error-model` (unknown model), `error-max-turns`, `image`
  (a generated 32×32 PNG), `interrupt` (interrupts `sleep 40` 5 s after it starts), `resume`
  (resumes the `text` session). Run a subset by naming them.
- It runs the plain `claude` found on `PATH` (skipping Hive's wrapper dir; `--claude <path>` to
  choose), with `--model haiku` (`--model` to change), `--strict-mcp-config` and
  `--setting-sources project,local`: your user settings, hooks and MCP servers are not loaded.
  It answers every prompt automatically; nothing to type.
- Outside `/var/tmp`, claude saves the sessions under `~/.claude/projects/<scratch path>` as any
  session does; the script prints that folder at the end so you can delete it.
- Recordings contain only the scratch repo's content and the prompts above. Share them with
  `tar -C /var/tmp/hive-chat-spike.XXXX -czf /var/tmp/hive-chat-spike.tar.gz rec` and tell the agent
  the path; the agent then replaces every *to be recorded* example with real lines and closes the
  open points (thinking shape, `stdio` + `permission_denied`, plan mode after approval,
  `apiKeySource` value, `session_state_changed`, image limits, `PermissionRequest` hook in `-p`).

---

## 12. Alternative considered: option B, rich prompts for the interactive `claude`

Keep only terminals, and answer the interactive `claude`'s permissions and questions from the app
through hooks: a `PermissionRequest` hook (and `PreToolUse` for `AskUserQuestion`/`ExitPlanMode`
with `updatedInput`) that blocks, asks the service, and returns the human's answer. It keeps
interactive billing and needs no chat UI, but the hook must block (conflicts with #27's ~200 ms
hook timeout), the terminal shows its own prompt at the same time, and there is no composer. This
is `hive.md`'s Fase 2 "Interações ricas". It stays the fallback if the billing split returns.

---

## 13. Proposed `docs/hive.md` changes (Portuguese, for the human to apply)

**🧭 Conceito central — novo 1º parágrafo (substitui o atual):**

> O Hive é **igual ao Orca neste ponto**: tem **terminais embutidos**, e eu rodo o `claude`
> interativo dentro deles. O Hive **observa** os agentes abertos nos seus terminais e mostra o
> estado de cada um; **nos terminais ele não controla e não conversa** com os agentes. Além dos
> terminais, o Hive tem **abas de chat** (7.3): nelas eu converso com o `claude` pelo próprio app,
> e o app é a interface do agente, inclusive das permissões, perguntas e aprovação de plano. O
> chat roda o binário oficial em modo headless (`claude -p` com `stream-json`), sem o pacote do
> Agent SDK e sem Node. Por conveniência, o Hive **pode iniciar** um `claude` digitando o comando
> num terminal novo (…resto igual).

**Consequências — substituir as linhas:**

| Hoje | Proposta |
|---|---|
| **Sem Claude Agent SDK.** O uso é o `claude` normal, dentro dos limites da assinatura. | **Sem o pacote do Claude Agent SDK e sem Node.** Terminais usam o `claude` interativo; o chat usa o `claude -p` com o protocolo `stream-json`, com o meu login. Hoje os dois consomem os limites da assinatura; o `-p` segue a regra de cobrança do SDK (ver #45). |
| O Hive **pode editar arquivos** (Etapa 3b), mas nunca age sobre o agente. | O Hive **pode editar arquivos** (Etapa 3b). Nos terminais nunca age sobre o agente; no chat, só repassa o que eu digito e clico. |

**💡 Decisões técnicas — novas linhas:**

| # | Decisão | Motivo |
|---|---|---|
| 40 | **Chat no app (7.3)**: aba de chat ao lado dos terminais; o serviço roda o `claude` real em modo headless (`-p --input-format stream-json --output-format stream-json --verbose --replay-user-messages --permission-prompt-tool stdio --settings <hooks do Hive>`), por **pipes, sem PTY**, num grupo de processos próprio; o chat usa o mesmo espaço de canais dos terminais (`HIVE_TERMINAL_ID` = canal do chat) e morre com o app como os terminais (#18); nunca `--bare` | Pedido meu de 2026-09-25; o `stream-json` é o que o SDK usa por baixo, sem precisar do pacote nem de Node; os hooks, a barra lateral, o contador e as notificações funcionam iguais |
| 41 | **O app é o host de permissões do chat**: pedidos de permissão, perguntas (`AskUserQuestion`) e aprovação de plano (`ExitPlanMode`) viram cartões na conversa; nada roda sem clique; fechar, cancelar ou erro = negar; o serviço só aceita resposta para um pedido pendente daquele chat, uma vez. Modo inicial `default`; `acceptEdits` e `plan` no seletor; `bypassPermissions` nunca | Com o chat, o Hive passa a ser o prompt de permissão; um erro aqui executaria comandos sem meu consentimento |
| 42 | **Estados do chat vêm do stream**, além dos hooks: pedido pendente → 🟡, `result` de sucesso → 🟠, `result` de erro ou saída sem `result` → 🔴, fim do processo → ⚫; a regra do silêncio do PTY não vale para o chat | O stream diz exatamente quando o agente espera por mim; pergunta e fim de turno deixam de parecer iguais |
| 43 | **Saída do chat é entrada não confiável**: linha ≤ 16 MiB, texto por entrada cortado em 64 KiB, plano em 256 KiB; imagens enviadas por mim ≤ 5 MiB, no máximo 10 por mensagem, tipo conferido pelo conteúdo; nada é renderizado como HTML; prompts e saídas de ferramentas fora dos logs no nível padrão | Mesmas regras de hooks e mensagens do socket (linha de base do código) |
| 44 | **Retomada do chat**: o `session_id` do `system/init` fica no chat; reabrir usa `--resume <id>`; o histórico vem do arquivo da sessão (leitor da 6.10); o `open-sessions.json` (4.12) guarda o tipo (terminal ou chat) e o chat volta como chat | Mesmo comportamento dos terminais depois de reiniciar o app |
| 45 | **Cobrança do chat**: usa o meu login do `claude` (o Hive nunca lê credenciais); o cabeçalho avisa quando o `apiKeySource` indica chave de API em vez da assinatura; o `total_cost_usd` (estimativa) não é mostrado como valor cobrado | Hoje o `claude -p` consome os limites da assinatura (nota da Anthropic de 16/06/2026); a separação anunciada foi pausada e pode voltar |
| 46 | **Pastas não confiáveis**: chat só em projetos adicionados ao Hive; no primeiro chat numa pasta que o Claude ainda não marcou como confiável, o Hive pede confirmação | O `-p` não mostra o diálogo de confiança e já roda hooks e servidores MCP do projeto |

**🗑️ Decisões revogadas — nova linha:**

| Antiga | O que era | Por que saiu |
|---|---|---|
| Conceito central | "O Hive não controla e não conversa com os agentes" valendo para todo o app | O chat (#40) conversa com o `claude`; os terminais continuam só observados. A linha "SDK (antiga #1)" continua revogada: o chat não usa o pacote do SDK nem Node, mas o risco de cobrança não foi eliminado (#45, risco novo) |

**⏸️ Fase 2, item 2 — nova redação:**

> **Interações ricas nos terminais**: responder permissões do `claude` interativo pelo app (hook
> `PermissionRequest`), perguntas de múltipla escolha como botões, mockups em markdown. *No chat
> (7.3) isso já existe.*

**⚠️ Riscos — novos itens (a lista hoje vai até 9):**

10. **Protocolo de controle do `stream-json` não documentado na referência da CLI** (`--permission-prompt-tool stdio`, `control_request` `initialize`/`can_use_tool`/`interrupt`/`set_permission_mode`): vem do SDK de código aberto e pode mudar numa versão do Claude Code. Mitigação: detectar recursos pelo `capabilities` do `system/init`; testes com gravações reais; o serviço recusa com erro claro uma versão que não sabe conduzir; regravar a cada atualização.
11. **A separação da cobrança do `claude -p`/SDK** (anunciada e pausada em 15/06/2026) **pode voltar**: o chat passaria a consumir outro crédito, diferente dos terminais. Mitigação: chat opcional; terminais intactos; plano B: interações ricas no `claude` interativo via hooks (Fase 2).

**`CLAUDE.md`** (do humano): a frase "Hive only **observes** agents; it never starts, controls or talks to them" também muda se o chat for aprovado.

---

## 14. Draft implementation tasks (not in TODO.md until the human approves)

- [ ] **7.3b Chat process in the service.** `task/7.3b-chat-service` — `hive::chat`: spawn the real `claude` headless (section 3.1) with pipes in a new process group, `HIVE_TERMINAL_ID` = chat channel, Hive's hook settings; bounded line reader (16 MiB), stream-json → typed entries (text, thinking, tool + result, error, divider, note), unknown types dropped; `initialize` on start; close/stop escalation (5.6); dies with the app (#18). Protocol: `open_chat`, `chat_opened`, `chat_send` (text only), `chat_entries`, `close_chat`, `chat_closed`; states from the stream (5.4). Tests drive a fake `claude` (copied `/usr/bin/dash` script or a small Rust test binary) replaying the recorded fixtures. Run the `security-reviewer`.
- [ ] **7.3c Permissions, questions and plans.** `task/7.3c-chat-requests` — `--permission-prompt-tool stdio`; pending request table; `chat_request` / `chat_answer` / `chat_request_gone`; answer validation (5.5); deny on close; 🟡 while pending; `set_permission_mode`. Run the `security-reviewer`.
- [ ] **7.3d Conversation view and composer.** `task/7.3d-chat-view` — generalize `TranscriptView` into `ConversationView` (entry kinds, 6.10 stays read-only), chat tab in the tab bar, composer (Enter/Shift+Enter, Send ↔ Stop), mock transport scripted chat, e2e.
- [ ] **7.3e Cards.** `task/7.3e-chat-cards` — permission, question (single/multi/Other) and plan cards, pinned above the composer while pending, keyboard (Enter/Esc), focus handling; e2e for each.
- [ ] **7.3f Images.** `task/7.3f-chat-images` — paste/drag in the composer, service validation by content and size, thumbnails in bubbles and tool results, CSP check.
- [ ] **7.3g Resume and restore.** `task/7.3g-chat-resume` — `--resume`, history from the transcript file, `open-sessions.json` `kind`, "Open as chat" in the Sessions panel, "Resume" on an ended chat.
- [ ] **7.3h Live text.** `task/7.3h-chat-streaming` — `--include-partial-messages`, coalesced deltas (≤ one update per 50 ms per chat), growing last entry.
- [ ] **7.3i Stop, modes, models, commands.** `task/7.3i-chat-controls` — `interrupt` with `cancel_queued`, mode selector, `set_model`, `/` command list from `init` and `commands_changed`, subscription/API-key warning from `apiKeySource`.
- 7.5's "+" → **Agent** opens a chat once 7.3d lands.

---

## 15. Questions for the human

1. **Concept:** approve the chat (Hive talks to Claude in chat tabs) while terminals stay
   observe-only? (Changes Conceito central, #1, the SDK revocation note, Fase 2 item 2.)
2. **Billing:** accept that chat usage follows the `claude -p` billing (subscription today, a
   separate credit if the paused split returns)? What should Hive do if it returns: hide the chat,
   warn, or nothing? Show no dollar value on a subscription (recommended)?
3. **Permissions:** starting mode `default` (recommended); offer `acceptEdits` and `plan`;
   offer `auto`? Offer "Always allow" (writes `.claude/settings.local.json`)? Never
   `bypassPermissions` (recommended).
4. **Untrusted folders:** chats only in added projects plus a Hive confirmation for a folder
   Claude has not trusted yet (recommended), or load no project settings in chats
   (`--setting-sources user`, different behaviour from the terminal)?
5. **Markdown:** render assistant text as Markdown (a new dependency, e.g. a small sanitizing
   renderer, to be named in 7.3d) or plain text with code blocks in monospace?
6. **Live text (7.3h):** needed in the first version, or is one entry per finished block enough?
7. **Thinking:** show collapsed (recommended) or hide like 6.10?
8. **Recording:** please run `python3 scripts/spike/record-chat.py` and share the `rec/` folder.
