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
  api_key_source: null,
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

const open = (commands: string[] = []) =>
  act(() =>
    apply({
      type: "chat_opened",
      channel: 3,
      chat: 3,
      cwd: "/w",
      session: null,
      model: null,
      mode: "default",
      commands,
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

test("the mode selector shows the service's mode and asks it for another", () => {
  const setMode = spyOn(transport, "chatSetMode").mockResolvedValue();
  composer();
  const select = screen.getByRole("combobox", { name: "Permission mode" }) as HTMLButtonElement;
  expect([select.disabled, select.textContent]).toEqual([true, "Default"]);
  open();
  expect(select.disabled).toBe(false);
  fireEvent.mouseDown(select);
  // Never bypassPermissions.
  expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
    "Default",
    "Accept edits",
    "Plan",
  ]);
  fireEvent.click(screen.getByRole("option", { name: "Plan" }));
  expect(setMode.mock.calls).toEqual([[3, "plan"]]);
  // The selector follows the service, not the click.
  expect(select.textContent).toBe("Default");
  act(() => apply({ type: "chat_status", channel: 3, ...status(false), mode: "plan" }));
  expect(select.textContent).toBe("Plan");
});

test("Esc in the message stops a running turn, and only then", () => {
  const { stop, input } = composer();
  open();
  fireEvent.keyDown(input, { key: "Escape" });
  expect(stop).not.toHaveBeenCalled();
  act(() => apply({ type: "chat_status", channel: 3, ...status(true) }));
  fireEvent.keyDown(input, { key: "Escape", isComposing: true });
  expect(stop).not.toHaveBeenCalled();
  fireEvent.keyDown(input, { key: "Escape" });
  expect(stop.mock.calls).toEqual([[3]]);
  expect(screen.getByRole("button", { name: "Stop" }).title).toBe("Stop the turn (Esc)");
});

test("/ lists the commands that start with what follows it, picked by keyboard or click", () => {
  const { send, stop, input } = composer();
  open(["compact", "clear", "review"]);
  const list = () => screen.queryByRole("listbox", { name: "Commands" });
  const options = () => screen.getAllByRole("option").map((o) => o.textContent);
  const selected = () => screen.getByRole("option", { selected: true }).textContent;
  fireEvent.change(input, { target: { value: "/" } });
  expect(options()).toEqual(["/compact", "/clear", "/review"]);
  expect(input.getAttribute("aria-controls")).toBe(list()?.id ?? "");
  expect(input.getAttribute("aria-activedescendant")).toBe(
    screen.getByRole("option", { selected: true }).id,
  );
  fireEvent.change(input, { target: { value: "/c" } });
  expect(options()).toEqual(["/compact", "/clear"]);
  expect(selected()).toBe("/compact");
  // ↑/↓ wrap around; Enter picks without sending.
  fireEvent.keyDown(input, { key: "ArrowUp" });
  expect(selected()).toBe("/clear");
  fireEvent.keyDown(input, { key: "ArrowDown" });
  expect(selected()).toBe("/compact");
  fireEvent.keyDown(input, { key: "ArrowDown" });
  fireEvent.keyDown(input, { key: "Enter" });
  expect([input.value, list(), send.mock.calls]).toEqual(["/clear ", null, []]);
  // Then Enter sends as usual.
  fireEvent.keyDown(input, { key: "Enter" });
  expect(send.mock.calls).toEqual([[3, "/clear ", []]]);

  // Tab picks too; no match or a space hides the list.
  fireEvent.change(input, { target: { value: "/r" } });
  fireEvent.keyDown(input, { key: "Tab" });
  expect(input.value).toBe("/review ");
  fireEvent.change(input, { target: { value: "/x" } });
  expect(list()).toBeNull();
  fireEvent.change(input, { target: { value: "hi /c" } });
  expect(list()).toBeNull();

  // A click picks (the mouse moves the selection); a press on the list keeps the focus.
  fireEvent.change(input, { target: { value: "/" } });
  const review = screen.getByRole("option", { name: "/review" });
  fireEvent.mouseMove(review);
  expect(selected()).toBe("/review");
  const pressed = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
  review.dispatchEvent(pressed);
  expect(pressed.defaultPrevented).toBe(true);
  fireEvent.click(review);
  expect(input.value).toBe("/review ");

  // Esc hides the list (and stops nothing, even while busy) until the text changes.
  act(() => apply({ type: "chat_status", channel: 3, ...status(true) }));
  fireEvent.change(input, { target: { value: "/co" } });
  fireEvent.keyDown(input, { key: "Escape" });
  expect([list(), stop.mock.calls]).toEqual([null, []]);
  // Arrows and Enter are the textarea's again.
  fireEvent.keyDown(input, { key: "ArrowDown" });
  fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
  expect(input.value).toBe("/co");
  fireEvent.change(input, { target: { value: "/c" } });
  expect(options()).toEqual(["/compact", "/clear"]);
  // Shift+Enter is a new line, not a pick.
  fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
  expect(input.value).toBe("/c");
});
