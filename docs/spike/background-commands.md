# Spike 28.8: The agent's background commands

Status: **proposal** (⏸ checkpoint). No product code changed. Sources: the public Claude Code docs read on 2026-11-27 (`code.claude.com/docs/en/hooks`, `tools-reference`, `interactive-mode`, `agent-sdk/typescript`), what Hive already parses (`crates/hive/src/states.rs`, `slim.rs`, `transcript.rs`, `osc.rs`), and public bug reports about Claude Code's background-task notices. There are no recordings yet: 26.2's spike report (`docs/spike/stage-0.md`) has not been written, and the real `claude` was not run. Every point marked **(to record)** is something the human's recording (§8) must confirm.

## 1. Recommendation in one paragraph

It is feasible, with one weak spot: the exit code. The start (the command, its description, its task id, which agent or subagent ran it) comes from the `PostToolUse` of `Bash` with `run_in_background`. Claude Code lists the session's running tasks, with their `command`, on every `Stop` and `SubagentStop` (`background_tasks`, now documented). No hook fires when a background command ends. Two signals tell Hive it ended: Claude Code's task notice in the transcript, which carries a status (`completed`, `failed`, `stopped`) and a summary that names the exit code, and the task leaving the next `background_tasks`. Build it on both: the notice gives the result within about a second (Hive already reads the transcript every second), and the next `Stop` reconciles anything the notice missed. The quiet notification is an inbox item with no tone and no OS notification. A command that failed plays the tone, where failed means status `failed`, or exit code not 0 in the summary. The summary's exit code is known to be wrong sometimes (it reports 0 for a command that exited 1, or for one killed at its timeout), so a failure Claude Code misreports shows as a success. That limit is Claude Code's, and the row says what Claude Code said.

## 2. Today

- **Long commands in Hive's own terminals (17.25, 18.40):** `hive::osc` times OSC 133 `C`/`D;<exit>` in a terminal's output and sends `command_finished {exit, secs}`, but only in a terminal with **no** agent. A command Claude runs never goes through the terminal's shell. It runs in Claude Code's own shell, so OSC 133 never sees it. Nothing from 17.25 can be reused for detection. Its presentation can be reused (inbox item, red on failure, `notifications.command_secs`).
- **The agent's background tasks (5.10, 7.6, 27.7):** `states.rs` reads:
  - `tool_response.backgroundTaskId` (Bash), `taskId` (`Monitor`) and `agentId` (`Agent`) of a subagent's `PostToolUse`, into `launched` (subagents only, to keep one listed while its own launch runs);
  - `background_tasks` on `Stop`/`SubagentStop`, for its length only (the activity "N background tasks") and for `prune` (the ids, matched to `launched`).
  The agent's **own** launches are not kept, and neither the command nor any entry field other than `id` is read.
- **`hive hook` slimming (27.5):** a `PostToolUse` keeps `tool_name`, `background_tasks` and the three `tool_response` ids. **`tool_input` is dropped**, so the command and `run_in_background` never reach the service today.
- **Transcript (`transcript::Usage`):** a bounded reader of new lines, run every second for an agent that is working, has subagents or is on a dialog. It recognises interrupts (`sessions::Ending`). It does not look at task notices.
- **Hooks registered:** the 16 events in `wrapper.rs`, with no matcher, so a `Monitor`'s `PostToolUse` reaches Hive too. The docs say only a `Bash` matcher skips Monitor.

## 3. What Claude Code gives

### 3.1 Start

| Source | What it carries | Status |
|---|---|---|
| `PreToolUse` / `PostToolUse` of `Bash` | `tool_input`: `command`, `description`, `run_in_background: true`, `timeout` | documented (`run_in_background` in the Bash `tool_input` table) |
| same `PostToolUse` | `tool_response.backgroundTaskId` | **not in the public docs**. Hive's code relies on it (5.10, found in the binary, hive.md risk 12). Its format and whether it equals `background_tasks[].id`: **(to record)** |
| same | `agent_id` when a subagent ran it | documented (common subagent fields) |
| A foreground command moved to the background (its timeout, or `Ctrl+B`) | the result text `Command did not complete within its 120s timeout and was moved to the background`, then the task id and the output file's path. A background subagent's command moved after 3 min: `Command was moved to the background (ID: <id>) after running for 180s` | documented as text. Whether `backgroundTaskId` is set in those cases: **(to record)** |
| `Monitor` `PostToolUse` | `tool_response.taskId` (already read by Hive); its command | `taskId` from Hive's code; the input field names **(to record)** |

