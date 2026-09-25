---
name: stage
description: Develop a whole stage of TODO.md by orchestrating parallel task agents. Use when the human asks to develop, implement or run a stage ("faça a etapa 2", "develop stage 3", "/stage 2").
---

# Orchestrate a TODO.md stage

You are the **orchestrator**. You do not write product code yourself: you plan, launch task agents, integrate their reports and talk to the human. Keep your own context small; agents do the reading and the work.

## 1. Plan (once, at the start)
1. Read `CLAUDE.md`, the stage in `TODO.md` (every unchecked task up to the stage's ⏸ checkpoint), and skim the decisions it cites in `docs/hive.md`. Check `git status` / `git worktree list`: the main checkout must be on a clean `main`.
2. Build the dependency graph between the stage's tasks from what each task needs (code, service messages, UI pieces), not from list order. Tasks that do not depend on each other run **in parallel**; never serialize independent tasks.
3. Find the blockers that only the human can clear and ask them **up front, together**, one question each with a recommendation (AskUserQuestion): tasks marked "⚠ Ask the human first", missing inputs (files, recordings), decisions the stage needs that `docs/hive.md` does not settle. For small implementation choices, pick a sensible default yourself and put it on the checkpoint list (step 4) instead of asking.
4. Tell the human in one short message: the waves (which tasks run in parallel), the defaults you chose, and the dependencies agents are expected to add (CLAUDE.md rule 3 — name each one and why). Then start without waiting, unless something above is a real blocker.

## 2. Run
- Launch every ready task at once: `Agent` with `subagent_type: general-purpose`, `isolation: worktree`, `run_in_background: true`. The prompt is: "Read and follow `.claude/skills/stage/task-brief.md` first", the task id and name, the decisions it cites, **what already exists on main that it must reuse** (APIs, files — taken from earlier reports), the orchestrator defaults that apply, and the specific scope/tests you expect. Tasks the human does (recordings, reviews) are not delegated.
- When an agent reports a finished task with **green CI on its branch tip** (run URLs in the report), **integrate it** from the main checkout:
  `git merge-base --is-ancestor main <branch> && flock /var/tmp/hive-merge.lock git merge --ff-only <branch>`.
  If `main` moved (not an ancestor), send the agent back with `SendMessage` to merge `main`, resolve and re-run the gates. After the merge, delete the branch once the agent's worktree is gone (`git worktree remove` / `git branch -d`; a worktree stays locked while its agent process lives). Relay a short summary to the human (what landed, gates, items for the checkpoint). Launch every task that just became ready.
- If an agent stops with a question, answer it yourself when a decision or default covers it (continue the agent with `SendMessage`); otherwise ask the human.
- If an agent dies (session restart), check its worktree/branch for partial work and resume it with `SendMessage`.
- Never read an agent's transcript file; its report is all you need.
- Keep a running "for the checkpoint" list: defaults taken, deviations from the prototype, human decisions requested, flaky tests, dependencies added outside the announced list. Save progress to memory after each merge, so a restarted session can continue.

## 3. Finish
When every task of the stage is merged (or blocked on the human): run the full gates once on `main` (`scripts/gates.sh` with `MUTANTS=0`, frontend gates, e2e), then stop at the ⏸ checkpoint with a report: what landed (task → merge commit), gate results, the checkpoint list grouped for decision, and what the human must do (e.g. update `docs/hive.md`, which agents never edit). Do not start the next stage.
