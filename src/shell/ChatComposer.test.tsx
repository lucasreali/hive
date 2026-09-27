import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import {
  apply,
  type ChatStatus,
  EMPTY_DRAFT,
  initialState,
  setChat,
  setChatScroll,
  setDraft,
  useHive,
} from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import {
  base64,
  ChatComposer,
  MAX_IMAGE_DATA,
  MAX_IMAGES,
  mention,
  mentionPaths,
  rankPaths,
} from "./ChatComposer";

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
  // A file larger than the limit is not even read.
  fireEvent.click(screen.getByTitle("Remove image 1"));
  const huge = png();
  const read = spyOn(huge, "arrayBuffer");
  Object.defineProperty(huge, "size", { value: MAX_IMAGE_DATA + 1 });
  await drop([huge]);
  expect([screen.getByRole("alert").textContent, read.mock.calls.length]).toEqual([tooMany, 0]);
});

test("Attach image opens a picker for images, whose files go through the same checks", async () => {
  const { input } = composer();
  const attach = screen.getByRole("button", { name: "Attach image" }) as HTMLButtonElement;
  expect(attach.disabled).toBe(true);
  open();
  expect(attach.disabled).toBe(false);
  const picker = input.closest("form")?.querySelector('input[type="file"]') as HTMLInputElement;
  expect([picker.accept, picker.multiple]).toEqual([
    "image/png,image/jpeg,image/gif,image/webp",
    true,
  ]);
  const click = spyOn(picker, "click");
  fireEvent.click(attach);
  expect(click).toHaveBeenCalledTimes(1);
  const pick = (files: File[]) =>
    act(async () => {
      fireEvent.change(picker, { target: { files } });
    });
  await pick([]);
  expect(screen.queryByRole("list", { name: "Images to send" })).toBeNull();
  await pick([png()]);
  expect(screen.getAllByRole("img")).toHaveLength(1);
  await pick([new File(["x"], "a.txt", { type: "text/plain" })]);
  expect(screen.getByRole("alert").textContent).toBe(
    "Only PNG, JPEG, GIF and WebP images can be added.",
  );
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
    "Plan",
    "Auto",
  ]);
  fireEvent.click(screen.getByRole("option", { name: "Plan" }));
  expect(setMode.mock.calls).toEqual([[3, "plan"]]);
  // The selector follows the service, not the click.
  expect(select.textContent).toBe("Default");
  act(() => apply({ type: "chat_status", channel: 3, ...status(false), mode: "plan" }));
  expect(select.textContent).toBe("Plan");
});

