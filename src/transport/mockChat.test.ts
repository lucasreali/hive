import { expect, test } from "bun:test";
import type { ChatAnswer, ChatEntry, ServiceMessage } from "../store";
import {
  createMockChat,
  describeAnswer,
  MOCK_CHAT_COMMANDS,
  MOCK_CHAT_MODEL,
  MOCK_CHAT_REQUESTS,
} from "./mockChat";

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
/** Long enough for a whole scripted turn at 1 ms a step. */
const settle = () => wait(80);

function mock(step = 1) {
  const messages: ServiceMessage[] = [];
  const chat = createMockChat((m) => messages.push(m), step);
  const take = () => messages.splice(0);
  /** Waits until the turn started (its busy status), then takes the messages. */
  const started = async () => {
    while (!messages.some((m) => m.type === "chat_status" && m.busy)) await wait(0);
    return take();
  };
  return { chat, take, started };
}

/** A mock with chat 1 open in `/w`, its messages taken. */
async function opened(step = 1) {
  const m = mock(step);
  m.chat.open(1, "/w", null, null);
  await settle();
  m.chat.confirm(1, "/w", true);
  await settle();
  m.take();
  return m;
}

const entriesOf = (messages: ServiceMessage[]): ChatEntry[] =>
  messages.flatMap((m) => (m.type === "chat_entries" ? m.entries : []));
const status = (
  busy: boolean,
  extra: object = {},
): Extract<ServiceMessage, { type: "chat_status" }> => ({
  type: "chat_status",
  channel: 1,
  chat: 1,
  busy,
  mode: "default",
  model: MOCK_CHAT_MODEL,
  retry: null,
  compacting: false,
  session: "mock-chat-1",
  ...extra,
});

test("the first chat in a folder asks for a confirmation, once it is accepted", async () => {
  const { chat, take } = mock();
  chat.open(1, "/w", null, "plan");
  expect(take()).toEqual([]);
  await settle();
  expect(take()).toEqual([{ type: "confirm_chat_folder", channel: 1, chat: 1, cwd: "/w" }]);
  // Answers for another chat or folder are ignored.
  chat.confirm(2, "/w", true);
  chat.confirm(1, "/x", true);
  chat.confirm(1, "/w", false);
  await settle();
  expect(take()).toEqual([{ type: "chat_closed", channel: 1, chat: 1, error: null }]);

  chat.open(2, "/w", null, null);
  await settle();
  expect(take()).toEqual([{ type: "confirm_chat_folder", channel: 2, chat: 2, cwd: "/w" }]);
  chat.confirm(2, "/w", true);
  await settle();
  expect(take()).toEqual([
    {
      type: "chat_opened",
      channel: 2,
      chat: 2,
      cwd: "/w",
      session: "mock-chat-2",
      model: MOCK_CHAT_MODEL,
      mode: "default",
      commands: MOCK_CHAT_COMMANDS,
      api_key_source: null,
    },
    { ...status(false), channel: 2, chat: 2, session: "mock-chat-2" },
  ]);

  chat.open(3, "/w", "s-1", "plan");
  await settle();
  const [open, , history] = take();
  expect(open).toMatchObject({ type: "chat_opened", chat: 3, session: "s-1", mode: "plan" });
  expect(history).toMatchObject({ type: "chat_entries", chat: 3, replace_last: false });
  expect(entriesOf([history as ServiceMessage]).map((e) => [e.id, e.kind])).toEqual([
    [1, "user"],
    [2, "assistant"],
  ]);
});

