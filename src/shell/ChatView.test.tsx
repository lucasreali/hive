import { afterEach, beforeAll, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { addTab, apply, initialState, type ServiceMessage, setChat, useHive } from "../store";
import { transport } from "../transport";
import { TerminalArea } from "./TerminalArea";

beforeAll(() => {
  // happy-dom has no layout: give the list its CSS size so the virtualizer shows entries.
  const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight")?.get;
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("transcript") ? 800 : original?.call(this);
    },
  });
});

afterEach(() => {
  mock.restore();
  cleanup();
  useHive.setState(initialState, true);
});

const opened: ServiceMessage = {
  type: "chat_opened",
  channel: 5,
  chat: 5,
  cwd: "/w",
  session: null,
  model: null,
  mode: "default",
  commands: [],
  api_key_source: null,
};

function chatTab() {
  render(<TerminalArea />);
  act(() => {
    setChat(5, "/w");
    addTab(5, "/w", "chat");
  });
  return screen.getByRole("region", { name: "Chat" });
}

const state = () => document.querySelector(".chat-view .state-label")?.textContent;

test("a chat tab shows the chat in place of the terminals, from start to end", () => {
  const view = chatTab();
  expect((document.querySelector(".terminal-host") as HTMLElement).hidden).toBe(true);
  expect(view.querySelector(".path")?.textContent).toBe("chat: /wstarting");
  expect(screen.getByText("Starting Claude…")).toBeDefined();
  act(() => apply(opened));
  expect(state()).toBe("ready");
  expect(screen.getByText("Send a message to start.")).toBeDefined();
  act(() =>
    apply({
      type: "chat_status",
      channel: 5,
      chat: 5,
      busy: true,
      mode: "default",
      model: null,
      retry: null,
      compacting: false,
      session: null,
    }),
  );
  expect(state()).toBe("working");
  const hi = { id: 1, kind: "user", text: "hi", tool: null, parent: null } as const;
  const entry = { ...hi, status: null, output: null, image: null };
  act(() =>
    apply({ type: "chat_entries", channel: 5, chat: 5, entries: [entry], replace_last: false }),
  );
  expect(view.querySelector(".transcript-entry")?.textContent).toBe("Youhi");
  act(() =>
    apply({ type: "chat_closed", channel: 5, chat: 5, error: "claude exited with code 1" }),
  );
  expect(state()).toBe("ended");
  expect(screen.getByRole("alert").textContent).toBe("claude exited with code 1");
  // The tab says so; closing an ended chat does not ask the service again.
  const close = spyOn(transport, "closeChat").mockResolvedValue();
  const tab = screen.getByRole("tab");
  expect(tab.textContent).toBe("/wended");
  fireEvent.contextMenu(tab);
  expect(screen.queryByRole("menu")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Close chat /w" }));
  expect(close).not.toHaveBeenCalled();
  expect(screen.queryByRole("region", { name: "Chat" })).toBeNull();
});

test("the first chat in a folder asks first; Start chat accepts, Esc refuses", () => {
  const confirm = spyOn(transport, "confirmChatFolder").mockResolvedValue();
  chatTab();
  act(() => apply({ type: "confirm_chat_folder", channel: 5, chat: 5, cwd: "/w" }));
  const dialog = screen.getByRole("dialog", { name: "Chat in this folder?" }) as HTMLDialogElement;
  expect(dialog.open).toBe(true);
  expect(document.activeElement?.textContent).toBe("Start chat Enter");
  fireEvent.click(screen.getByRole("button", { name: "Start chat Enter" }));
  expect(confirm.mock.calls).toEqual([[5, "/w", true]]);
  expect(screen.queryByRole("dialog")).toBeNull();
  act(() => apply(opened));

  // Another chat's refusal: by Esc (the dialog's close), the header button or Cancel.
  for (const refuse of [
    (d: HTMLElement) => fireEvent(d, new Event("close")),
    () => fireEvent.click(screen.getByTitle("Cancel (Esc)")),
    () => fireEvent.click(screen.getByRole("button", { name: "Cancel Esc" })),
  ]) {
    cleanup();
    useHive.setState(initialState, true);
    chatTab();
    act(() => apply({ type: "confirm_chat_folder", channel: 5, chat: 5, cwd: "/w" }));
    confirm.mockClear();
    refuse(screen.getByRole("dialog"));
    expect(confirm.mock.calls).toEqual([[5, "/w", false]]);
    expect(screen.queryByRole("dialog")).toBeNull();
  }
});
