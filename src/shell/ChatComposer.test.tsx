import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { apply, type ChatStatus, initialState, setChat, useHive } from "../store";
import { transport } from "../transport";
import { base64, ChatComposer, MAX_IMAGE_DATA, MAX_IMAGES } from "./ChatComposer";

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

const PNG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const png = (bytes = PNG) => new File([new Uint8Array(bytes)], "a.png", { type: "image/png" });
const paste = (input: HTMLElement, files: File[]) =>
  act(async () => {
    fireEvent.paste(input, { clipboardData: { files } });
  });

test("files are read as base64, in slices", async () => {
  expect(await base64(png())).toBe("iVBORw0KGgo=");
  const big = new Uint8Array(0x8000 * 2 + 5).fill(65);
  expect(await base64(new Blob([big]))).toBe(btoa("A".repeat(big.length)));
});

test("pasted images show as thumbnails, can be removed, and go with the message", async () => {
  const { send, input } = composer();
  open();
  await paste(input, [png(), png([0x47, 0x49, 0x46])]);
  const list = screen.getByRole("list", { name: "Images to send" });
  const thumbs = list.querySelectorAll("img");
  expect([...thumbs].map((img) => img.getAttribute("src"))).toEqual([
    "data:image/png;base64,iVBORw0KGgo=",
    "data:image/png;base64,R0lG",
  ]);
  fireEvent.click(screen.getByTitle("Remove image 2"));
  expect(list.querySelectorAll("img")).toHaveLength(1);
  // An image alone can be sent.
  fireEvent.keyDown(input, { key: "Enter" });
  expect(send.mock.calls).toEqual([[3, "", [{ media_type: "image/png", data: "iVBORw0KGgo=" }]]]);
  expect(screen.queryByRole("list", { name: "Images to send" })).toBeNull();
  // Pasting text is left to the textarea.
  await paste(input, []);
  expect(screen.queryByRole("alert")).toBeNull();
});

test("dropped files are added; other files, too many or too large ones are refused", async () => {
  const { input } = composer();
  const form = input.closest("form") as HTMLFormElement;
  const drop = (files: File[], types = ["Files"]) =>
    act(async () => {
      fireEvent.drop(form, { dataTransfer: { files, types } });
    });
  // Not before the chat started.
  expect(fireEvent.dragOver(form, { dataTransfer: { types: ["Files"] } })).toBe(true);
  await drop([png()]);
  expect(screen.queryByRole("list", { name: "Images to send" })).toBeNull();
  open();
  expect(fireEvent.dragOver(form, { dataTransfer: { types: ["text/plain"] } })).toBe(true);
  expect(fireEvent.dragOver(form, { dataTransfer: { types: ["Files"] } })).toBe(false);
  await drop([png()], ["text/plain"]);
  expect(screen.queryByRole("list", { name: "Images to send" })).toBeNull();
  await drop([new File(["x"], "a.txt", { type: "text/plain" })]);
  expect(screen.getByRole("alert").textContent).toBe(
    "Only PNG, JPEG, GIF and WebP images can be added.",
  );
  await drop([png()]);
  expect(screen.queryByRole("alert")).toBeNull();
  await drop(Array.from({ length: MAX_IMAGES }, () => png()));
  const tooMany = "At most 10 images, 3 MiB together, can be sent at once.";
  expect(screen.getByRole("alert").textContent).toBe(tooMany);
  expect(screen.getAllByRole("img")).toHaveLength(1);
  // 3 MiB of base64 is 2.25 MiB of bytes: exactly that fits with nothing else.
  fireEvent.click(screen.getByTitle("Remove image 1"));
  await drop([png(new Array((MAX_IMAGE_DATA / 4) * 3).fill(0))]);
  expect(screen.getAllByRole("img")).toHaveLength(1);
  await drop([png()]);
  expect(screen.getByRole("alert").textContent).toBe(tooMany);
});

test("a message the app could not send says why", async () => {
  const { send, input } = composer();
  send.mockRejectedValue("frame payload too large");
  open();
  fireEvent.change(input, { target: { value: "hi" } });
  await act(async () => {
    fireEvent.keyDown(input, { key: "Enter" });
  });
  expect(screen.getByRole("alert").textContent).toBe("frame payload too large");
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
    "Plan only",
  ]);
  fireEvent.click(screen.getByRole("option", { name: "Plan only" }));
  expect(setMode.mock.calls).toEqual([[3, "plan"]]);
  // The selector follows the service, not the click.
  expect(select.textContent).toBe("Default");
  act(() => apply({ type: "chat_status", channel: 3, ...status(false), mode: "plan" }));
  expect(select.textContent).toBe("Plan only");
});