test("a turn plays the scripted entries, then ends idle with its usage", async () => {
  const { chat, take } = await opened();
  const image = { media_type: "image/png", data: "iVBO" };
  const other = { media_type: "image/gif", data: "R0lG" };
  chat.send(1, "Hello", [image, other]);
  expect(take()).toEqual([]);
  await settle();
  const messages = take();
  expect(messages[0]).toEqual(status(true));
  expect(messages.at(-1)).toEqual(status(false));
  const entries = entriesOf(messages);
  expect(entries.map((e) => [e.id, e.kind, e.text])).toEqual([
    [1, "user", "Hello"],
    [2, "user", ""],
    [3, "thinking", "Let me look at the worktree first."],
    [4, "assistant", "I'll list the files."],
    [5, "tool", "ls -la"],
    [5, "tool", "ls -la"],
    [6, "assistant", "The worktree has"],
    [6, "assistant", "The worktree has a README.md and a src folder."],
    [7, "usage", "2.3 s · 40 output tokens · 12% context"],
  ]);
  expect(entries[0]?.image).toEqual(image);
  expect(entries[1]?.image).toEqual(other);
  expect(entries.slice(4, 6).map((e) => [e.tool, e.status, e.output])).toEqual([
    ["Bash", "running", null],
    ["Bash", "ok", "README.md\nsrc\n"],
  ]);
  const replaced = messages.filter((m) => m.type === "chat_entries" && m.replace_last);
  expect(entriesOf(replaced).map((e) => e.id)).toEqual([6]);
  // Unknown chats are ignored.
  chat.send(9, "x", []);
  chat.setMode(9, "plan");
  chat.interrupt(9);
  chat.answer(9, "r", { kind: "allow" });
  chat.close(9);
  await settle();
  expect(take()).toEqual([]);
});

test("words in the turn add a subagent, a compaction and an error", async () => {
  const { chat, take } = await opened();
  chat.send(1, "subagent compact error", []);
  await settle();
  const messages = take();
  const entries = entriesOf(messages);
  const agent = entries.find((e) => e.tool === "Agent");
  expect(agent?.status).toBe("running");
  const parent = `toolu_mock_${agent?.id}`;
  expect(entries.filter((e) => e.parent === parent).map((e) => [e.kind, e.status])).toEqual([
    ["user", null],
    ["assistant", null],
    ["tool", "running"],
    ["tool", "ok"],
  ]);
  expect(entries.filter((e) => e.id === agent?.id).at(-1)?.output).toBe("notes.txt has 2 lines.");
  expect(entries.find((e) => e.kind === "divider")?.text).toBe(
    "Conversation compacted (150k tokens)",
  );
  expect(entries.find((e) => e.kind === "error")?.text).toBe("API Error: 529 overloaded");
  expect(messages).toContainEqual(status(true, { compacting: true }));
  expect(messages).toContainEqual(status(true, { retry: "Retrying 2/10…" }));
  expect(messages.at(-1)).toEqual(status(false));
  // Ids increase in the order entries first appear.
  const firsts = entries.map((e) => e.id).filter((id, i, all) => all.indexOf(id) === i);
  expect(firsts).toEqual([...firsts].sort((a, b) => a - b));
});

test("a crash closes the chat with an error", async () => {
  const { chat, take } = await opened();
  chat.send(1, "crash", []);
  await settle();
  expect(take().at(-1)).toEqual({
    type: "chat_closed",
    channel: 1,
    chat: 1,
    error: "mock: claude exited with code 1",
  });
  chat.send(1, "again", []);
  await settle();
  expect(take()).toEqual([]);
});

test("each request waits for its answer, which is acknowledged and replied to", async () => {
  const { chat, take } = await opened();
  const cases: [keyof typeof MOCK_CHAT_REQUESTS, ChatAnswer][] = [
    ["permission", { kind: "allow" }],
    ["permission", { kind: "deny", message: "not now" }],
    ["question", { kind: "answers", answers: [["English"], ["README.md", "CONTRIBUTING.md"]] }],
    ["plan", { kind: "keep_planning", feedback: "shorter" }],
    ["plan", { kind: "approve_plan", accept_edits: false }],
  ];
  let n = 0;
  for (const [word, answer] of cases) {
    chat.send(1, `please ${word}`, []);
    await settle();
    const messages = take();
    const id = `req_mock_${++n}`;
    expect(messages.at(-1)).toEqual({
      type: "chat_request",
      channel: 1,
      chat: 1,
      request: { id, ...MOCK_CHAT_REQUESTS[word] },
    });
    chat.answer(1, "req_other", { kind: "allow" });
    await settle();
    expect(take()).toEqual([{ type: "error", message: "no pending request req_other" }]);
    chat.answer(1, id, answer);
    await settle();
    const reply = take();
    expect(reply[0]).toEqual({ type: "chat_request_gone", channel: 1, chat: 1, request: id });
    expect(entriesOf(reply).map((e) => [e.kind, e.text])).toEqual([
      ["assistant", describeAnswer(answer)],
      ["usage", "2.3 s · 40 output tokens · 12% context"],
    ]);
    expect(reply.at(-1)).toEqual(status(false));
    // Answered once only.
    chat.answer(1, id, answer);
    await settle();
    expect(take()).toEqual([{ type: "error", message: `no pending request ${id}` }]);
  }
  chat.send(1, "plan", []);
  await settle();
  take();
  chat.answer(1, `req_mock_${++n}`, { kind: "approve_plan", accept_edits: true });
  await settle();
  expect(take().at(-1)).toEqual(status(false, { mode: "accept_edits" }));
});

