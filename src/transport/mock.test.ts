import { expect, test } from "bun:test";
import type { ServiceMessage } from "../store";
import { createMockTransport, MOCK_REPOS } from "./mock";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

async function connected() {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  return { transport, messages };
}

async function opened() {
  const { transport, messages } = await connected();
  let output = "";
  const decoder = new TextDecoder();
  const id = await transport.openTerminal("/w", 80, 24, (b) => {
    output += decoder.decode(b);
  });
  await tick();
  return { transport, messages, id, output: () => output };
}

test("welcomes the UI asynchronously", async () => {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  expect(messages).toEqual([]);
  await tick();
  expect(messages).toEqual([
    { type: "welcome", version: "mock", distro: "Ubuntu" },
    { type: "projects", projects: MOCK_REPOS.slice(0, 2) },
  ]);
});

test("a scenario fails the connection instead", async () => {
  for (const [scenario, types] of [
    ["mismatch", ["version_mismatch"]],
    ["disconnected", ["disconnected"]],
    ["", ["welcome", "projects"]],
  ] as const) {
    const messages: ServiceMessage[] = [];
    await createMockTransport(scenario).connect((m) => messages.push(m));
    await tick();
    expect(messages.map((m) => m.type)).toEqual([...types]);
  }
});

test("projects are added from the fake repositories only", async () => {
  const transport = createMockTransport("empty");
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  expect(messages.at(-1)).toEqual({ type: "projects", projects: [] });

  const [shop] = MOCK_REPOS;
  await transport.addProject(shop.path);
  await transport.addProject(shop.path);
  await transport.addProject("/nope");
  await transport.listProjects();
  await tick();
  expect(messages.slice(2)).toEqual([
    { type: "project_added", project: shop },
    { type: "project_added", project: shop },
    {
      type: "add_project_failed",
      path: "/nope",
      error: "not_found",
      message: "cannot open /nope: No such file or directory (os error 2)",
    },
    { type: "projects", projects: [shop] },
  ]);
  expect(shop.worktrees.map((w) => [w.name, w.branch, w.main, w.claude])).toEqual([
    ["main", "main", true, false],
    ["fix-login", "worktree-fix-login", false, true],
    ["feat-checkout", "worktree-feat-checkout", false, true],
  ]);
});

test("a terminal prints a prompt, echoes input and repeats the line", async () => {
  const { transport, messages, id, output } = await opened();
  expect(id).toBe(1);
  expect(messages.at(-1)).toEqual({ type: "terminal_opened", channel: 1 });
  expect(output()).toBe("mock$ ");
  await transport.writeTerminal(id, "hi\r\r");
  expect(output()).toBe("mock$ hi\r\nhi\r\nmock$ \r\nmock$ ");
  await transport.resizeTerminal(id, 100, 30);
  expect(await transport.openTerminal("/", 1, 1, () => {})).toBe(2);
});

test("exit ends the terminal with code 0 and close with no code", async () => {
  const { transport, messages, id, output } = await opened();
  await transport.writeTerminal(id, "exit\rignored");
  await tick();
  expect(messages.at(-1)).toEqual({ type: "terminal_exited", channel: id, code: 0 });
  expect(output()).toBe("mock$ exit");

  const second = await transport.openTerminal("/", 1, 1, () => {});
  await transport.closeTerminal(second);
  await transport.closeTerminal(second);
  await transport.writeTerminal(second, "x");
  await tick();
  expect(messages.filter((m) => m.type === "terminal_exited")).toEqual([
    { type: "terminal_exited", channel: id, code: 0 },
    { type: "terminal_exited", channel: second, code: null },
  ]);
});
