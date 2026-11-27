---
name: semgrep
description: >-
  Runs a Semgrep static-analysis scan of Hive with a fixed ruleset (Rust, TypeScript,
  React, GitHub Actions, secrets), writes SARIF to a temp folder and triages the findings.
  Use for a whole-repo security pass (the security audit, 32.5) or when the human asks for
  a Semgrep scan. Semgrep is not installed on the host: check first, never install it.
allowed-tools: Bash Read Glob Grep
---

# Semgrep scan (Hive)

A first-pass static analysis that complements the `security-reviewer` agent; it never
replaces it. Findings go through `fp-check` before anyone acts on them, and `sarif-parsing`
reads the output.

## Essential principles

1. **Semgrep is a host tool the human installs, never you.** The human approved its use on
   2026-11-27, but not an install by an agent: no `pip`, `brew`, `pipx` or `uv tool install`.
   If `semgrep --version` fails, stop and say so; the scan can instead run in CI (below).
2. **Always `--metrics=off`.** Semgrep sends telemetry by default, and `--config auto` phones
   home. Every command carries `--metrics=off`; never use `--config auto`.
3. **Never write into the repository or the real `HOME`** (`CLAUDE.md` rule 5). Output goes
   to a fresh temp folder; Semgrep's own cache and settings (`~/.semgrep`) go to a temporary
   `HOME`. A stray file in the repo would also enter coverage and mutants.
4. **A fixed ruleset.** Only the registry packs below. No third-party rule repositories
   (no cloning), no Semgrep Pro, no AppSec Platform login. Adding a ruleset is the human's call.
5. **Read-only.** Never `--autofix`. A finding is a lead, not a fix order.
6. **Report what did not run.** A ruleset that failed to download or a scan that exited with an
   error goes in the report: a partial scan presented as complete is worse than none.

## The ruleset

| Pack | Covers |
|---|---|
| `p/rust` | `crates/`, `src-tauri/` |
| `p/typescript` | `src/`, `e2e/`, `test/` |
| `p/react` | the React components under `src/` |
| `p/github-actions` | `.github/workflows/` |
| `p/secrets` | the whole tree |

## Running it locally (when the human has installed Semgrep)

```bash
OUT=$(mktemp -d "${TMPDIR:-/tmp}/hive-semgrep.XXXXXX")   # outside the repository
SEMGREP_HOME=$(mktemp -d "${TMPDIR:-/tmp}/hive-semgrep-home.XXXXXX")
HOME="$SEMGREP_HOME" semgrep scan --metrics=off \
  --config p/rust --config p/typescript --config p/react \
  --config p/github-actions --config p/secrets \
  --exclude node_modules --exclude target --exclude dist --exclude docs/prototype \
  --max-target-bytes 20000000 \
  --json --output "$OUT/results.json" \
  --sarif-output "$OUT/results.sarif" \
  .
```

Run it from your own worktree, at the repo root. `--json` keeps the rule metadata the
important-only filter needs; the SARIF file is what `sarif-parsing` reads. A file over
`--max-target-bytes` is skipped silently: say so in the report if any source is that large.
Check the CLI flags against `semgrep scan --help` of the installed version before the first run.

**Modes.** *Run all*: every finding. *Important only*: add `--severity WARNING --severity ERROR`
and apply the metadata post-filter of [references/scan-modes.md](references/scan-modes.md)
(security category, medium/high confidence and impact) to `results.json`. Ask the human which
mode when it is not obvious from the task; the audit (32.5) uses *run all*.

## Running it in CI (for the human to decide)

The host does not need Semgrep if the scan runs in GitHub Actions: a job in a new or existing
workflow using the `semgrep/semgrep` container image (pinned by digest, like Hive's other
actions), running the command above with `--metrics=off`, writing the SARIF as a workflow
artifact. Adding or changing a workflow is a separate task the human approves, and the personal
token needs the `workflow` scope to push it (`CLAUDE.md`, Working notes). Do not add it on your own.

## Triage

1. Read `results.sarif` with the `sarif-parsing` skill (count by rule and file, dedupe).
2. For each finding, read the code at the reported line and follow the data from where it
   enters (hook payloads, socket frames, file names, environment, git output: the trust
   boundaries listed in `.claude/agents/security-reviewer.md`).
3. Verify anything you would report with the `fp-check` skill: TRUE or FALSE POSITIVE, with the
   evidence.
4. Report: what ran (packs, mode, Semgrep version, files skipped), what failed, and the true
   positives most severe first (file:line, the input that triggers it, the fix). Never silence
   a rule in the code (`nosemgrep`) to make a scan clean; a false positive is explained in the
   report.

## Rationalizations to reject

| Shortcut | Why it is wrong |
|---|---|
| "Semgrep is missing, I'll install it quickly" | Installing a host tool is the human's decision. Stop and report. |
| "I'll write the output next to the code" | Rule 5; a stray file enters coverage and mutants. Temp folder only. |
| "`--config auto` picks better rules" | It sends metrics and changes the ruleset under you. Fixed packs only. |
| "Some packs failed, the rest is enough" | Report the failures; a partial scan read as complete hides gaps. |
| "Add `nosemgrep` to quiet a false positive" | Silencing a check is weakening a gate in spirit; explain it in the report. |