test("describes every answer", () => {
  expect(describeAnswer({ kind: "allow" })).toBe("Allowed, so I went ahead.");
  expect(describeAnswer({ kind: "deny", message: null })).toBe("Denied. I stopped.");
  expect(describeAnswer({ kind: "deny", message: "no" })).toBe("Denied: no. I stopped.");
  expect(describeAnswer({ kind: "answers", answers: [["a", "b"], ["c"]] })).toBe(
    "You chose: a, b / c.",
  );
  expect(describeAnswer({ kind: "approve_plan", accept_edits: true })).toBe(
    "Plan approved, accepting edits. Done.",
  );
  expect(describeAnswer({ kind: "approve_plan", accept_edits: false })).toBe(
    "Plan approved. Done.",
  );
  expect(describeAnswer({ kind: "keep_planning", feedback: "f" })).toBe("Planning again: f");
});

test("an interrupt stops the turn and drops its pending request", async () => {
  const { chat, take } = await opened();
  // Not busy: nothing to stop.
  chat.interrupt(1);
  await settle();
  expect(take()).toEqual([]);

  chat.send(1, "question", []);
  await settle();
  take();
  chat.interrupt(1);
  await settle();
  const stopped = take();
  expect(stopped[0]).toEqual({
    type: "chat_request_gone",
    channel: 1,
    chat: 1,
    request: "req_mock_1",
  });
  expect(entriesOf(stopped).map((e) => [e.kind, e.text])).toEqual([["note", "Interrupted"]]);
  expect(stopped.at(-1)).toEqual(status(false));
  chat.answer(1, "req_mock_1", { kind: "allow" });
  await settle();
  expect(take()).toEqual([{ type: "error", message: "no pending request req_mock_1" }]);
});

test("an interrupt in the middle of a turn drops the rest of its script", async () => {
  const { chat, take, started } = await opened(40);
  chat.send(1, "long", []);
  await started();
  chat.interrupt(1);
  await wait(500);
  const messages = take();
  expect(messages.at(-1)).toEqual(status(false));
  expect(entriesOf(messages).some((e) => e.kind === "usage")).toBe(false);
});

test("the mode can change, and closing ends the chat and its script", async () => {
  const { chat, take, started } = await opened(40);
  chat.setMode(1, "plan");
  await settle();
  expect(take()).toEqual([status(false, { mode: "plan" })]);

  chat.send(1, "hi", []);
  await started();
  chat.close(1);
  await wait(500);
  const messages = take();
  expect(messages.at(-1)).toEqual({ type: "chat_closed", channel: 1, chat: 1, error: null });
  expect(entriesOf(messages).some((e) => e.kind === "usage")).toBe(false);
  chat.close(1);
  await settle();
  expect(take()).toEqual([]);

  // A chat still waiting for its folder's confirmation closes too.
  chat.open(2, "/other", null, null);
  chat.close(2);
  await settle();
  expect(take()).toEqual([
    { type: "confirm_chat_folder", channel: 2, chat: 2, cwd: "/other" },
    { type: "chat_closed", channel: 2, chat: 2, error: null },
  ]);
});
