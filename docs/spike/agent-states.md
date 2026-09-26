# Agent states: re-evaluation (TODO 7.6, proposal)

Status: **applied** (task `task/7.6-agent-states`, 2026-09-26): the human accepted every recommendation (Q1–Q5) on 2026-09-25 and asked to implement before the recordings; see [§8](#8-applied-for-docshivemd). The recordings (§6) are still wanted to confirm the rows marked (to record). The original proposal follows unchanged.

Original status: proposal. No product code changed. Built from the code on `main` (v0.2.0), the official hooks reference (`code.claude.com/docs/en/hooks`, read 2026-09-25) and a read-only look at existing transcripts. Anything marked **(to record)** is not confirmed yet and is what the recording session below must answer.

## 1. The bug, and why

The human saw a multiple-choice question (AskUserQuestion) and a finished turn both show 🟠 "waiting for you". The cause is wider than AskUserQuestion:

1. **Rule 2 (PTY silence) also decays 🟡.** `Agent::reconcile` (`crates/hive/src/states.rs`) turns both `Working` **and** `WaitingPermission` into `WaitingYou` after `agents.silence_secs` (5 s) with no output and no hook event. Every Claude Code dialog (permission, question, plan approval, MCP form) is static: the spinner stops, so the terminal goes quiet. A plain permission prompt therefore goes 🟡 → 🟠 after 5 s; Claude Code's own `Notification permission_prompt` arrives ~6 s after the prompt (only if the user has not typed) and puts it back to 🟡; 5 s later it is 🟠 again, for good.
2. **Questions and plan approval are not recognised.** Hive maps `PreToolUse` of any tool to 🔵. For `AskUserQuestion` and `ExitPlanMode` the `PreToolUse` *is* the moment the dialog opens (the tool never auto-runs: "Actions no mode auto-approves" lists AskUserQuestion; the docs call plan approval a permission prompt). Whether a `PermissionRequest` also fires for them is **(to record)**; with or without it, rule 2 ends in 🟠.
3. **Auto-compaction resets the agent.** `SessionStart` with `source: "compact"` fires after every compaction, with the same `session_id`. `Daemon::saw` → `detect` builds a fresh `Agent`: state 🟢 idle in the middle of a turn, subagents and token usage forgotten.
4. Smaller: `Notification` types `elicitation_url_dialog`, `elicitation_complete`, `elicitation_response`, `quota_auto_resume_*` fall into `Other` (no change); `agent_needs_input` means a question on screen, not a finished turn.

## 2. What the hooks give (only what matters for states)

| Event | When | Fields used / useful | Registered by Hive today |
|---|---|---|---|
| `SessionStart` | new, resume, `/clear`, after compaction, fork | `source` (`startup`, `resume`, `clear`, `compact`, `fork`), `transcript_path` | yes |
| `UserPromptSubmit` | prompt sent | — | yes |
| `PreToolUse` | before every tool call (not `EndConversation`) | `tool_name`, `tool_input`, `tool_use_id`, `agent_id` | yes |
| `PermissionRequest` | immediately when a permission dialog is about to show | `tool_name`, `tool_input` (no `tool_use_id`) | yes |
| `PostToolUse` / `PostToolUseFailure` | after a tool call; failure has `error`, `is_interrupt`. "Cancelling a running tool does not fire this hook" | `tool_response` (`backgroundTaskId`, `taskId`, `agentId`) | yes |
| `PostToolBatch` | once after a batch of parallel calls | — | no (not needed) |
| `PermissionDenied` | auto mode denied a call (Claude continues) | `reason` | no (not needed: more tool events follow) |
| `Notification` | `permission_prompt` (~6 s after a permission dialog, only if the user is away), `idle_prompt` (~60 s after a finished turn, no typing), `elicitation_dialog` / `elicitation_url_dialog` (~6 s), `elicitation_complete`, `elicitation_response`, `agent_needs_input`, `agent_completed`, `auth_success`, `quota_auto_resume_fired` / `_stale` / `_disabled` | `notification_type`, `message`, `title` | yes |
| `Elicitation` / `ElicitationResult` | an MCP server opens a form / the user answered | `mcp_server_name`, `mode` | no |
| `Stop` | turn finished. **Not on user interrupt** | `background_tasks[]` (`id`, `type`, `status`, …), `session_crons[]`, `last_assistant_message` | yes |
| `StopFailure` | turn ended by an API error (instead of `Stop`) | `error` (`rate_limit`, `overloaded`, `authentication_failed`, `billing_error`, `server_error`, `max_output_tokens`, …, `unknown`), `error_details` | yes |
| `SubagentStart` / `SubagentStop` | subagent spawned or resumed / finished its turn | `agent_id`, `agent_type`, `background_tasks` | yes |
| `PreCompact` / `PostCompact` | before / after compaction | `trigger` (`manual`, `auto`) | no |
| `SessionEnd` | session over (`/exit`, `/clear`, `/resume`, logout) | `reason` | yes |
| `TaskCreated` / `TaskCompleted`, `TeammateIdle`, `CwdChanged`, `FileChanged`, `ConfigChange`, `InstructionsLoaded`, `MessageDisplay`, model switch | — | — | no (no state meaning) |

What the transcript adds (read-only look at existing `~/.claude/projects/**/*.jsonl`): an interrupt writes a user entry `[Request interrupted by user]` (77 seen) or `[Request interrupted by user for tool use]` (43 seen); a rejected question or plan writes an error `tool_result` "The user doesn't want to proceed with this tool use…"; an answered question "Your questions have been answered: …" / "The user answered: …"; an approved plan "User has approved your plan…". Hive already reads this file (`transcript::Usage`) for tokens.

## 3. Every situation

"Fires" is from the docs unless marked (to record). "Today" is what `states.rs` does now.

| # | Situation | Hooks that fire | Hooks that don't | Today | Problem | Proposed |
|---|---|---|---|---|---|---|
| 1 | Session opened, no prompt yet | `SessionStart` (`startup`/`resume`/`fork`) | — | 🟢 idle | — | 🟢 idle |
| 2 | Working (thinking, writing text) | `UserPromptSubmit`, then tool events | nothing while it writes text | 🔵 | — | 🔵 |
| 3 | Tool running (short or long, e.g. `sleep 45`) | `PreToolUse` … `PostToolUse`/`Failure` | nothing during the call | 🔵 + activity | rule 2 flips it to 🟠 if the spinner stops writing (to record, spike Q2) | 🔵 + activity |
| 4 | Permission prompt | `PreToolUse`, `PermissionRequest` at once; `Notification permission_prompt` ~6 s later if away | nothing while it waits | 🟡 then 🟠 after 5 s (rule 2), 🟡 again at ~6 s, 🟠 for good at ~11 s | rule 2 | 🟡 **permission** until the next hook |
| 5 | Multiple-choice question (AskUserQuestion) | `PreToolUse` `AskUserQuestion`; `PermissionRequest`? (to record); `Notification`? (to record) | — | 🔵 then 🟠 after 5 s | same as a finished turn | 🟡 **question** |
| 6 | Plan approval (ExitPlanMode) | `PreToolUse` `ExitPlanMode` (`tool_input.plan`), likely `PermissionRequest` (to record) | — | 🔵 or 🟡, then 🟠 | same | 🟡 **plan** |
| 7 | MCP elicitation (form or URL) | `Elicitation` (not registered), `Notification elicitation_dialog` / `elicitation_url_dialog` ~6 s | — | 🟡 only via the ~6 s notification, then 🟠 | late, then decays | 🟡 **question** |
| 8 | Answered / approved | `PostToolUse` of that tool, then more tool events | — | 🔵 | — | 🔵 |
| 9 | Declined with Esc (permission, question, plan) | error `tool_result` in the transcript; any hook? (to record) | `Stop` | 🟠 by rule 2 | right by accident | 🟠, not pending (the user is at the keyboard) |
| 10 | Turn finished | `Stop` | — | 🟠 | — | 🟠 |
| 11 | Turn finished, background tasks still running | `Stop` with non-empty `background_tasks`; later wake-up events (to record) | — | 🟠 | nothing says work is still going | 🟠 + activity "N background tasks" (question Q3) |
| 12 | Interrupted (Esc / Ctrl+C) while working | none; transcript `[Request interrupted by user…]` | `Stop` | 🟠 after 5 s silence | a 5 s guess; alerts the user who just pressed Esc | 🟠, not pending; from the transcript line, rule 2 as fallback |
| 13 | API error (rate limit, auth, overload…) | `StopFailure` (`error`) | `Stop` | 🔴 | — | 🔴 |
| 14 | Usage limit, auto-resume | `StopFailure rate_limit` (to record), `Notification quota_auto_resume_fired` / `_stale` / `_disabled` | — | 🔴, stays | never leaves 🔴 until the next prompt | `fired` → 🔵; `stale`, `disabled` → 🟠 |
| 15 | Idle for 60 s after a turn | `Notification idle_prompt` | — | 🟠 | — | 🟠 (no change) |
| 16 | Compacting (auto mid-turn, or `/compact`) | `PreCompact`, `PostCompact`, then `SessionStart source=compact` (same `session_id`) | — | 🟢 idle, agent rebuilt (subagents and tokens lost) | reset mid-turn | 🔵 + activity "Compacting"; `SessionStart compact` keeps the agent as it is |
| 17 | Subagents running | `SubagentStart`, their tool events with `agent_id`, `SubagentStop` | — | 🟣 (most urgent wins) | — | 🟣; a subagent's 🟡 (permission/question) shows on the parent |
| 18 | Subagent waiting on its own background launch | `SubagentStop` with its launch in `background_tasks` | — | kept listed, 🔵 (5.10) | — | unchanged |
| 19 | Background session / teammate asks | `Notification agent_needs_input` | — | 🟠 | it is a question on screen | 🟡 **question** |
| 20 | `/clear`, `/resume` | `SessionEnd` (`clear`/`resume`), `SessionStart` with a new `session_id` | — | row removed, new row 🟢 | — | unchanged |
| 21 | Ended (`/exit`, Ctrl+D ×2) | `SessionEnd` | — | ⚫ then removed | — | unchanged |
| 22 | `claude` killed / crashed | none | `SessionEnd` | stays in the last state; the terminal's exit removes it | — | unchanged |

## 4. Proposed state table (for `docs/hive.md`, in Portuguese)

The three "needs your decision" states share the 🟡 colour (`--state-permission`, `#DEC184`) and urgency, and differ in icon and label, so no new colour token is needed. Emoji stay as the shorthand used across `docs/hive.md`.

### Mapeamento de estados (hooks do Claude Code → estado visual)

| Estado | Origem | Urgência |
|---|---|---|
| 🟢 Ocioso | SessionStart (`startup`, `resume`, `clear`, `fork`) | nenhuma |
| 🔵 Trabalhando | UserPromptSubmit, PreToolUse (exceto AskUserQuestion e ExitPlanMode), PostToolUse, PostToolUseFailure, PreCompact (atividade "Compacting"); Notification `elicitation_complete`, `elicitation_response`, `quota_auto_resume_fired` | baixa |
| 🟡 Aguardando permissão | PermissionRequest (exceto AskUserQuestion e ExitPlanMode); Notification `permission_prompt` | alta 🚨 |
| 🟡 Aguardando resposta | PreToolUse/PermissionRequest `AskUserQuestion`; Elicitation; Notification `elicitation_dialog`, `elicitation_url_dialog`, `agent_needs_input` | alta 🚨 |
| 🟡 Aguardando aprovação do plano | PreToolUse/PermissionRequest `ExitPlanMode` | alta 🚨 |
| 🟠 Aguardando você | Stop; Notification `idle_prompt`, `quota_auto_resume_stale`, `quota_auto_resume_disabled`; interrupção (regra 2) | média |
| 🔴 Erro | StopFailure | alta 🚨 |
| 🟣 Com subagentes | SubagentStart / SubagentStop | baixa |
| ⚫ Encerrado | SessionEnd | nenhuma |

**Regras:**

1. **O mais urgente vence:** um subagente em qualquer 🟡 põe o pai nesse 🟡, não em 🟣. Entre os 🟡: permissão > plano > resposta.
2. **Interrupção:** o `Stop` não dispara quando o usuário interrompe (Esc/Ctrl+C) nem quando recusa uma permissão, pergunta ou plano. O serviço detecta a interrupção pela linha `[Request interrupted by user…]` (ou pelo `tool_result` de recusa) no transcript da sessão e leva o agente para 🟠 **sem pendência, som nem notificação** (o usuário está no teclado). Reserva: se o agente está em 🔵 e o PTY fica `agents.silence_secs` sem saída nem evento, ele vai para 🟠. **Os três 🟡 não decaem por silêncio:** o diálogo é estático; saem só com o próximo evento de hook ou com a interrupção.
3. **Compactação não reinicia o agente:** o `SessionStart` com `source: "compact"` de uma sessão conhecida não muda o estado, os subagentes nem os tokens.
4. **Tarefas em segundo plano:** um `Stop` com `background_tasks` não vazio fica em 🟠 com a atividade "N background tasks" (ver pergunta Q3).

`docs/ui-reference.md` rows (English, for the same change):

| Token | Color | Icon (Phosphor, `weight="bold"`, 14 px) | Urgency |
|---|---|---|---|
| `--state-permission` | `#DEC184` | `ShieldWarningIcon` (waiting for permission) · `QuestionIcon` (waiting for your answer) · `ListChecksIcon` (waiting for plan approval) | high · alert (bell) |
| `--state-error` | `#D07277` | `XCircleIcon` | high · alert (bell) |
| `--state-you` | `#E08A5A` | `ChatCircleDotsIcon` | medium |
| `--state-working` | `#74ADE8` | `CircleDashedIcon`, spinning | low |
| `--state-subagents` | `#B477CF` | `CirclesThreeIcon` | low |
| `--state-idle` | `#A1C181` | `CheckCircleIcon` | none |
| `--state-ended` | `#878A98` | `StopCircleIcon` | none |

All names checked in the installed `@phosphor-icons/react` 2.1.10 (`dist/csr/<Name>.es.js`). Only `waiting_permission` changes icon (`CircleHalfIcon` → `ShieldWarningIcon`, so the three 🟡 read as one family); the others are what `src/shell/icons.tsx` already uses. Glossary labels: "waiting for permission", "waiting for your answer", "waiting for plan approval".

## 5. Questions for the human

- **Q1. Three 🟡 states or one?** Recommendation: three (same colour and urgency, different icon and label), because the sidebar row then says what to do. Minimal alternative: keep one 🟡 and only fix rules 2 and 3; the question already stops looking like a finished turn.
- **Q2. Interrupt from the transcript.** Recommendation: yes, with rule 2 as the fallback for 🔵 only. It needs reading the transcript tail every tick while an agent is 🔵 or 🟡 (the service already polls it for tokens). Alternative: keep only rule 2, but not for 🟡: then a declined dialog stays 🟡 until the next prompt.
- **Q3. Turn finished with background tasks** (`Stop` with `background_tasks`): 🟠 with an activity (recommended: the user can talk to it, and a `Monitor`/`tail -f` task never ends, so 🔵 could stick forever) or 🔵 until they end?
- **Q4. Interrupted = 🟠 not pending** (no bell, sound or OS notification, since the user caused it). Recommended. Alternative: a separate "Interrompido" state (not recommended: one more icon for the same next step).
- **Q5.** Register `PreCompact`, `PostCompact` and `Elicitation` in the generated hooks settings (#27 applies: synchronous, 1 s timeout). Recommended; the others in §2 stay out.

## 6. Recording procedure (for the human)

Never in a Hive terminal (the `claude` wrapper there adds its own `--settings`): use a plain WSL shell. Everything lands in `/tmp/hive-spike` (the scratch repo, model `haiku`, `--strict-mcp-config`, `~/.claude/settings.json` untouched). `setup.sh` wipes a previous `/tmp/hive-spike`. The recorder now also registers `Elicitation`, `ElicitationResult`, `TaskCreated` and `TaskCompleted` (`scripts/spike/lib.sh`).

```sh
scripts/spike/setup.sh                 # builds hive (release) and prepares /tmp/hive-spike
scripts/spike/run.sh observe           # starts claude there; hook calls → /tmp/hive-spike/records/observe.jsonl
scripts/spike/mark.sh <what you did>   # from a second shell, at each marked step (→ target/spike/marks.tsv)
```

Wait for Claude to finish each step before the next, unless the step says otherwise. Steps marked ⏱ need you to **not type** for the time given (the ~6 s and ~60 s notifications only fire when you look away).

| Step | Situation (§3) | Do |
|---|---|---|
| 1 | 5, 8 | `Use the AskUserQuestion tool to ask me whether I prefer red or blue, then write my answer to colour.txt.` ⏱ 15 s on the question, then answer **red**. Approve the write when asked, after ⏱ 10 s on that prompt (4). |
| 2 | 5, 9 | Same prompt again. On the question press **Esc**. `mark.sh esc on question`. ⏱ 10 s. |
| 3 | 6, 8 | Press **Shift+Tab** until the footer says plan mode. `Plan how to append "planned" to notes.txt. Keep the plan to two lines.` ⏱ 15 s on the plan approval, then approve (**Yes, and manually approve edits**). Approve the edit. |
| 4 | 6, 9 | Shift+Tab to plan mode again, same prompt. On the plan approval press **Esc**. `mark.sh esc on plan`. ⏱ 10 s. |
| 5 | 4, 9 | `Run the shell command "rm -f colour.txt".` On the permission prompt press **Esc**. `mark.sh esc on permission`. ⏱ 10 s. |
| 6 | 3, 12 | `Run the shell command "sleep 45" and then say done.` Approve; ⏱ let it finish. Then the same prompt again, and press **Esc** while `sleep` runs; `mark.sh esc on tool`. ⏱ 10 s. |
| 7 | 12 | `Write the numbers from 1 to 300, one per line.` Press **Esc** while it writes; `mark.sh esc on text`. ⏱ 10 s. |
| 8 | 11 | `Run "sleep 40" in the background (run_in_background), do not wait for it, and end your turn. When it finishes, say "background done".` ⏱ 60 s: note whether it wakes up by itself; `mark.sh background woke` when it does. |
| 9 | 17 | `Use a general-purpose subagent to run the shell command "sleep 20" and report back.` Approve the subagent's prompt if one appears (note whether it names the subagent). |
| 10 | 15 | After a finished turn, ⏱ **70 s** without typing (`idle_prompt`). |
| 11 | 16 | Type `/compact`. |
| 12 | 20, 21 | Type `/clear`, then `/exit`. |

Optional, only if easy: an API error for `StopFailure` (e.g. start `run.sh` with the network off) and an MCP server that asks for input (elicitation). Both are covered by the docs; skip them otherwise.

**Hand-off.** Then run, in a plain shell:

```sh
cp -r ~/.claude/projects/-tmp-hive-spike-repo /tmp/hive-spike/transcripts
claude --version > /tmp/hive-spike/version.txt
```

and tell the agent "7.6 recordings ready", plus anything that looked odd. The agent reads `/tmp/hive-spike/records/observe.jsonl` (hook calls with timestamps), `/tmp/hive-spike/logs/observe-*.{out,timing}` (terminal output with timing: does anything write while a dialog is open or during `sleep 45`?), `/tmp/hive-spike/transcripts/` (interrupt and refusal lines) and `target/spike/marks.tsv`. Everything stays on this machine (`/tmp`, `target/` is ignored by git); the records hold full prompts, so none of it is committed.

What the recordings must answer:

- R1. Which hooks fire for AskUserQuestion and ExitPlanMode (`PermissionRequest`? `Notification`? with which `notification_type`)?
- R2. What fires when a question, a plan or a permission is declined with Esc (`PostToolUseFailure` with `is_interrupt`? nothing?), and the exact transcript lines written.
- R3. Does the terminal print anything while a dialog is open, and during `sleep 45` (the spinner question of spike 1.12)?
- R4. Order and payloads of `PreCompact` / `PostCompact` / `SessionStart compact` (same `session_id`? is there a `Stop` after `/compact`?).
- R5. After a `Stop` with `background_tasks`, which hooks fire when the task ends and Claude wakes up (`UserPromptSubmit`? only `PreToolUse`/`Stop`?).
- R6. Does a subagent's permission prompt carry `agent_id` in `PermissionRequest`?

## 7. Draft implementation tasks (after approval)

1. **Protocol.** `AgentState` gains `WaitingAnswer` and `WaitingPlan` between `Error` and `WaitingPermission` (urgency order Ended < Idle < Working < WithSubagents < WaitingYou < Error < WaitingAnswer < WaitingPlan < WaitingPermission); `pending()` unchanged (≥ WaitingYou). `Notification` gains the new documented types or keeps them as `Other` and the adapter maps by string. `docs/architecture.md` message catalog and the state paragraph updated.
2. **Adapter** (`crates/hive/src/adapter.rs`): `SessionStart` keeps `source`; new kinds `CompactStarted` (`PreCompact`), `CompactFinished` (`PostCompact`), `ElicitationRequested` (`Elicitation`); activity "Compacting" for `PreCompact`, "Asking a question" / "Plan ready for approval" for the two tools.
3. **State table** (`states::state_of`): tool-aware mapping for `ToolStarted`/`PermissionRequested` (`AskUserQuestion` → answer, `ExitPlanMode` → plan); `permission_prompt` does not override an answer/plan state; new notification mappings (§4 table). Unit tests per row.
4. **Rule 2 narrowed** (`Agent::reconcile`): only `Working` decays; the 🟡 states never do. Tests: a permission, a question and a plan stay 🟡 past the silence.
5. **Interrupt from the transcript** (if Q2 = yes): while an agent is 🔵 or 🟡, the daemon's tick reads the new transcript bytes (reuse the `transcript::Usage` reader and its limits) and, on a `[Request interrupted by user…]` entry or a refusal `tool_result`, moves it to 🟠 marked seen (not pending). Integration test with a fake transcript under a temp HOME.
6. **Compaction** (`Daemon::saw` / `detect`): `SessionStart source=compact` of a known session id leaves the agent untouched (only refreshes `transcript_path`). Integration test: subagents and usage survive.
7. **Hooks settings** (`wrapper::EVENTS`): add `PreCompact`, `PostCompact`, `Elicitation` (Q5); update the `docs/hive.md` #27 wording only if the human asks.
8. **Background tasks** (Q3): `Stop` with non-empty `background_tasks` sets the activity "N background tasks" (clipped, bounded count).
9. **Frontend** (presentation only): `AgentState` type, `STATE_LABEL`, `STATE_ICON` (`ShieldWarningIcon`, `QuestionIcon`, `ListChecksIcon`), CSS colour for the new states (`--state-permission`), inbox/notify/close-app dialog lists that enumerate states, mock transport; unit tests for each new file path and an e2e row in `e2e/states.e2e.ts`.
10. **Docs**: the human applies §4 to `docs/hive.md`; the agent updates `docs/ui-reference.md` (state rows, glossary) and `docs/architecture.md`.

## 8. Applied (for `docs/hive.md`)

Implemented as in §4, with Q1–Q5 as recommended. The rows below are what the code now does, in `docs/hive.md`'s wording, for the human to copy into "Mapeamento de estados" (the agent never edits `docs/hive.md`).

| Estado | Origem | Urgência |
|---|---|---|
| 🟢 Ocioso | SessionStart (`startup`, `resume`, `clear`, `fork`) | nenhuma |
| 🔵 Trabalhando | UserPromptSubmit, PreToolUse (exceto AskUserQuestion e ExitPlanMode), PostToolUse, PostToolUseFailure, SubagentStart, PreCompact (atividade "Compacting"); Notification `elicitation_complete`, `elicitation_response`, `quota_auto_resume_fired` | baixa |
| 🟡 Aguardando permissão | PermissionRequest (exceto AskUserQuestion e ExitPlanMode); Notification `permission_prompt` | alta 🚨 |
| 🟡 Aguardando aprovação do plano | PreToolUse/PermissionRequest `ExitPlanMode` | alta 🚨 |
| 🟡 Aguardando resposta | PreToolUse/PermissionRequest `AskUserQuestion`; Elicitation; Notification `elicitation_dialog`, `elicitation_url_dialog`, `agent_needs_input` | alta 🚨 |
| 🟠 Aguardando você | Stop (com `background_tasks`: atividade "N background tasks"); Notification `idle_prompt`, `quota_auto_resume_stale`, `quota_auto_resume_disabled`; interrupção (regra 2) | média |
| 🔴 Erro | StopFailure | alta 🚨 |
| 🟣 Com subagentes | SubagentStart / SubagentStop | baixa |
| ⚫ Encerrado | SessionEnd | nenhuma |

**Regras (aplicadas):**

1. **O mais urgente vence:** urgência, da maior para a menor: permissão > plano > resposta > erro > aguardando você > com subagentes > trabalhando > ocioso > encerrado. Um subagente em qualquer 🟡 põe o pai nesse 🟡.
2. **Interrupção:** enquanto o agente está 🔵, 🟣 ou 🟡, o serviço lê a cada segundo as linhas novas do transcript da sessão; se a última mensagem da conversa principal é `[Request interrupted by user…]` ou a recusa de uma ferramenta (o `toolUseResult` "User rejected tool use" que o próprio Claude Code grava, com o `tool_result` em erro; recusar com instruções grava outro valor e o Claude continua), o agente e seus subagentes em 🔵/🟡 vão para 🟠 **sem pendência, som, item na caixa de entrada nem notificação** (campo `interrupted` do `agent_state`), até o estado mudar de novo. O que o transcript já tinha quando o agente foi detectado não conta. Reserva: 🔵 com o PTY `agents.silence_secs` em silêncio vai para 🟠 (como antes, com alerta). **Os três 🟡 não decaem por silêncio.**
3. **Compactação:** `PreCompact` mostra 🔵 "Compacting" e guarda o estado; `PostCompact` devolve o estado e a atividade de antes. O `SessionStart` com `source: "compact"` de uma sessão conhecida não muda nada (estado, subagentes, tokens).
4. **Lembrete de permissão:** o `Notification permission_prompt` (~6 s depois de qualquer diálogo) não transforma uma pergunta ou um plano em permissão.
5. **Tarefas em segundo plano:** `Stop` com `background_tasks` não vazio fica em 🟠 com a atividade "N background tasks"; o próximo `UserPromptSubmit` limpa a atividade.
6. **Hooks registrados** (#27, síncronos, 1 s): os 12 de antes mais `PreCompact`, `PostCompact` e `Elicitation`.

Added beyond §7: the `interrupted` field on `agent_state`, because the app cannot otherwise tell an interrupt (no tone, Q4) from a turn finished in view (tone, hive.md item 5); and `PostCompact` restoring the pre-compaction state, so a `/compact` typed after a turn goes back to 🟠 instead of staying 🔵 until the silence fallback.

To check with the recordings (§6): R1 whether AskUserQuestion/ExitPlanMode also fire `PermissionRequest` (handled either way); R2 that declining writes exactly those transcript lines; R4 the compaction order (a `Stop` after `/compact` would also be fine); R5 which hooks wake a `Stop` with background tasks.
