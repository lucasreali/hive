import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
import { chatSession, closeChat, openChat, resumeChat } from "./chats";
import { apply, initialState, useHive } from "./store";
import { transport } from "./transport";

// Other files' tests may leave tabs behind: start from a clean store.
beforeEach(() => useHive.setState(initialState, true));
afterEach(() => {
  mock.restore();
  useHive.setState(initialState, true);
});

test("a new chat opens a shown chat tab in its worktree; closing ends it and drops its data", async () => {
  const open = spyOn(transport, "openChat").mockResolvedValue(4);
  const close = spyOn(transport, "closeChat").mockResolvedValue();
  expect(await openChat("/w")).toBe(4);
  expect(open.mock.calls).toEqual([["/w", null, null]]);
  const s = useHive.getState();
  expect([s.tabs, s.activeTab, s.selection, s.chats[4]?.cwd]).toEqual([
    [{ id: 4, cwd: "/w", kind: "chat" }],
    4,
    "/w",
    "/w",
  ]);
  closeChat(4);
  expect(close.mock.calls).toEqual([[4]]);
  expect([useHive.getState().tabs, useHive.getState().chats]).toEqual([[], {}]);

  // An ended chat is not asked to close again.
  await openChat("/w");
  apply({ type: "chat_closed", channel: 4, chat: 4, error: null });
  closeChat(4);
  expect(close.mock.calls).toHaveLength(1);
});

test("a chat the service refused adds no tab", async () => {
  spyOn(transport, "openChat").mockRejectedValue(new Error("no"));
  await expect(openChat("/w")).rejects.toThrow("no");
  expect(useHive.getState().tabs).toEqual([]);
});

test("an ended chat resumes its session in a new chat that replaces its tab", async () => {
  const open = spyOn(transport, "openChat").mockResolvedValueOnce(4).mockResolvedValueOnce(5);
  expect(await openChat("/w", "s-1", "plan")).toBe(4);
  expect(open.mock.calls).toEqual([["/w", "s-1", "plan"]]);
  // Nothing to resume while the service named no session.
  expect(chatSession(useHive.getState().chats[4])).toBeNull();
  await resumeChat(4);
  expect(open).toHaveBeenCalledTimes(1);
  apply({
    type: "chat_opened",
    channel: 4,
    chat: 4,
    cwd: "/w",
    session: "s-1",
    model: null,
    mode: "plan",
    commands: [],
    api_key_source: null,
  });
  apply({ type: "chat_closed", channel: 4, chat: 4, error: null });
  await resumeChat(4);
  expect(open.mock.calls[1]).toEqual(["/w", "s-1", "plan"]);
  expect(useHive.getState().tabs).toEqual([{ id: 5, cwd: "/w", kind: "chat" }]);
  expect(useHive.getState().chats[4]).toBeUndefined();

  // The status's session and mode are the latest.
  apply({
    type: "chat_status",
    channel: 5,
    chat: 5,
    busy: false,
    mode: "accept_edits",
    model: null,
    retry: null,
    compacting: false,
    session: "s-2",
  });
  apply({ type: "chat_closed", channel: 5, chat: 5, error: "gone" });
  open.mockRejectedValueOnce("refused");
  await resumeChat(5);
  expect(open.mock.calls[2]).toEqual(["/w", "s-2", "accept_edits"]);
  expect(useHive.getState().notice).toBe("Cannot resume the chat in /w: refused");
  expect(useHive.getState().tabs).toEqual([{ id: 5, cwd: "/w", kind: "chat" }]);
  // No such chat: nothing.
  await resumeChat(99);
  expect(open).toHaveBeenCalledTimes(3);
});
