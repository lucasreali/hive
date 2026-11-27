---
name: web-design-guidelines
description: Review Hive's UI code (React components and CSS under src/) against interface guidelines - accessibility, focus, reduced motion, content overflow, long lists, theming, destructive actions - and report terse file:line findings. Use for UI tasks before finishing, when asked to "review the UI", "check accessibility" or "audit the design", and in the frontend audit.
metadata:
  author: vercel
  version: "1.1.0"
  argument-hint: <file-or-pattern>
---

# Web Interface Guidelines (adapted for Hive)

Review files for compliance with the interface guidelines in
[references/command.md](references/command.md).

## The look's authority

`docs/prototype/` (the human's prototype), `docs/ui-reference.md` (tokens and the PT→EN
glossary) and `docs/hive.md` decide how Hive looks, reads and behaves; shortcuts follow
`docs/hive.md` #35. When a guideline here disagrees with them, they win: report the
disagreement as a question for the human, never as a finding to fix. Never edit `docs/hive.md`
or `docs/prototype/`.

## How it works

1. Read [references/command.md](references/command.md). Do not fetch rules from the network.
2. Read the files to review (the ones named, or the components the task changed).
3. Check them against every rule.
4. Output findings in the terse `file:line` format the guidelines give.

Where the guidelines say `$ARGUMENTS`, use the files or pattern you were asked to review. A
finding that needs a real browser to confirm (focus order, contrast in both themes, reduced
motion) is checked in the Playwright e2e suite on the mock transport, not guessed.

## Source

Rules taken from https://github.com/vercel-labs/web-interface-guidelines/blob/434b7f91364665f2f733b310ec54809bf8f37937/command.md
(MIT, Copyright (c) 2025 Vercel Labs), as bundled by vercel-labs/agent-skills; trimmed to what
applies to a desktop Tauri app (see `SOURCE.md`). There is no update procedure: a newer upstream
version is a new task.
