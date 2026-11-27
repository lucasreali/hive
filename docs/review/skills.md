# Review 28.3: skills that help develop Hive

Task 28.3 (`TODO.md`). No code; nothing was installed into `.claude/`. Written 2026-11-27 against `main` at `0fa77ce1`. Every candidate was shallow-cloned into a temporary folder outside the repository and read in full as text (very large, repetitive test fixtures were skimmed); **no candidate script was run**. Recommendations: **take** (copy as is, after the review), **adapt** (copy with the changes listed), **skip**. The human picks (⏸).

## 1. Summary

| Recommendation | Skills |
|---|---|
| **Take** | `verification-before-completion`, `receiving-code-review` (superpowers, minus its GitHub-replies section) · `property-based-testing` (ToB; proptest itself needs approval) · `sarif-parsing` (ToB, only with semgrep) · `composition-patterns` (Vercel) |
| **Adapt** | `test-driven-development` + `writing-good-tests.md`, `systematic-debugging`, `requesting-code-review` → a third reviewer agent (superpowers) · `fp-check`, `sharp-edges`, `semgrep`, `rust-review` as checklist text, `mutation-testing` triage method (ToB) · `react-best-practices`, `web-design-guidelines` (Vercel) · `playwright-cli` (Microsoft) · `accessibility` (addyosmani) · `rust-best-practices` (Apollo) · `animate`, `review-animations` (emilkowalski) |
| **Skip** | `frontend-design`, `webapp-testing` (Anthropic) · `using-git-worktrees`, `finishing-a-development-branch`, `subagent-driven-development`, `dispatching-parallel-agents` (superpowers) · `differential-review`, `entry-point-analyzer`, `supply-chain-risk-auditor`, `codeql`, `agentic-actions-auditor`, `second-opinion`, `code-improver`, `cargo-fuzz`, `coverage-analysis` (ToB) · `react-view-transitions` (Vercel) · `improve-animations` (emilkowalski) |

Rules for anything adopted, whatever the source:

- Every `npm`/`npx`/`pnpm`/`pip`/`brew`/global `cargo install` example becomes `bun`/`bunx`/`cargo add`, or is removed. Left in, `guard.py` blocks the command and the agent may try to work around it.
- Cross-references to skills that are not imported (`superpowers:<name>`, `choreograph`, `pick-ui-library`) are removed or renamed.
- Any output folder a skill writes goes to the scratchpad or a temp dir, never into the repository (rule 5, and a stray file would enter coverage and mutants).
- No skill may grant itself `Write`/`Edit` through `allowed-tools` unless it is meant to write code.
- No advice to silence a gate survives (`#[allow]`, `#[expect]`, `#[ignore]`, `test.fixme`, `-ignore-filename-regex`, "equivalent mutant → skip"): rule 6.
- The look's authority stays `docs/prototype/`, `docs/ui-reference.md` and `docs/hive.md`; a design skill's advice loses on conflict.
- Licenses: copying Trail of Bits text makes that file CC-BY-SA-4.0 with attribution; the MIT and Apache-2.0 ones need their notice kept. Hive has no LICENSE of its own, so the human decides whether share-alike files are acceptable in `.claude/`.

## 2. Sources

| Source | Commit (pinned) | Date | License | Signal |
|---|---|---|---|---|
| https://github.com/anthropics/skills | `e4bb5d960d87126ff3b3a8af1a104b7db67560ba` | 2026-11-16 | Apache-2.0 (each skill's `LICENSE.txt`) | Anthropic |
| https://github.com/obra/superpowers | `22f0b463e7a5e579904dd6211a520321035526f7` | 2026-11-03 | MIT | ~328k stars, pushed 2026-11-26 |
| https://github.com/trailofbits/skills | `149d7b21f4067765b82e67029730b42d228ef4b4` | 2026-11-12 | CC-BY-SA-4.0 (repo; no per-plugin license) | Trail of Bits |
| https://github.com/vercel-labs/agent-skills | `5a37a6329fbad3666c3207392a6a7c0110e92efb` | 2026-11-20 | MIT per README and SKILL.md frontmatter; no root LICENSE file; `web-design-guidelines` states none | Vercel |
| https://github.com/microsoft/playwright-cli | `dc5f4d5616ab5a28ded12fd7101a4d49bd59a820` | 2026-11-24 | Apache-2.0 | Microsoft, ~15.5k stars |
| https://github.com/addyosmani/web-quality-skills | `afa8da942115f2961fdbfa80807ea0b232ff6c00` | 2026-08-24 | MIT | ~3.1k stars, slowest to update |
| https://github.com/apollographql/skills | `8d0129fdc96a1b80d999a6682e48fc11e137dc09` | 2026-11-27 | MIT | Apollo GraphQL |
| https://github.com/emilkowalski/skills | `64eb3deb9a5fbfa041faf783e0bc3fd635872e9c` | 2026-11-15 | MIT | ~60k stars, author of Sonner/Vaul |