test("Shift+Tab asks for the next mode, like the CLI, from Auto back to Default", () => {
  const setMode = spyOn(transport, "chatSetMode").mockResolvedValue();
  const { input } = composer();
  open(["compact"]);
  const shiftTab = () => fireEvent.keyDown(input, { key: "Tab", shiftKey: true });
  const asked = () => setMode.mock.calls.at(-1)?.[1];
  // Shift+Tab stays in the composer: the focus does not move.
  expect(shiftTab()).toBe(false);
  expect(asked()).toBe("accept_edits");
  for (const [mode, next] of [
    ["accept_edits", "plan"],
    ["plan", "auto"],
    ["auto", "default"],
  ] as const) {
    act(() => apply({ type: "chat_status", channel: 3, ...status(false), mode }));
    shiftTab();
    expect(asked()).toBe(next);
  }
  // Even with the command list shown, it switches the mode; plain Tab still picks.
  fireEvent.change(input, { target: { value: "/c" } });
  shiftTab();
  expect([input.value, asked(), setMode.mock.calls.length]).toEqual(["/c", "default", 5]);
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

test("the draft (text, images, caret) comes back when the composer shows again; sending clears it", async () => {
  const { send, input } = composer();
  open();
  fireEvent.change(input, { target: { value: "hello there" } });
  input.setSelectionRange(2, 5);
  await paste(input, [png()]);
  // The tab switches: the composer unmounts, and mounts again.
  cleanup();
  render(<ChatComposer chat={3} />);
  const again = screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement;
  expect(again.value).toBe("hello there");
  expect([again.selectionStart, again.selectionEnd]).toEqual([2, 5]);
  expect(
    screen.getByRole("list", { name: "Images to send" }).querySelector("img")?.getAttribute("src"),
  ).toBe("data:image/png;base64,iVBORw0KGgo=");

  fireEvent.keyDown(again, { key: "Enter" });
  expect(send.mock.calls).toEqual([
    [3, "hello there", [{ media_type: "image/png", data: "iVBORw0KGgo=" }]],
  ]);
  expect(useHive.getState().drafts[3]).toBeUndefined();
  expect(again.value).toBe("");
  // Unmounted after sending, it keeps only where the caret was.
  cleanup();
  expect(useHive.getState().drafts[3]).toEqual({ ...EMPTY_DRAFT });
});

test("two chats keep separate drafts, and a closed chat drops its draft and scroll", () => {
  const { input } = composer();
  open();
  setChat(4, "/v");
  render(<ChatComposer chat={4} />);
  const other = screen.getAllByRole("textbox", { name: "Message" })[1] as HTMLTextAreaElement;
  fireEvent.change(input, { target: { value: "for three" } });
  fireEvent.change(other, { target: { value: "for four" } });
  expect([input.value, other.value]).toEqual(["for three", "for four"]);
  act(() => setChatScroll(3, 500, false));
  expect(useHive.getState().chatScrolls[3]).toEqual({ offset: 500, atBottom: false });

  act(() => setChat(3, null));
  expect(useHive.getState().drafts[3]).toBeUndefined();
  expect(useHive.getState().chatScrolls[3]).toBeUndefined();
  expect(useHive.getState().drafts[4]?.text).toBe("for four");
  // Nothing is kept for a chat that is not open.
  setDraft(3, { text: "late" });
  setChatScroll(3, 1, true);
  expect(useHive.getState().drafts[3]).toBeUndefined();
  expect(useHive.getState().chatScrolls[3]).toBeUndefined();
  expect(EMPTY_DRAFT).toEqual({ text: "", images: [], start: 0, end: 0 });
});

const said = (texts: string[]) =>
  act(() =>
    apply({
      type: "chat_entries",
      channel: 3,
      chat: 3,
      entries: texts.map((text, i) => ({
        id: i + 1,
        kind: i === 1 ? "assistant" : "user",
        text,
        tool: null,
        parent: null,
        status: null,
        output: null,
        images: [],
      })),
      replace_last: false,
    }),
  );

test("↑/↓ walk this chat's sent messages, text only, and come back to what was typed", () => {
  const { input } = composer();
  open();
  // The assistant's reply and an image-only message (no text) are not in the history.
  said(["first", "a reply", "second\nline", ""]);
  fireEvent.change(input, { target: { value: "draft" } });
  input.setSelectionRange(2, 2);
  expect(fireEvent.keyDown(input, { key: "ArrowUp" })).toBe(false);
  expect(input.value).toBe("second\nline");
  expect([input.selectionStart, input.selectionEnd]).toEqual([11, 11]);
  // On the last line of a multi-line message ↑ moves the caret, as usual.
  expect(fireEvent.keyDown(input, { key: "ArrowUp" })).toBe(true);
  expect(input.value).toBe("second\nline");
  input.setSelectionRange(3, 3);
  fireEvent.keyDown(input, { key: "ArrowUp" });
  expect(input.value).toBe("first");
  // Nothing older: the key does its usual thing.
  expect(fireEvent.keyDown(input, { key: "ArrowUp" })).toBe(true);
  expect(input.value).toBe("first");
  // Modified arrows are left alone.
  expect(fireEvent.keyDown(input, { key: "ArrowDown", shiftKey: true })).toBe(true);
  fireEvent.keyDown(input, { key: "ArrowDown" });
  expect(input.value).toBe("second\nline");
  // ↓ on the first line of a multi-line message moves the caret.
  input.setSelectionRange(1, 1);
  expect(fireEvent.keyDown(input, { key: "ArrowDown" })).toBe(true);
  input.setSelectionRange(8, 8);
  fireEvent.keyDown(input, { key: "ArrowDown" });
  expect(input.value).toBe("draft");
  expect(input.selectionStart).toBe(5);
  // Past what was typed ↓ does nothing more.
  expect(fireEvent.keyDown(input, { key: "ArrowDown" })).toBe(true);
  expect(input.value).toBe("draft");
});

test("typing after ↑ keeps the text as the new draft; ↑ inside a multi-line text moves the caret", () => {
  const { input } = composer();
  open();
  said(["first"]);
  fireEvent.change(input, { target: { value: "one\ntwo" } });
  input.setSelectionRange(5, 5);
  expect(fireEvent.keyDown(input, { key: "ArrowUp" })).toBe(true);
  expect(input.value).toBe("one\ntwo");
  input.setSelectionRange(1, 1);
  fireEvent.keyDown(input, { key: "ArrowUp" });
  expect(input.value).toBe("first");
  fireEvent.change(input, { target: { value: "first!" } });
  // Editing ends the walk: ↓ no longer brings back the old draft.
  expect(fireEvent.keyDown(input, { key: "ArrowDown" })).toBe(true);
  expect(input.value).toBe("first!");
  // The same text brought back still puts the caret at its end.
  fireEvent.change(input, { target: { value: "first" } });
  input.setSelectionRange(0, 0);
  fireEvent.keyDown(input, { key: "ArrowUp" });
  expect([input.value, input.selectionStart]).toEqual(["first", 5]);
});

test("↑ moves in the slash-command list instead of the history", () => {
  const { input } = composer();
  open(["clear", "compact"]);
  said(["first"]);
  fireEvent.change(input, { target: { value: "/c" } });
  fireEvent.keyDown(input, { key: "ArrowUp" });
  expect(input.value).toBe("/c");
  expect(screen.getAllByRole("option")[1].getAttribute("aria-selected")).toBe("true");
});

test("Ctrl+C copies a selection, stops a running turn, or clears the composer", async () => {
  const { stop, input } = composer();
  open();
  fireEvent.change(input, { target: { value: "hello" } });
  await paste(input, [png()]);
  // With text selected it copies, as usual.
  input.setSelectionRange(0, 3);
  expect(fireEvent.keyDown(input, { key: "c", ctrlKey: true })).toBe(true);
  expect(input.value).toBe("hello");
  input.setSelectionRange(5, 5);
  // Other modifiers or keys are left alone.
  expect(fireEvent.keyDown(input, { key: "c", ctrlKey: true, altKey: true })).toBe(true);
  expect(fireEvent.keyDown(input, { key: "c", ctrlKey: true, metaKey: true })).toBe(true);
  expect(fireEvent.keyDown(input, { key: "c" })).toBe(true);
  expect(fireEvent.keyDown(input, { key: "v", ctrlKey: true })).toBe(true);
  expect(input.value).toBe("hello");
  // While Claude works it interrupts, keeping the draft.
  act(() => apply({ type: "chat_status", channel: 3, ...status(true) }));
  expect(fireEvent.keyDown(input, { key: "c", ctrlKey: true })).toBe(false);
  expect(stop.mock.calls).toEqual([[3]]);
  expect(input.value).toBe("hello");
  // Idle it clears the text and the images; the chat stays open.
  act(() => apply({ type: "chat_status", channel: 3, ...status(false) }));
  expect(fireEvent.keyDown(input, { key: "C", ctrlKey: true, shiftKey: true })).toBe(false);
  expect(stop.mock.calls).toEqual([[3]]);
  expect(input.value).toBe("");
  expect(screen.queryByRole("list", { name: "Images to send" })).toBeNull();
  expect(useHive.getState().drafts[3]).toBeUndefined();
  expect(input.disabled).toBe(false);
});

test("a listing's paths are its files and their folders, ending in /", () => {
  expect(mentionPaths(["src/a/b.ts", "src/c.ts", "README.md"])).toEqual([
    "README.md",
    "src/",
    "src/a/",
    "src/a/b.ts",
    "src/c.ts",
  ]);
  expect(rankPaths(["src/", "src/chat.ts", "docs/c.md", "x/s/ch"], "sch")).toEqual([
    "x/s/ch",
    "src/chat.ts",
  ]);
  expect(
    rankPaths(
      Array.from({ length: 60 }, (_, i) => `f${i}`),
      "",
    ),
  ).toHaveLength(50);
  expect([mention("a/b.ts"), mention("my dir/")]).toEqual(["@a/b.ts", '@"my dir/"']);
});

/** The chat runs at the root of worktree `/w`, whose listing is `files`. */
function worktree(files: string[]) {
  const [repo] = MOCK_REPOS as [(typeof MOCK_REPOS)[number]];
  const [main] = repo.worktrees as [(typeof repo.worktrees)[number]];
  act(() => {
    apply({
      type: "projects",
      projects: [{ ...repo, worktrees: [{ ...main, id: "/w", path: "/w" }] }],
    });
    apply({ type: "files", path: "/w", files, truncated: false });
  });
}

test("@ lists the worktree's files and folders, fuzzy-filtered, and picking puts in the path", () => {
  const { send, input } = composer();
  open(["compact"]);
  worktree(["src/chat.ts", "src/deep/x.md", "notes.txt"]);
  const list = () => screen.queryByRole("listbox", { name: "Files" });
  const options = () => screen.getAllByRole("option").map((o) => o.textContent);
  const selected = () => screen.getByRole("option", { selected: true }).textContent;
  fireEvent.change(input, { target: { value: "see @" } });
  expect(options()).toEqual([
    "@notes.txt",
    "@src/",
    "@src/chat.ts",
    "@src/deep/",
    "@src/deep/x.md",
  ]);
  // The list's worktree is watched while it shows.
  expect(useHive.getState().mentioning).toBe("/w");
  fireEvent.change(input, { target: { value: "see @sd" } });
  expect(options()).toEqual(["@src/deep/", "@src/deep/x.md"]);
  // ↓ and Enter pick a folder: no space after it, and its files are listed.
  fireEvent.keyDown(input, { key: "ArrowDown" });
  expect(selected()).toBe("@src/deep/x.md");
  fireEvent.keyDown(input, { key: "ArrowUp" });
  fireEvent.keyDown(input, { key: "Enter" });
  expect([input.value, input.selectionStart, send.mock.calls]).toEqual(["see @src/deep/", 14, []]);
  expect(options()).toEqual(["@src/deep/", "@src/deep/x.md"]);
  // Tab picks a file, then a space; the list is gone and Enter sends the text as typed.
  fireEvent.keyDown(input, { key: "ArrowDown" });
  fireEvent.keyDown(input, { key: "Tab" });
  expect([input.value, list()]).toEqual(["see @src/deep/x.md ", null]);
  expect(useHive.getState().mentioning).toBeNull();
  fireEvent.keyDown(input, { key: "Enter" });
  expect(send.mock.calls).toEqual([[3, "see @src/deep/x.md ", []]]);

  // In the middle of the text, only the word the caret ends is replaced; a click picks too.
  fireEvent.change(input, { target: { value: "a @no b" } });
  input.focus();
  input.setSelectionRange(5, 5);
  // A click puts the caret there (React sees it on mouseup).
  fireEvent.mouseUp(input);
  fireEvent.click(screen.getByRole("option", { name: "@notes.txt" }));
  expect(input.value).toBe("a @notes.txt  b");
  // Esc hides the list; typing shows it again. No match, or no @ word, lists nothing.
  fireEvent.change(input, { target: { value: "@c" } });
  fireEvent.keyDown(input, { key: "Escape" });
  expect(list()).toBeNull();
  expect(useHive.getState().mentioning).toBeNull();
  fireEvent.change(input, { target: { value: "@ch" } });
  expect(options()).toEqual(["@src/chat.ts"]);
  for (const value of ["@zzz", "me@src", "@src "]) {
    fireEvent.change(input, { target: { value } });
    expect(list()).toBeNull();
  }
});

test("@ lists nothing in a chat that does not run at a worktree's root", () => {
  const { input } = composer();
  open();
  fireEvent.change(input, { target: { value: "@" } });
  expect(screen.queryByRole("listbox")).toBeNull();
  expect(useHive.getState().mentioning).toBeNull();
});