### 3.2 While it runs: `background_tasks`

Documented in the hooks reference (Stop input): `Stop` and `SubagentStop` carry `background_tasks` and `session_crons`, "present when the task registry is reachable and empty when nothing is in flight", scoped to the **parent session** (a subagent's `SubagentStop` lists the whole session's). Each entry:

| Field | Meaning |
|---|---|
| `id` | task id |
| `type` | `shell`, `subagent`, `monitor`, `workflow`, `teammate`, `cloud session`, `MCP task`, or the raw kind |
| `status` | "current task status" (values not listed; `running` in the example) |
| `description` | ≤ 1000 characters, clipped with `… [+N chars]` |
| `command` | ≤ 1000 characters, `shell` and command `monitor` tasks only |
| `agent_type`, `server`, `tool`, `name` | for subagents, MCP / monitor, workflows |

Example from the docs: `{"id": "task-001", "type": "shell", "status": "running", "description": "tail logs", "command": "tail -f /var/log/syslog"}`.

Consequence: `background_tasks` is a full, authoritative list at every turn end. It covers shells Hive never saw start (started before Hive detected the agent, after a service restart, or moved to the background with no `backgroundTaskId`). The entries do not say which agent or subagent started a shell. That comes only from the `PostToolUse`'s `agent_id`.

### 3.3 End

- **No hook event** fires when a background command ends. `TaskCreated`/`TaskCompleted` are the to-do list (`TaskCreate`/`TaskUpdate`), not background shells. `Notification` `agent_completed` is a background *session* in agent view. The documented extra sources of `UserPromptSubmit` (a scheduled task firing, a background subagent reporting back, a message from another session) do not include a background shell ending.
- **The task notice.** Claude Code tells the model when a task ends. In the Agent SDK it is the documented `SDKTaskNotificationMessage`: `task_id`, `tool_use_id?`, `status: "completed" | "failed" | "stopped"`, `output_file`, `summary`, `usage?`. It covers `run_in_background` Bash, Monitor watches and background subagents. In an interactive session the same notice reaches the model as a `<task-notification>` block in the conversation (public bug reports quote `<task-notification>` with a summary like `Background command "<description>" completed (exit code 0)`). It is therefore written to the transcript. Its exact record shape (a `user` message whose content holds the XML, or an attachment / queued-command record), its tags and the summary's wording for each status: **(to record)**.
- **Exit code:** no structured field anywhere. It is only in the summary text. Public reports show it can be wrong: `completed (exit code 0)` for a command that exited 1, and for one killed at its timeout. A time-limit stop also says so in text: `Background command "<description>" was stopped after reaching its background time limit` (only unattended sessions have that limit, so a terminal session never sees it).
- **Stopped:** by the user from `/tasks`, by Claude's `TaskStop` tool (`PreToolUse`/`PostToolUse` of `TaskStop` with the task id; the input field name is **(to record)**), at exit (Claude Code stops them all; `SessionEnd`), at its 5 GB output cap, on memory pressure after 30 idle minutes, and, for a **foreground** subagent's command, when that subagent's run ends.
- **Afterwards:** when Claude was idle, the notice wakes it. A turn follows (`PreToolUse`… `Stop`), and the `Stop`'s `background_tasks` no longer has the task. Which hook opens that turn (7.6's R5, still open) is **(to record)**. Mid-turn, the notice is read at the next model call and the list updates at the turn's end.

### 3.4 The output

The notice names `output_file`, Claude Code's own file of the task's output. Hive does not read it. It sits outside the folders Hive reads (`transcript::open_inside`), it can be up to 5 GB, and showing its text is not asked for. Not proposed.