Looked at and rejected as sources: `hypothesi/mcp-server-tauri` (`519313fc`, MIT, 338 stars: needs `tauri-plugin-mcp-bridge` and `withGlobalTauri: true` in the app, a runtime IPC-control surface against the security-reviewer's Tauri rules); `microsoft/playwright`'s own `.claude/skills` (for Playwright contributors); tauri-apps and motiondivision publish no skill; community Tauri, accessibility and motion skills on skills.sh (individuals or aggregators, few stars, several installed with `npx skills add`); no Rust async/tokio skill from an established org exists.

## 3. Frontend (React, design, accessibility, motion)

**frontend-design** (Anthropic) — prompt-only art direction and a list of "AI-default" looks. Overlap: none. Conflict: "take aesthetic risk" contradicts the prototype as the look's source. Risks: none (no scripts). **Skip**; at most its "writing in design" (UI copy) section could go into the task brief.

**react-best-practices** (Vercel) — 70 React/Next.js performance rules in 8 groups. Applies to a Tauri SPA: the `rerender-*` rules on effects and state (derived state without effect, effect → event, no inline components, functional setState, lazy init, transitions, deferred value, refs for transient values), `rendering-activity`/`content-visibility`/`conditional-render`, all `js-*` and `advanced-*`, `client-passive-event-listeners`/`localstorage-schema`, `bundle-barrel-imports` (Hive imports from the `@phosphor-icons/react` barrel), `async-parallel`. Does not apply: every `server-*`, hydration, script/resource-hint, `async-api-routes`/`suspense-boundaries`, `client-swr-dedup`; redundant with the React Compiler (on in `vite.config.ts`): the `rerender-memo*` and `rendering-hoist-jsx` rules. Conflicts: `npx svgo`, `pnpm install`, rules that bring `swr`/`better-all`/`next/*`. **Adapt**: keep the rules listed, note the compiler, remove npx and new dependencies, narrow the trigger (today it fires on any React task).

**composition-patterns** (Vercel) — compound components, state in providers, explicit variants instead of boolean props, React 19 `ref` as prop and `use()`. No conflicts, no network, no scripts; fits #37 (the frontend only presents) and React 19.3. **Take** (some examples are React Native; harmless).

**web-design-guidelines** (Vercel) — reviews UI files against a bundled rule list (`file:line` findings). No fetch at run time (only its manual "Updating" section uses `gh api`/`curl`). Web-only parts (URL state, safe areas, touch, image loading, font preconnect, hydration, Accept-Language) do not apply; its Title Case and copy rules may contradict the prototype; its `references/command.md` is copied from `vercel-labs/web-interface-guidelines` with no license stated. **Adapt**: keep accessibility, focus, reduced motion, `transition: all`, truncation/`min-w-0`, virtualization, `color-scheme`, destructive confirmations; add "the prototype, ui-reference and hive.md win"; confirm the license first.

**react-view-transitions** (Vercel) — `<ViewTransition>` patterns, a third Next.js. Its "implement all applicable patterns" after an audit goes against "do only what was asked"; Hive has no router and happy-dom has no `startViewTransition` (100% coverage). **Skip** until a task asks for animated transitions.

**accessibility** (addyosmani/web-quality-skills) — WCAG 2.2 audit: focus visible and not obscured, 24 px targets, drag alternatives, ARIA tabs, live regions, reduced motion; evidence first (tree, keyboard). No overlap. Conflicts: `npx lighthouse`, `npm install -g @axe-core/cli`. Risks: none (text only); written for websites (skip links, page language, captions). **Adapt**: tooling becomes Playwright accessibility snapshots in the e2e suite (`@axe-core/playwright` would be a new dependency for the human to approve); cut website-only sections; the drag alternative matches #57 (Alt+↑/↓ reorder).

**animate, review-animations, animation-performance, improve-animations** (emilkowalski) — when to animate at all (keyboard and frequent actions: never), CSS transitions/WAAPI over libraries, `transform`/`opacity` only, curve and duration tables, `prefers-reduced-motion` always; `review-animations` gives a block/approve verdict. No overlap; matches Hive's "no motion library" and `e2e/animations.e2e.ts`. Conflicts: a canned persona reply at the start of each skill; `improve-animations` writes `plans/` in the repo and spawns worktree agents outside `/stage`; cross-links to skills not imported. Risks: none (text only). **Adapt** `animate` and `review-animations` (optionally `animation-performance`): drop the persona replies and dead links, use Hive's tokens. **Skip** `improve-animations`.

## 4. Security (Rust, Tauri, trust boundaries)

No maintained Tauri security skill exists from an established source (section 2). The existing `security-reviewer` agent stays the reviewer; the useful ToB material is checklists to fold into it.

**rust-review** (ToB) — whole-repo Rust audit: 15 clusters / 69 bug classes, ~23 workers in waves of up to 16, dedup and false-positive judges, SARIF + REPORT.md. Adds to `security-reviewer`: FFI/unsafe soundness, deadlocks, blocking in async, TOCTOU, raw fds (Hive has ~200 `unsafe` outside tests: `src-tauri/src/macos.rs`, `crates/hive/src/windows.rs`). Conflicts: writes `$(pwd)/.rust-review-results/` in the repo (`SKILL.md:151`), suggests `brew install uv`, runs `find ~/.claude`, workers get unrestricted Bash/Write. Risks: `uv run` of 3 local Python scripts, high token cost; its false-positive judge drops findings that cross no privilege boundary, while Hive's attackers act as the same user (hook payloads, repository file names). **Adapt as text**: copy the FFI and concurrency checklists into `security-reviewer.md` with Hive's trust boundaries spelled out; optionally one supervised run before a release with a temp output dir, limited to `crates/hive/src` and `src-tauri/src`.

**differential-review** (ToB) — phased diff review (risk triage, blame on removed code, test gaps, blast radius, attacker model). Conflicts: `git checkout <baseline>` (changes state in a shared checkout), a mandatory report file written to the repo, the Desktop or `~/.claude/skills/...`; `patterns.md` is mostly Solidity; test gaps are already enforced harder by coverage + mutants. **Skip**; two lines worth putting in `security-reviewer`: `git log -S` on removed validation, and "state who can write to the socket or forge a hook payload".

**sharp-edges** (ToB) — footgun review of API/config design; Read/Grep/Glob only, no conflicts. **Adapt**: the relevant parts of `config-patterns.md` become a design checklist for spike tasks (what 0/-1 means for a limit or timeout, which of file/env/CLI wins, config paths escaping their folder, `"false"` as a string); drop the crypto, auth and other-language references.

**entry-point-analyzer** (ToB) — smart contracts only. **Skip**.

**supply-chain-risk-auditor** (ToB) — npm/PyPI/Go only (no Cargo, ignores `bun.lock`); calls OSV, npm, deps.dev, Scorecard and the GitHub API with `gh auth token` (the active `gh` account is the work one); needs `uv` with Python ≥ 3.11. **Skip**. If wanted instead: tighten `cargo deny`'s unmaintained/yanked advisories, and a `bun.lock` audit tool after approval.

**static-analysis** (ToB) — `sarif-parsing`: jq recipes and a stdlib helper; **take** together with semgrep (drop its `npm install -g ajv-cli` hint). `semgrep`: output defaults to `./static_analysis_semgrep_N` in the repo, clones ~12 rule repos, its `workflows/semgrep-scan.js` skips the approval step and searches `$HOME`; telemetry off (`--metrics=off`). **Adapt**: temp output dir and temporary `HOME`, semgrep installed only with approval, a fixed ruleset (`p/rust`, `p/typescript`, `p/react`, `p/github-actions`, `p/secrets`), drop Decurity, Pro and the workflow script. `codeql`: Rust barely covered; build-fix references call pip, npm, brew, `softwareupdate` and `cargo clean` without asking; writes CodeQL's pack cache. **Skip** (a `github/codeql-action` CI job would be a separate human decision).

**agentic-actions-auditor** (ToB) — audits CI workflows that call AI agents; none of Hive's workflows does. **Skip** (reconsider if one is added).

**second-opinion** (ToB) — sends the diff, untracked files and project instructions to Codex/Gemini/Antigravity. Private code leaves the machine; installs via `npm i -g`; needs third-party accounts; headless `agy` does not make the workspace read-only (its own docs). **Skip**.

**fp-check** (ToB) — verifies an existing finding: 5 phases, 6 gates, TRUE/FALSE POSITIVE verdict; complements `security-reviewer`. Risks: `hooks/hooks.json` installs two `type: prompt` hooks on **Stop and SubagentStop with matcher `*`** (30 s each): once enabled they run for every session and subagent, `/stage` task agents and Hive's reviewers included, can block stopping and each costs a model call; its `poc-builder` agent writes and runs PoCs inside the repo (rule 5; a leftover PoC enters coverage and mutants). **Adapt**: keep the method (`SKILL.md`, `false-positive-patterns.md`, `gate-reviews.md`, `standard-verification.md`) and the two read-only agents; drop the hooks; drop `poc-builder` or limit it to pseudocode or a temp dir; remove Write/Edit.

## 5. Rust (idioms, performance, async)

**rust-best-practices** (Apollo) — Apollo's Rust handbook: borrowing vs cloning, `let-else`, thiserror/anyhow, clippy discipline, test naming, dispatch, typestate, `Send`/`Sync`; nothing substantial on async/tokio. Overlap: the no-`unwrap` baseline, the clippy gate. Conflicts: recommends `#[expect]`/`#[allow]` with a justification and mentions `#[ignore = "..."]` (rule 6), `cargo install flamegraph`/`cargo-insta`, `rustup update`, `cargo +nightly fmt`; `allowed-tools` grants Write/Edit. Risks: none (text only). **Adapt**: strip lint silencing, ignores and global installs; keep idioms, errors and typestate. Moderate value: clippy already enforces much of it.

No async/tokio skill from an established source was found.

## 6. Testing (TDD, Playwright, mutants, properties)

**test-driven-development** + `writing-good-tests.md` (superpowers) — red-green-refactor as an "Iron Law" (code written before its test is deleted); honest-test rules (no mirror assertions, no asserting on mocks, a "mutation check"). Supports rule 6 and the mutants gate. Conflicts: `npm test` commands. Risks: none. **Adapt**: `bun test` / `cargo test` (nextest), soften "delete the code" for stage tasks, keep `writing-good-tests.md` nearly verbatim.

**webapp-testing** (Anthropic) — Python Playwright with `scripts/with_server.py` starting dev servers. Replaces Hive's TS e2e on the mock transport; needs the Python `playwright` package (pip, rule 3); examples use `npm run dev`; tells the agent to run scripts with `--help` and not read their source; `with_server.py` uses `subprocess.Popen(cmd, shell=True)` (against the no-shell-string baseline) and never drains its pipes (a server can hang); writes to `/tmp`. **Skip**.

**playwright-cli** (Microsoft) — drives a browser through the `playwright-cli` binary (snapshots with element refs, click/fill, tracing, request mocking) and a plan/generate/heal workflow for `@playwright/test` specs. Its healing advice (no sleeps, no `networkidle`, ask the human when spec vs app is unclear) fits rule 6. Conflicts: `allowed-tools: Bash(npx playwright:*)`, `npm install -g @playwright/cli@latest`, `npm init playwright`, `gh pr create/comment --attach` (gh only through `scripts/ci.sh`), suggests `test.fixme` (a skip). Risks: can attach to the user's real Chrome profile (`attach --extension/--cdp`, rule 5); writes `.playwright-cli/` snapshots, traces and storage state into the cwd; needs the global binary or Playwright ≥ 1.59 locally (Hive has 1.65). **Adapt**: keep generate/heal and the assertion guidance (`toMatchAriaSnapshot`, `generate-locator`); commands become `bunx playwright …` / `bun run e2e` against the mock with `E2E_PORT`; drop attach, PR attachments, global installs and `test.fixme`; artifacts to the scratchpad.

**mutation-testing** (ToB) — mewt/muton campaigns; sorts survivors into "equivalent" vs "test gap" with priority tiers. Overlap: `cargo mutants` per diff and weekly, `/stage` step 2 (`scripts/ci.sh survivors`). Conflicts: writes `mutation-testing-report.md` in the cwd; "equivalent" invites a skip (rule 6; Hive allows no missed mutant); `tests/smoke-test.sh` runs the real `claude` and calls the API. **Adapt** as text: fold the five-check equivalence procedure and its "rationalizations to reject" into `/stage` step 2, with the rule "an equivalent mutant is refactored away or goes to the human, never skipped"; drop mewt, blockchain and report parts.

**property-based-testing** (ToB) — writes, reviews and triages property tests (roundtrip, idempotence, oracle), proptest/fast-check, calls out vacuous properties. No overlap; says adding a property library is the user's call (rule 3). Risks: `evals-extra/run.sh` and `effectiveness.sh` run the real `claude` (do not copy them); proptest writes `proptest-regressions/` in the repo and random inputs can make mutant runs flaky (fix the seed/cases in `[profile.mutants]`). Fit: the `hive-protocol` encode/decode pair is a natural roundtrip. **Take** `skills/property-based-testing/` only; proptest as a dev-dependency needs the human's approval.

**cargo-fuzz** (ToB handbook) — generic cargo-fuzz how-to. Hive already has `crates/hive-protocol/fuzz/fuzz_targets/decode.rs`. Conflicts: hand-edits `arbitrary` into `Cargo.toml` (rule 3), global `cargo install` of four tools, writes a `./generate_html` script, downloads a corpus with `curl`. **Skip** (adapt only if a fuzzing task comes).

**coverage-analysis** (ToB handbook) — coverage for fuzzing campaigns, mostly C/C++. Advises `-ignore-filename-regex` filtering (a coverage exclusion if applied to the gate, rule 6); installs gcovr into `~/.local`. **Skip**.

## 7. Refactoring, review and process

**verification-before-completion** (superpowers) — no completion claim without fresh evidence; check the diff rather than trust an agent's report. Mostly covered by `gates` and the task brief; adds "do not trust a subagent's success report" to the orchestrator. No conflicts. **Take** (or fold into `gates`/`stage`).

**requesting-code-review** + `code-reviewer.md` (superpowers) — a general reviewer on a `BASE..HEAD` diff with Critical/Important/Minor verdicts. Covers correctness and plan fit, which `gate-integrity-reviewer` and `security-reviewer` do not. Conflicts: example paths under `docs/superpowers/plans/`; the reviewer may `git worktree add /tmp/review-…`. **Adapt** as a third agent in `.claude/agents/`, the "spec" being the `TODO.md` task and the `docs/hive.md` decisions it cites, run from `/stage` before a merge; its `task-reviewer-prompt.md` / `re-review-prompt.md` (from `subagent-driven-development`) can seed it.

**receiving-code-review** (superpowers) — verify feedback technically before acting, no performative agreement; a suggestion that conflicts with the human's decisions means stop and ask (rule 1). Its GitHub-replies section uses `gh api` (only `scripts/ci.sh` may use gh). **Take** without that section.

**systematic-debugging** (superpowers) — 4-phase root-cause method, stop after 3 failed fixes; `condition-based-waiting.md` fits flaky e2e and PTY tests, `defense-in-depth.md` the trust boundaries. `find-polluter.sh` runs `npm test` per file. **Adapt**: drop or rewrite `find-polluter.sh` for bun/nextest; fix the `superpowers:` references.

**using-git-worktrees** (superpowers) — runs `npm`/`pip`/`poetry install`, may edit and commit `.gitignore`, puts worktrees in `.worktrees/` inside the repo (against the mutant-path and `TMPDIR` rules), asks consent per worktree. `/stage` and the task brief already define worktrees. **Skip**.

**finishing-a-development-branch** (superpowers) — merge/PR/keep/discard menu with raw `git push -u origin` and PR creation (rule 4: `scripts/ci.sh push`, no PRs), `git pull`/merge without green CI or the merge lock, a PR body that would carry an attribution line. **Skip**.

**subagent-driven-development** (superpowers) — one implementer per plan task in sequence, with reviewers and a fix loop. Competes with `/stage` (parallel worktrees, green CI, ⏸); "Rulings, not stalls" tells the controller to decide ambiguities itself (rule 1: decisions are the human's); forbids parallel implementers; its scripts write a `.superpowers/sdd/` workspace in the repo and `rm -rf` it at the end. **Skip** (borrow the reviewer prompts above).

**dispatching-parallel-agents** (superpowers) — a generic subset of `/stage`'s waves. No conflicts, low value. **Skip**.

**code-improver** (ToB) — autonomous review-and-fix loop (`workflows/improve.js`) driving reviewer plugins Hive does not have; writes `.code-improver/` in the repo, can `git init` and commit as `code-improver-baseline`, bumps `plugin.json` versions, runs `uv run`, searches the real `~/.claude`. Overlaps `/code-review --fix`, `/simplify` and `/stage`. **Skip**.

## 8. For the human to decide

1. Which skills to take or adapt (section 1); adapted copies go to `.claude/skills/` (or `.claude/agents/` for the reviewer) in a follow-up task.
2. Whether CC-BY-SA-4.0 text (Trail of Bits) is acceptable in the repository, with attribution.
3. Edits to `.claude/agents/security-reviewer.md` from ToB: FFI/concurrency checklists, `git log -S` and attacker-model lines, the fp-check verification method.
4. New tools or dependencies the adaptations would bring, each needing approval: `proptest` (dev-dependency), `@axe-core/playwright` (dev-dependency), semgrep (a host tool).
