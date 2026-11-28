---
name: playwright-cli
description: Drive Hive's frontend in a browser against the mock transport with the Playwright CLI (snapshots with element refs, click/fill/press, generated locators), and write, generate or heal the Playwright e2e specs under e2e/. Use for UI tasks that add or fix an e2e spec, to explore a UI flow before testing it, or to debug a failing e2e test.
---

# Browser automation with the Playwright CLI (Hive)

The CLI ships with the repository's own `@playwright/test` (1.65; `playwright cli`), so nothing is
installed globally. Every action prints the equivalent Playwright TypeScript, which is the raw
material for an e2e spec.

## Hive's rules

- **Only the dev server on the mock transport.** Start it on a free port of your own (other
  worktrees run theirs): `E2E_PORT=$((1430 + RANDOM % 500))`, `bun run dev --port $E2E_PORT`
  in the background. Never the installed Hive app, never a real site, never the real `claude`.
- **Never attach to the human's browser.** No `attach --extension`, `attach --cdp`,
  `--profile` or `--persistent`: those reach the real browser profile (`CLAUDE.md` rule 5). The
  only `attach` allowed is to a paused test's session (`tw-…`, below).
- **Artifacts stay out of the repository.** The CLI writes snapshots, traces and saved state
  into its working directory (`.playwright-cli/`). Run it from a scratch folder with the
  repository's binary:

  ```bash
  PW="$(git rev-parse --show-toplevel)/node_modules/.bin/playwright"   # in your worktree
  OUT=<your scratchpad>/pw-<task>; mkdir -p "$OUT"
  cd "$OUT" && "$PW" cli open "http://localhost:$E2E_PORT"
  cd "$OUT" && "$PW" cli snapshot
  ```

  (Below, `pw` stands for `cd "$OUT" && "$PW" cli`.) A test run writes to `target/e2e`
  (`e2e/playwright.config.ts`), which git ignores.
- **No PR attachments, no global installs, no `gh`** (only `scripts/ci.sh` uses gh).
- **Never skip a test.** No `test.skip`, `test.fixme` or `.only` (`CLAUDE.md` rule 6); a test
  that cannot pass because the app is wrong goes to the human.

## Commands

```bash
pw open "http://localhost:$E2E_PORT"
pw snapshot                     # page structure with refs (e1, e2, …)
pw snapshot e34                 # a region only; --depth=4 for a shallow view
pw find "Commit"                # search the snapshot (--regex "/commit|push/i")
pw click e15
pw dblclick e7
pw fill e5 "feature/login" --submit
pw type "text"
pw press Enter                  # keys: ArrowDown, Escape, Meta+K, …
pw keydown Shift; pw keyup Shift
pw hover e4
pw drag e2 e8
pw check e12; pw uncheck e12
pw select e9 "option-value"
pw eval "document.title"
pw eval "el => getComputedStyle(el).transitionProperty" e5
pw resize 1440 900              # the e2e viewport
pw set-color-scheme dark        # light/dark themes
pw set-reduced-motion reduce    # prefers-reduced-motion
pw set-forced-colors active
pw console                      # app-side errors
pw generate-locator e5 --raw    # the locator expression for an assertion
pw screenshot --filename="$OUT/after.png"
pw tracing-start; pw tracing-stop
pw close
```

Target elements by snapshot ref; CSS selectors and Playwright locators work too
(`pw click "getByRole('button', { name: 'Commit' })"`). When the snapshot does not show an
attribute, read it with `eval` ([references/element-attributes.md](references/element-attributes.md)).
`--raw` strips page status and generated code, for piping (`pw --raw snapshot > "$OUT/before.yml"`).

## Running and debugging the e2e specs

```bash
E2E_PORT=$E2E_PORT PLAYWRIGHT_HTML_OPEN=never bun run e2e                            # all (Chromium + WebKit)
E2E_PORT=$E2E_PORT PLAYWRIGHT_HTML_OPEN=never bun run e2e e2e/commit.e2e.ts       # one spec
E2E_PORT=$E2E_PORT PLAYWRIGHT_HTML_OPEN=never bun run e2e e2e/commit.e2e.ts:42 --project=ui --debug=cli
```

`--debug=cli` pauses the test at its start and prints a session name (`tw-…`): run it in the
background, wait for "Debugging Instructions", then `pw attach tw-…` and explore the paused page
(`pw snapshot`, `pw console`). Stop the background run when done. Specs press the host's keys
(`e2e/host.ts`); timing specs (`load`, `tree-perf`) run alone on Chromium.

To write, generate or heal a spec, follow [references/test-generation.md](references/test-generation.md).
