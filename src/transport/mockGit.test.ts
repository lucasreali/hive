import { expect, test } from "bun:test";
import type { ServiceMessage } from "../protocol";
import { createMockTransport, MOCK_REPOS } from "./mock";
import { MOCK_WORKFLOWS } from "./mockGit";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const [shop, api] = MOCK_REPOS;
const [main, login] = shop.worktrees;
const refactor = api.worktrees[1];

async function connected() {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  messages.length = 0;
  return { transport, messages };
}

/** Each changed file's staged state in the last `changes` of `path`. */
const staged = (messages: ServiceMessage[], path: string) => {
  const last = [...messages].reverse().find((m) => m.type === "changes" && m.path === path);
  if (last?.type !== "changes") throw new Error("expected changes");
  return last.files.map((f) => [f.path, f.staged]);
};

test("files are staged, committed and the changes sent again, as the watch does", async () => {
  const { transport, messages } = await connected();
  await transport.listChanges(login.path, "branch");
  await transport.stageFiles(login.path, ["src/auth/session.ts"], true);
  await tick();
  expect(messages[1]).toEqual({
    type: "git_done",
    worktree: login.path,
    action: "stage",
    message: "Staged src/auth/session.ts",
  });
  // Against the base the panel asked for.
  expect(messages[2]).toMatchObject({ type: "changes", base: "branch" });
  expect(staged(messages, login.path)).toEqual([
    ["src/auth/constants.ts", "no"],
    ["src/auth/session.ts", "yes"],
  ]);
  await transport.stageFiles(login.path, ["src/auth/constants.ts", "x"], false);
  await tick();
  expect(messages.at(-2)).toMatchObject({ action: "unstage", message: "Unstaged 2 files" });
  await transport.commit(login.path, "  ");
  await transport.commit(login.path, "Fix\n\nbody");
  await tick();
  const failed = messages.filter((m) => m.type === "git_failed").map((m) => m.message);
  expect(failed).toEqual(["A commit message is needed"]);
  expect(messages.at(-2)).toEqual({
    type: "git_done",
    worktree: login.path,
    action: "commit",
    message: "Committed 4f2a9c1: Fix",
  });
  expect(staged(messages, login.path)).toEqual([["src/auth/constants.ts", "no"]]);
  // Nothing staged: git's own refusal; a partly staged file stays, unstaged.
  messages.length = 0;
  await transport.commit(login.path, "again");
  await tick();
  expect(messages[0]).toMatchObject({
    type: "git_failed",
    message: 'no changes added to commit (use "git add" and/or "git commit -a")',
  });
});

test("a partly staged file stays after a commit, unstaged; refusals outside a worktree", async () => {
  const { transport, messages } = await connected();
  // `refactor-auth` has a staged rename; mark another file partly staged as git would.
  await transport.listChanges(refactor.path, "head");
  await tick();
  const first = messages[0];
  if (first?.type !== "changes") throw new Error("expected changes");
  expect(first.files.find((f) => f.status === "renamed")?.staged).toBe("yes");
  const file = first.files.find((f) => f.path === "package.json");
  if (file) file.staged = "partly";
  messages.length = 0;
  await transport.commit(refactor.path, "Rename");
  await tick();
  expect(staged(messages, refactor.path)).toEqual([
    ["assets/logo.png", "no"],
    ["package.json", "no"],
    ["src/legacy/jwt.ts", "no"],
    ["src/middleware/auth.ts", "no"],
  ]);
  messages.length = 0;
  await transport.stageFiles("/nope", ["a"], true);
  await transport.commit("/nope", "m");
  await transport.push("/nope", "b", "origin/b");
  await transport.getPushTarget("/nope");
  await tick();
  const why = "/nope is not a worktree of a followed project";
  expect(messages.map((m) => ("message" in m ? m.message : "error" in m ? m.error : null))).toEqual(
    [why, why, why, why],
  );
});

test("a push goes to its upstream or origin; a moved remote branch is refused", async () => {
  const { transport, messages } = await connected();
  await transport.getPushTarget(main.path);
  await transport.getPushTarget(login.path);
  await transport.push(login.path, "b", "origin/b");
  await transport.push(refactor.path, "b", "origin/b");
  await tick();
  expect(messages[0]).toEqual({
    type: "push_target",
    worktree: main.path,
    branch: "main",
    target: "origin/main",
    set_upstream: false,
    ahead: 1,
    error: null,
  });
  expect(messages[1]).toMatchObject({ set_upstream: true, ahead: null, error: null });
  expect(messages[2]).toMatchObject({
    type: "git_done",
    action: "push",
    message: `Pushed ${login.branch} to origin/${login.branch}`,
  });
  expect(messages[3]).toMatchObject({ type: "git_failed", action: "push" });
  if (messages[3]?.type !== "git_failed") throw new Error("expected the refusal");
  expect(messages[3].message).toContain("! [rejected]");
});

test("a detached worktree has nothing to push", async () => {
  // A worktree on no branch, as the service lists one.
  const detached = { ...login, id: `${shop.id}/.claude/worktrees/x`, branch: null };
  detached.path = detached.id;
  const repos = [{ ...shop, worktrees: [...shop.worktrees, detached] }, api];
  const transport = createMockTransport(null, null, { repos });
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  messages.length = 0;
  await transport.getPushTarget(detached.path);
  await transport.push(detached.path, "b", "origin/b");
  await tick();
  const none = "This worktree is on no branch (a detached HEAD): nothing to push";
  expect(messages).toEqual([
    {
      type: "push_target",
      worktree: detached.path,
      branch: null,
      target: null,
      set_upstream: false,
      ahead: null,
      error: none,
    },
    { type: "git_failed", worktree: detached.path, action: "push", message: none },
  ]);
});

test("shop's workflows run by hand; a started one is followed to its run", async () => {
  const { transport, messages } = await connected();
  await transport.listWorkflows(shop.id, login.path);
  await transport.listWorkflows(api.id, api.worktrees[0]?.path ?? "");
  await tick();
  expect(messages[0]).toEqual({
    type: "workflows",
    project: shop.id,
    worktree: login.path,
    workflows: MOCK_WORKFLOWS,
    error: null,
  });
  expect(messages[1]).toMatchObject({ type: "workflows", workflows: [] });
  messages.length = 0;
  await transport.runWorkflow(shop.id, "release.yml", "main", [{ name: "level", value: "minor" }]);
  await tick();
  const [started, run, runs] = messages;
  expect(started).toMatchObject({ type: "workflow_started", branch: "main", run: 303 });
  expect(run).toMatchObject({ type: "run", run: 303 });
  if (runs?.type !== "runs") throw new Error("expected runs");
  expect(runs.runs[0]).toMatchObject({ id: 303, workflow: "Release", state: "running" });
  messages.length = 0;
  await transport.runWorkflow(shop.id, "nope.yml", "main", []);
  await tick();
  expect(messages).toEqual([
    {
      type: "workflow_failed",
      project: shop.id,
      message: "gh workflow run failed: HTTP 404: could not find workflow nope.yml",
    },
  ]);
});
