# Test generation (plan → generate → heal), for Hive's e2e specs

Authoring and maintaining Playwright specs with the Playwright CLI. Every CLI action emits the
equivalent Playwright TypeScript, and that generated code is the raw material for every test.
`pw` stands for `cd "$OUT" && "$PW" cli` (see `SKILL.md`: the repository's binary, run from a
scratch folder). Hive's specs live in `e2e/*.e2e.ts`, run against the Vite dev server on the mock
transport (`e2e/playwright.config.ts`, `E2E_PORT`), on Chromium and WebKit.

---

## 0. How generation works

```bash
pw open "http://localhost:$E2E_PORT"
pw snapshot
# Output shows: e1 [textbox "Branch name"], e3 [button "Create"]
pw fill e1 "feature/login"
# Ran Playwright code:
# await page.getByRole('textbox', { name: 'Branch name' }).fill('feature/login');
pw click e3
# Ran Playwright code:
# await page.getByRole('button', { name: 'Create' }).click();
```

### Use semantic locators

```typescript
// Generated (good - semantic)
await page.getByRole('button', { name: 'Commit' }).click();

// Avoid (fragile - CSS selectors)
await page.locator('#commit-btn').click();
```

Read the existing specs first and reuse their helpers (`e2e/host.ts`, the mock's
fixtures) rather than writing new ones.

### Add assertions

Generated code captures actions but not assertions. Add expectations with one of:

- `toBeVisible()` — element is rendered and visible
- `toHaveText(text)` — element text content matches
- `toHaveValue(value) / toBeEmpty()` — input/select value matches
- `toBeChecked() / not.toBeChecked()` — checkbox state matches
- `toMatchAriaSnapshot(snapshot)` — page (or locator) matches a partial accessibility snapshot

Use `pw --raw generate-locator <ref>` for the locator expression, and `pw --raw eval
"el => el.textContent" <ref>` / `pw --raw snapshot <ref>` for the expected value. When asserting
text, the locator must not contain the text itself (`getByTestId()` or `getByLabel()` work well);
with a text-based locator prefer `toBeVisible()`. An aria snapshot only needs what the assertion is
about; use regular expressions for unstable values.

```typescript
await expect(page.getByRole('alert')).toBeVisible();
await expect(page.getByRole('navigation')).toMatchAriaSnapshot(`
  - button "Changes"
  - button /\\d+ agents?/
`);
```

Each assertion checks behaviour the task asks for (`CLAUDE.md` rule 6: no test that only runs
code). The expected values come from the spec or the prototype, never from what the app happens
to print today.

---

## 1. Plan

Goal: know which scenarios to test before writing them. For a Hive task the scenarios come
from the `TODO.md` task (its "Tests:" line) and the `docs/hive.md` decisions it cites; write
the plan in your scratchpad, never in the repository.

Explore the flow through an existing spec's setup, not by opening the URL alone (the setup puts
the mock in the right state): run the closest spec with `--debug=cli` in the background, attach
to its `tw-…` session, resume, then probe:

```bash
E2E_PORT=$E2E_PORT PLAYWRIGHT_HTML_OPEN=never bun run e2e e2e/<closest>.e2e.ts --project=ui --debug=cli
pw attach tw-XXXX
pw resume
pw snapshot
pw click e5
```

Map: the interactive surfaces, the main journey end to end, edge cases (empty states, errors,
very long names), keyboard paths (#35 shortcuts), light and dark themes, reduced motion. Stop the
background test when done.

## 2. Generate

For each scenario, one at a time (they share the debug session): walk the steps with `pw`,
treating the plan as the plan and the live app as the source of truth, and collect the generated
code into the spec file. If a step contradicts the app, that is a question (section 3.4), not an
edit to the decision.

Rules:

- Follow the existing specs' layout: a test name that says the behaviour, the spec file named
  after the feature (`e2e/<feature>.e2e.ts`).
- Close the CLI session and stop the background test before the next scenario.
- Run the new spec once on both projects:
  `E2E_PORT=$E2E_PORT PLAYWRIGHT_HTML_OPEN=never bun run e2e e2e/<feature>.e2e.ts`.

## 3. Heal

### 3.1 Find failing tests

Run the specs of what you touched; record each failing `<file>:<line>` and fix them one at a
time (shared state and the single CLI session make parallel fixes fragile).

### 3.2 Debug one failure

```bash
E2E_PORT=$E2E_PORT PLAYWRIGHT_HTML_OPEN=never bun run e2e e2e/<feature>.e2e.ts:<line> --project=ui --debug=cli
pw attach tw-XXXX
pw snapshot      # did the element change / move / rename?
pw console       # app-side errors?
```

Common causes: locator drift, a new wrapper element, a label or ARIA rename, timing (a
transition, an async update from the mock), assertion text changed in the app, state leaking
between tests. A failure only under load: re-run before concluding it is real.

### 3.3 Apply the fix

Update the locator, assertion, step order or inputs; stop the debug run; re-run the single test.
**Never add sleeps, never use `networkidle`, never skip hooks**: wait on a web-first assertion
(`await expect(locator)…`, `expect.poll`).

### 3.4 Spec or app?

- **Purely technical fix** (locator drift, better assertion shape) → done.
- **The app's visible behaviour changed** → check it against the task and `docs/hive.md`. If the
  change is what the task asked for, update the test.
- **Unclear whether the app change is intended or a regression** → stop and ask the human, with
  the test, the decision it checks and what the app does now. Never `test.fixme` or `test.skip`
  it, and never loosen the assertion to make it pass (`CLAUDE.md` rule 6).
