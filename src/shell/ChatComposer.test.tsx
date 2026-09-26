import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { apply, type ChatStatus, initialState, setChat, useHive } from "../store";
import { transport } from "../transport";
import { ChatComposer } from "./ChatComposer";

beforeEach(() => useHive.setState(initialState, true));
afterEach(() => {
  mock.restore();
  cleanup();
  useHive.setState(initialState, true);
});

const status = (busy: boolean): ChatStatus => ({
  chat: 3,
  busy,
  mode: "default",
  model: null,
  retry: null,
  compacting: false,
  session: null,
});

function composer() {
  const send = spyOn(transport, "chatSend").mockResolvedValue();
  const stop = spyOn(transport, "chatInterrupt").mockResolvedValue();
  setChat(3, "/w");
  render(<ChatComposer chat={3} />);
  const input = screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement;
  return { send, stop, input };
}

const open = () =>
  act(() =>
    apply({
      type: "chat_opened",
      channel: 3,
      chat: 3,
      cwd: "/w",
      session: null,
      model: null,
      mode: "default",
      commands: [],
      api_key_source: null,
    }),
  );

test("the composer waits for the chat to start, and is disabled once it ended", () => {
  const { input } = composer();
  expect(input.disabled).toBe(true);
  open();
  expect(input.disabled).toBe(false);
  act(() => apply({ type: "chat_closed", channel: 3, chat: 3, error: null }));
  expect(input.disabled).toBe(true);
  expect((screen.getByRole("button", { name: "Send" }) as HTMLButtonElement).disabled).toBe(true);
});

test("Enter sends the text, Shift+Enter and an empty text do not", () => {
  const { send, input } = composer();
  open();
  fireEvent.keyDown(input, { key: "Enter" });
  expect(send).not.toHaveBeenCalled();
  fireEvent.change(input, { target: { value: "hello\nthere" } });
  fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
  fireEvent.keyDown(input, { key: "a" });
  expect(send).not.toHaveBeenCalled();
  fireEvent.keyDown(input, { key: "Enter" });
  expect(send.mock.calls).toEqual([[3, "hello\nthere", []]]);
  expect(input.value).toBe("");
  // The Send button does the same.
  fireEvent.change(input, { target: { value: "again" } });
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  expect(send.mock.calls.at(-1)).toEqual([3, "again", []]);
});

test("while a turn runs Send turns into Stop, and Enter keeps the draft", () => {
  const { send, stop, input } = composer();
  open();
  act(() => apply({ type: "chat_status", channel: 3, ...status(true) }));
  fireEvent.change(input, { target: { value: "next" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect([send.mock.calls, input.value]).toEqual([[], "next"]);
  expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Stop" }));
  expect(stop.mock.calls).toEqual([[3]]);
  act(() => apply({ type: "chat_status", channel: 3, ...status(false) }));
  expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
});