## 4. Feasibility against the task's wording

| Wanted | Possible? | From |
|---|---|---|
| Each agent's row lists its **running** background commands | yes | the `PostToolUse` start (command, description, who), kept until it ends; corrected by each `background_tasks` |
| The command text | yes | `tool_input.command` (start) or `background_tasks[].command` (≤ 1000 characters) |
| Which subagent ran it | yes for one Hive saw start; unknown for one only seen in `background_tasks` | `agent_id` of the `PostToolUse` |
| Says when one **ends** | yes, within ~1 s from the transcript notice; else at the next `Stop`/`SubagentStop` | §3.3 |
| **Failure** (tone only then) | partly: status `failed`, or the exit code parsed from the summary; Claude Code's own misreports pass as successes | §3.3 |
| How long it ran | yes (Hive's clock, start to end) | |

## 5. Proposed design

### 5.1 Service (`hive::states`, with a new small `hive::background` module)

1. **Slimming (`slim.rs`):** for a `Bash` `PostToolUse` with `tool_input.run_in_background == true`, also keep `tool_input.command` (clipped to 1000 characters, as Claude Code's own list) and `tool_input.description` (200). The same for `Monitor` (field names after the recording). Nothing else of `tool_input`. The rest of 27.5's limits stay.
2. **Per agent, `background: Vec<BackgroundCommand>`**, at most 32 (like subagents), oldest first: `{id, kind: shell | monitor, command, description, by: Option<subagent id>, started_ms, ended: Option<{status, exit, ms}>}`. Ids over 256 bytes are ignored, as today. The agent's own launches are kept too (today only subagents' are), and the existing `launched` map is folded into it so there is one record.
3. **Start:** the `PostToolUse` with a `backgroundTaskId` (or `taskId` for Monitor) adds it.
4. **Reconcile on every `background_tasks`** (`Stop`, `SubagentStop`), as `prune` already does for subagents:
   - an entry of `type` `shell`/`monitor` that Hive does not have is added from its `command`/`description`, with `by` unknown;
   - a running one missing from the list has ended. When no notice came for it, it ends with status `unknown`, quiet.
5. **Notice from the transcript:** `transcript::Usage::read` also recognises a task notice record (shape after the recording). It yields `(task_id, status, exit)`, with the exit code parsed from the summary only with a strict pattern (`exit code <-?digits>`), else `None`. Matching the task id of a listed command ends it with that status. The every-second read is extended to agents with a running background command (today it runs only while working, with subagents or on a dialog). The text is untrusted, so the parse is bounded by the reader's existing line cap and nothing of it is echoed but the parsed fields.
6. **Ended commands** stay on the row for 10 s (struck/greyed, with their result) and then leave, so a glance after the notification still finds them. `SessionEnd` drops all of them with no notification (Claude Code stops them at exit). A foreground subagent's commands that are still listed when it leaves are kept until the next `background_tasks` (the docs say they stop, but a notice will say so).
7. **Message:** `agent_state` gains `background: [{id, kind, command, description, by, since_ms, ended?}]`, omitted when empty (like `writing`). A new `background_finished {agent, id, command, description, status, exit, secs}` is sent once per command that ends, with `alert: quiet | failed`. The service decides `failed` (#37): status `failed`, or `exit` not 0 and not `None`. `stopped` is never a failure. The activity "N background tasks" stays as it is.

### 5.2 App

- **Sidebar row:** under the agent, after its subagents, one line per running command: a terminal icon, the description (else the command), clipped to one line, its age ("2m"), and the full command in the title. A subagent's commands are marked with its type. Collapsed with the agent. No actions: Hive observes (#52), so it shows no Stop button. The user stops a task in Claude Code's `/tasks`.
- **On `background_finished`:** an inbox item, "Background command finished: <description>", with a neutral check, or "… failed (exit 3)" in red. Quiet means **no tone and no OS notification**. A failure plays the tone and posts an OS notification when the agent is out of view (as 17.25's red items do). A click shows the agent's terminal.
- **The agent's own alert:** a command ending usually wakes Claude, whose turn then ends with the usual `finished` alert and tone. The background item must not repeat that alert. Here it is only the inbox line, so the user's one alert is still the agent's own (Q3).

### 5.3 Limits and security

- `command` and `description` are untrusted text, cleaned and clipped as `adapter::clip` does (control characters out). The command line may hold a secret (`curl -H "Authorization: …"`, `export TOKEN=…`). It is shown only in this app, never logged at the default level, and it does cross to a remote app as `agent_state` already does for activities (Q5).
- 32 commands per agent, 1000/200 characters, 10 s of ended ones: one `agent_state` stays far under the frame limit (32 × ~1.3 KiB).

## 6. Questions for the human (recommendations first)

- **Q1. Build on the notice, the list, or both?** *Recommended: both.* The transcript notice is prompt and has the status and exit code, but its shape is undocumented. `background_tasks` is documented, but it comes only at turn ends and has no result. With only the list: the end is known late (at the next `Stop`) and nothing is ever a failure, so no tone ever plays.
- **Q2. What counts as failed?** *Recommended:* status `failed`, or an exit code other than 0 in the summary. `stopped` (by `/tasks`, `TaskStop`, the time limit) and `unknown` are quiet. Alternative: only `failed`, if the recording shows a non-zero exit always comes as `failed`.
- **Q3. "Quiet notification" = an inbox item only, no tone and no OS notification?** *Recommended: yes*, since Claude's wake-up turn already alerts `finished` with its tone. A failure gets the tone and an OS notification (out of view). Alternative: an OS notification without sound for every end, which risks two OS notifications for one event.
- **Q4. Only commands (`shell`, `monitor`), or every background task?** *Recommended: shells and monitors only*: subagents and teammates already have their own lines (5.10, 26.10), and workflows / MCP tasks are not commands. Monitors never end by themselves (`tail -f`), so they would sit on the row for the whole session. Alternative: shells only.
- **Q5. Show the full command text?** *Recommended: yes in the row's title and to remote apps of the same user*, never in logs. It is what Delta shows and what Claude Code's `/tasks` shows. Alternative: description only.
- **Q6. How long does an ended command stay on the row?** *Recommended: 10 s*, then only the inbox item remains. Alternative: until the next `UserPromptSubmit`.
- **Q7. Wait for the recording before building?** *Recommended: yes for the notice parse (step 5.1.5).* Steps 5.1.1–4 and the UI rest on documented fields plus `backgroundTaskId`, which Hive already relies on, so they can start at once with fixtures marked SYNTHETIC (as 27.5 and 27.7 did). The notice parse lands when the recording gives its shape. Until then an end is known at the next `Stop`, quiet.
- **Q8 (hive.md, risk 12):** `background_tasks` is now in the public hooks reference with its fields. A proposed row change (Portuguese, not applied): *"12. Fontes não documentadas: o registro `<config>/sessions/<pid>.json` (8.16) e o `backgroundTaskId` do `tool_response` (5.10, 28.8) foram achados no binário; o `background_tasks` passou a constar da referência de hooks (2026-11)."*

## 7. Build plan and tests (after the checkpoint)

1. **28.8a Service:** slimming of `tool_input` (`slim.rs`), `hive::background` (records, reconcile, end, alert rule), the agent's own launches, `agent_state.background`, `background_finished` in `hive-protocol`, the catalog in `docs/architecture.md`.
   - Rust unit tests: start from a `PostToolUse` (agent's own and a subagent's); an unknown shell added from `background_tasks` (and a `subagent`/`workflow` entry ignored); a running one missing from the list ends `unknown` and quiet; a notice ends it `completed`/`failed`/`stopped` with its exit; exit parse (0, 3, -1, missing, malformed, huge, repeated); a non-zero exit is `failed`, `stopped` never is; 32-command and length caps; ids over 256 bytes; `SessionEnd` drops them with no message; an old `sent_ns` still counts for the launch (9.17); the ended ones leave after 10 s (injected clock).
   - Slimming tests: `command`/`description` kept, clipped, only with `run_in_background`; nothing else of `tool_input`.
   - Integration (`crates/hive/tests/integration/`, `common::Env`, the fake `claude`): a synthetic hook sequence (`PostToolUse` with `backgroundTaskId`, a transcript line with the notice, `Stop` with an empty list), and the app connection receives `agent_state` with the command, then `background_finished` failed with exit 3; and the same without the notice line ends `unknown` at the `Stop`. Fixtures `crates/hive/tests/fixtures/background_synthetic.json`, marked SYNTHETIC until replaced by the recording.
2. **28.8b App:** the row's lines, the inbox item, the tone only on `failed`, the mock transport's stand-in (`?mock=…` emitting a start and an end).
   - bun: the lines under the agent (own and subagent's, clipped, title), removal on end, the inbox text for each status, tone and OS notification only on failure and out of view.
   - e2e (Chromium, WebKit) on the mock: a command appears under its agent, ends, the inbox item shows, red for a failure.
3. **28.8c (after the recording):** the notice parse in `transcript.rs`, tested on the recorded lines (stripped of the human's text) plus malformed variants.

Expected size: 28.8a + 28.8b about 8 points as estimated; 28.8c small.

## 8. What the human's recording should confirm

One short session through `scripts/spike/dogfood.sh` (or 26.2's provoked session), in a scratch repository, hooks recorded with `hive hook --record`. Keep the session's transcript (`.jsonl`) for the lines in question.

| # | Prompt / action | Confirms |
|---|---|---|
| B1 | `Run "sleep 20; echo ok" in the background (run_in_background) and end your turn.` Wait for it to wake. | `PostToolUse` `tool_response.backgroundTaskId` and its format; `tool_input.command`/`description`; equal to `background_tasks[].id`; `status` values seen; the notice's transcript record (shape, tags, summary for exit 0); which hook opens the wake-up turn (7.6 R5) |
| B2 | `Run "sleep 5; exit 3" in the background and end your turn.` | the notice for a non-zero exit: status `failed` or `completed`, the summary's exit code |
| B3 | `Run "sleep 300" in the background and end your turn.` Then stop it from `/tasks`. | status `stopped`, its summary; whether Claude wakes |
| B4 | `Run "sleep 300" in the background, then stop it with TaskStop.` | `TaskStop`'s `tool_input` field name; the notice |
| B5 | `Run "sleep 200"` (foreground), press `Ctrl+B` while it runs. | whether the `PostToolUse` has `backgroundTaskId` when moved by hand |
| B6 | `Use a subagent to run "sleep 10" in the background and report.` (foreground subagent) | `agent_id` on the launch; what the notice says when the subagent's end stops it |
| B7 | `Watch "tail -f notes.txt" with the Monitor tool`, then end it. | Monitor's `tool_input`/`tool_response` fields, `background_tasks` entry with `type: monitor` and `command` |
| B8 | Start B1 again and, while it runs, ask a long question so the notice arrives mid-turn. | when the notice is written relative to the turn's events |

## 9. Sources

- Hooks reference, Stop input (`background_tasks`, `session_crons`), SubagentStop, TaskCreated/TaskCompleted, Notification types, UserPromptSubmit sources: https://code.claude.com/docs/en/hooks
- Tools reference, Background commands, time limit, moving to the background, `TaskOutput` removed (v2.1.277), `TaskStop`, Monitor: https://code.claude.com/docs/en/tools-reference
- Interactive mode, How backgrounding works: https://code.claude.com/docs/en/interactive-mode
- Agent SDK, `SDKTaskNotificationMessage`: https://code.claude.com/docs/en/agent-sdk/typescript
- Bug reports about the notice's exit code (not official docs): "Background Bash task notification reports exit code 0 when the command exited 1" (claudeissues.com/issue/81270), "Backgrounded Bash command killed by its timeout reports completed (exit code 0)" (claudeissues.com/issue/90616), "Background task notifications should include exit code and output tail" (claudeissues.com/issue/63649)
