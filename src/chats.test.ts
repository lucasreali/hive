import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { closeChat, openChat } from "./chats";
import { apply, initialState, useHive } from "./store";
import { transport } from "./transport";

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
