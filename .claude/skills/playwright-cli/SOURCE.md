# Source of `playwright-cli`

- **Upstream:** https://github.com/microsoft/playwright-cli, path `skills/playwright-cli/`
- **Commit:** `dc5f4d5616ab5a28ded12fd7101a4d49bd59a820`
- **License:** Apache-2.0
- **Review:** `docs/review/skills.md` (task 28.3); recommendation **Adapt**, approved by the human on 2026-11-27.
- **Copied:** Hive task 28.4. Fetched on 2026-11-27 as the pinned commit's tarball (`codeload.github.com/<repo>/tar.gz/<commit>`) into a new empty folder outside the repository (the agent's worktree isolation refused a `git clone` there); read as text, no script from it was run.

## What changed

- `SKILL.md` rewritten (no `allowed-tools`; the upstream line pre-approved a package runner that Hive blocks): the repository's own `playwright cli` (Playwright 1.65), run from a scratch folder so `.playwright-cli/` artifacts stay out of the repository; only the dev server on the mock transport with its own `E2E_PORT`; `bun run e2e` for the specs.
- Dropped: `attach --extension` / `--cdp`, persistent profiles, storage-state and cookie commands, WebMCP, video, PR attachments (`gh`), global installs and `npx`, and the references for them (`pr-attachments`, `session-management`, `storage-state`, `video-recording`, `running-code`, `request-mocking`, `tracing`, `playwright-tests`).
- `references/test-generation.md` adapted: plan from the `TODO.md` task (written to the scratchpad), specs in `e2e/*.e2e.ts`, no `npm init playwright`, and `test.fixme` replaced by asking the human (rule 6). `references/element-attributes.md` kept with `pw` in its commands.
- Apache-2.0: the license text is in `LICENSE.txt` next to this file; this is a modified version.

## License notice

Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0; the full text is in `LICENSE.txt` in this folder. The files here are modified (see above).
