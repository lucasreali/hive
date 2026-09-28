import { expect, test } from "bun:test";
import type { ServiceMessage } from "../protocol";
import { createMockTransport, MOCK_REPOS } from "./mock";
import { MOCK_PULLS } from "./mockPulls";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const [shop, api] = MOCK_REPOS;
const login = shop.worktrees[1];

async function connected() {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  messages.length = 0;
  return { transport, messages };
}

test("shop lists its pull requests; api has no GitHub remote", async () => {
  const { transport, messages } = await connected();
  await transport.listPulls(shop.id, false);
  await transport.listPulls(api.id, true);
  await tick();
  const [listed, none] = messages;
  expect(listed).toMatchObject({ type: "pulls", project: shop.id, error: null });
  if (listed?.type !== "pulls") throw new Error("expected pulls");
  expect(listed.mine).toEqual(MOCK_PULLS[shop.id]?.mine ?? []);
  expect(listed.mine[0]?.worktree).toBe(login.id);
  expect(listed.repo?.merge_methods).toEqual(["merge", "squash"]);
  expect(none).toMatchObject({
    type: "pulls",
    repo: null,
    mine: [],
    error: "No remote of this repository is on github.com",
  });
});

test("details, actions and a refused merge", async () => {
  const { transport, messages } = await connected();
  await transport.openPull(shop.id, 12);
  await transport.openPull(shop.id, 99);
  await tick();
  expect(messages[0]).toMatchObject({ type: "pull", number: 12, error: null });
  if (messages[0]?.type !== "pull") throw new Error("expected pull");
  expect(messages[0].pull?.summary.title).toBe("Fix the login redirect");
  expect(messages[0].pull?.head).toHaveLength(40);
  expect(messages[1]).toEqual({
    type: "pull",
    project: shop.id,
    number: 99,
    pull: null,
    error: "no pull request #99",
  });
  messages.length = 0;
  for (const [action, state, message] of [
    [{ kind: "ready" }, "open", "#9 is ready for review"],
    [{ kind: "merge", method: "squash", head: "a".repeat(40) }, "merged", "Merged pull request #9"],
    [{ kind: "close" }, "closed", "Closed pull request #9"],
  ] as const) {
    await transport.actOnPull(shop.id, 9, action);
    await tick();
    expect(messages.slice(0, 1)).toEqual([
      { type: "pull_done", project: shop.id, number: 9, message },
    ]);
    expect(messages[1]).toMatchObject({ type: "pull", pull: { summary: { state } } });
    expect(messages[2]).toMatchObject({ type: "pulls", project: shop.id });
    messages.length = 0;
  }
  await transport.actOnPull(shop.id, 14, { kind: "merge", method: "merge", head: "b" });
  await tick();
  expect(messages).toHaveLength(1);
  expect(messages[0]).toMatchObject({ type: "pull_failed", number: 14 });
  // A checkout is a new worktree `pr-<number>`.
  messages.length = 0;
  await transport.actOnPull(shop.id, 14, { kind: "checkout" });
  await tick();
  expect(messages[0]).toMatchObject({
    type: "worktree_created",
    path: `${shop.id}/.claude/worktrees/pr-14`,
  });
});

test("creating one needs a title and a GitHub repository", async () => {
  const { transport, messages } = await connected();
  await transport.createPull(login.path, " ", "", "main", false);
  await transport.createPull(api.worktrees[1]?.path ?? "", "T", "", "main", false);
  await tick();
  expect(messages).toEqual([
    {
      type: "pull_failed",
      project: shop.id,
      number: null,
      message: "A title of 1 to 256 characters is needed",
    },
    {
      type: "pull_failed",
      project: api.id,
      number: null,
      message: "No remote of this repository is on github.com",
    },
  ]);
  messages.length = 0;
  await transport.createPull(login.path, " New ", "body", "main", true);
  await tick();
  expect(messages[0]).toEqual({
    type: "pull_done",
    project: shop.id,
    number: 23,
    message: "Opened pull request #23",
  });
  if (messages[1]?.type !== "pulls") throw new Error("expected pulls");
  expect(messages[1].mine[0]).toMatchObject({ number: 23, title: "New", state: "draft" });
  await transport.createPull(login.path, "Open", "", "main", false);
  await tick();
  expect(messages[3]).toMatchObject({ type: "pulls" });
  if (messages[3]?.type !== "pulls") throw new Error("expected pulls");
  expect(messages[3].mine[0]).toMatchObject({ number: 24, state: "open" });
});
