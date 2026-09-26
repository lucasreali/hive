import { afterEach, beforeAll, beforeEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import {
  activateTab,
  addTab,
  apply,
  initialState,
  type ServiceMessage,
  setChat,
  useHive,
} from "../store";
import { transport } from "../transport";
import { MOCK_CHAT_REQUESTS } from "../transport/mockChat";
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

beforeEach(() => useHive.setState(initialState, true));
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
      api_key_source: null,
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
  // It never named a session: there is nothing to resume.
  expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
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

test("an ended chat with a session offers to resume it in its place", async () => {
  const open = spyOn(transport, "openChat").mockResolvedValue(6);
  chatTab();
  act(() => apply({ ...opened, session: "s-1" } as ServiceMessage));
  expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
  act(() => apply({ type: "chat_closed", channel: 5, chat: 5, error: null }));
  expect(screen.getByText("The chat ended.")).toBeDefined();
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Resume" })));
  expect(open.mock.calls).toEqual([["/w", "s-1", "default"]]);
  expect(useHive.getState().tabs).toEqual([{ id: 6, cwd: "/w", kind: "chat" }]);
});

test("pending requests pin above the composer until the service says they are gone", () => {
  const view = chatTab();
  act(() => apply(opened));
  expect(view.querySelector(".chat-requests")).toBeNull();
  const request = { id: "req_1", ...MOCK_CHAT_REQUESTS.permission };
  act(() => apply({ type: "chat_request", channel: 5, chat: 5, request }));
  const pinned = view.querySelector(".chat-requests") as HTMLElement;
  expect(pinned.nextElementSibling?.className).toBe("chat-composer");
  expect(screen.getByRole("region", { name: "Permission request" })).toBeDefined();
  act(() => apply({ type: "chat_request_gone", channel: 5, chat: 5, request: "req_1" }));
  expect(screen.queryByRole("region", { name: "Permission request" })).toBeNull();
});

test("the header shows the model and mode, what else runs, and warns of an API key", () => {
  const view = chatTab();
  const meta = () => view.querySelector(".chat-meta")?.textContent;
  expect(meta()).toBe("");
  act(() => apply(opened));
  expect(meta()).toBe("Default");
  const status = {
    type: "chat_status",
    channel: 5,
    chat: 5,
    busy: true,
    mode: "plan",
    model: "claude-haiku-4-5",
    retry: null,
    compacting: false,
    session: null,
    api_key_source: null,
  } as const;
  act(() => apply(status));
  expect(meta()).toBe("claude-haiku-4-5 · Plan");
  expect(screen.queryByRole("status")).toBeNull();
  expect(screen.queryByRole("note")).toBeNull();
  act(() => apply({ ...status, retry: "Retrying 2/10…" }));
  expect(screen.getByRole("status").textContent).toBe("Retrying 2/10…");
  act(() => apply({ ...status, retry: "Retrying 2/10…", compacting: true }));
  expect(screen.getByRole("status").textContent).toBe("Compacting…");
  act(() => apply({ ...status, api_key_source: "ANTHROPIC_API_KEY" }));
  const warning = screen.getByRole("note");
  expect(warning.textContent).toBe(" API key");
  expect(warning.title).toBe(
    "This chat runs on an API key (ANTHROPIC_API_KEY), not your subscription login: it may be billed separately.",
  );
  expect(meta()).toBe(" API keyclaude-haiku-4-5 · Plan");
});

test("switching to another chat and back keeps each one's draft and scroll position", () => {
  chatTab();
  act(() => apply(opened));
  const input = () => screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement;
  fireEvent.change(input(), { target: { value: "half a thought" } });
  const transcript = document.querySelector(".transcript") as HTMLElement;
  Object.defineProperty(transcript, "scrollHeight", { value: 6000 });
  transcript.scrollTop = 1000;
  fireEvent.scroll(transcript);

  act(() => {
    setChat(6, "/w");
    addTab(6, "/w", "chat");
  });
  expect(input().value).toBe("");

  const scrollTo = mock((_: ScrollToOptions) => {});
  const original = HTMLElement.prototype.scrollTo;
  HTMLElement.prototype.scrollTo = scrollTo as unknown as typeof original;
  try {
    act(() => activateTab({ id: 5, cwd: "/w", kind: "chat" }));
    expect(input().value).toBe("half a thought");
    expect(scrollTo.mock.calls[0]?.[0].top).toBe(1000);
  } finally {
    HTMLElement.prototype.scrollTo = original;
  }
});
