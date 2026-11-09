import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { asMac } from "../../test/mac";
import type { ChangedFile, FileText } from "../protocol";
import { apply } from "../reduce";
import { initialState, pinFile, select, setOpenFile, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { toText } from "../viewer/buffer";
import { RightPanel } from "./RightPanel";
import { TerminalArea } from "./TerminalArea";

// 11.2: a Markdown file's tab shows rendered or as text.

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [, api] = MOCK_REPOS;
const refactor = api.worktrees[1];

function panel() {
  spyOn(transport, "listChanges").mockImplementation(async () => {});
  apply({ type: "projects", projects: MOCK_REPOS });
  render(
    <>
      <RightPanel />
      <TerminalArea />
    </>,
  );
  act(() => select(refactor.id));
}

const answer = (path: string, content: string | null, patch: Partial<FileText> = {}) =>
  act(() =>
    apply({
      type: "file",
      worktree: refactor.path,
      path,
      content,
      base: content,
      version: "v",
      binary: false,
      too_large: false,
      error: null,
      ...patch,
    }),
  );

const open = (path: string, editing = false) =>
  act(() => setOpenFile({ worktree: refactor.path, path }, editing));
const eye = () => screen.queryByRole("button", { name: "Show rendered Markdown" });
const pressed = () => eye()?.getAttribute("aria-pressed");
const body = () => document.querySelector(".file-view-body") as HTMLElement;
const editor = () => body().querySelector(".cm-editor");

const MD = [
  "# Title",
  "- one",
  "| a | b |\n|---|---|\n| 1 | 2 |",
  "```\ncode\n```",
  "[site](https://x.dev) [bad](javascript:alert(1)) <b>raw</b> ![logo](https://x.dev/i.png)",
].join("\n\n");

test("the eye button shows only for Markdown files and switches the tab to rendered and back", () => {
  panel();
  open("src/middleware/auth.ts");
  expect(eye()).toBeNull();

  open("README.md");
  answer("README.md", MD);
  expect(pressed()).toBe("false");
  expect(eye()?.title).toBe("Show rendered (Ctrl+Shift+V)");
  expect(editor()).not.toBeNull();

  fireEvent.click(eye() as HTMLElement);
  expect(pressed()).toBe("true");
  expect(eye()?.title).toBe("Show the text (Ctrl+Shift+V)");
  expect(editor()).toBeNull();
  const shown = body().querySelector(".markdown") as HTMLElement;
  expect(shown.querySelector("h1")?.textContent).toBe("Title");
  expect(shown.querySelector("li")?.textContent).toBe("one");
  expect([...shown.querySelectorAll("td")].map((c) => c.textContent)).toEqual(["1", "2"]);
  expect(shown.querySelector("pre code")?.textContent).toBe("code");
  // Sanitised as #43: only http(s)/mailto links, an unlisted tag as its text (D20.2), no image.
  expect([...shown.querySelectorAll("a")].map((a) => a.getAttribute("href"))).toEqual([
    "https://x.dev",
  ]);
  expect(shown.textContent).toContain("bad");
  expect(shown.textContent).toContain("bad raw logo");
  expect(shown.querySelector("b, img")).toBeNull();
  expect(shown.querySelector(".md-img")?.textContent).toBe("logo");

  fireEvent.click(eye() as HTMLElement);
  expect(pressed()).toBe("false");
  expect(editor()).not.toBeNull();
});

test("a macOS tooltip names Cmd+Shift+V", () => {
  asMac();
  panel();
  open("README.md");
  expect(eye()?.title).toBe("Show rendered (⇧⌘V)");
});

test("the rendered choice is the tab's: another Markdown file, or one opened again, starts as text", () => {
  panel();
  open("README.md");
  fireEvent.click(eye() as HTMLElement);
  // Kept (11.1), so docs/api.md gets a tab of its own instead of replacing it.
  act(() => pinFile({ worktree: refactor.path, path: "README.md" }));
  open("docs/api.md");
  expect(pressed()).toBe("false");
  open("README.md");
  expect(pressed()).toBe("true");
  act(() => setOpenFile(null));
  open("README.md");
  expect(pressed()).toBe("false");
});

test("Edit shows the text again; Ctrl+S saves the edits shown rendered and shows the text", () => {
  panel();
  const md: ChangedFile = {
    path: "notes.md",
    status: "modified",
    old_path: null,
    added: 1,
    removed: 0,
  };
  act(() =>
    apply({
      type: "changes",
      path: refactor.path,
      base: "head",
      branch: null,
      base_error: null,
      files: [md],
      added: 1,
      removed: 0,
      error: null,
    }),
  );
  open("notes.md");
  answer("notes.md", "# Disk\n", { base: "# Old\n" });
  fireEvent.click(eye() as HTMLElement);
  fireEvent.click(screen.getByRole("button", { name: "Edit" }));
  expect(pressed()).toBe("false");
  expect(useHive.getState().editing).toBe(true);
  expect(editor()?.querySelector(".cm-content")?.getAttribute("contenteditable")).toBe("true");

  // Unsaved edits show rendered.
  act(() => useHive.setState((s) => ({ edit: s.edit && { ...s.edit, doc: toText("# Mine\n") } })));
  fireEvent.click(eye() as HTMLElement);
  expect(body().querySelector("h1")?.textContent).toBe("Mine");

  const save = spyOn(transport, "saveFile").mockResolvedValue();
  fireEvent.keyDown(window, { key: "s", ctrlKey: true, shiftKey: true });
  fireEvent.keyDown(window, { key: "s", ctrlKey: true, altKey: true });
  fireEvent.keyDown(window, { key: "x", ctrlKey: true });
  fireEvent.keyDown(window, { key: "s" });
  expect(pressed()).toBe("true");
  expect(save).not.toHaveBeenCalled();
  fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  expect(save).toHaveBeenCalledWith(refactor.path, "notes.md", "# Mine\n", "v");
  expect(pressed()).toBe("false");
  expect(editor()).not.toBeNull();
});

test("a Markdown file that cannot be shown says why, rendered too", () => {
  panel();
  open("README.md");
  answer("README.md", null, { binary: true, base: null });
  fireEvent.click(eye() as HTMLElement);
  expect(body().textContent).toContain("Binary file not shown.");
  expect(body().querySelector(".markdown")).toBeNull();
});

// 25.4: a block of the preview edited as its source.

const blockBox = () => body().querySelector<HTMLTextAreaElement>(".md-block-editor");
const DOC = "# Title\n\nA paragraph\nof two lines.\n\n- one\n- two\n\nEnd [link](https://x.dev)\n";

function preview() {
  panel();
  open("README.md");
  answer("README.md", DOC);
  fireEvent.click(eye() as HTMLElement);
}

test("a click on a block opens its source; only its lines change, and the save is as the text's", () => {
  const save = spyOn(transport, "saveFile").mockImplementation(async () => {});
  save.mockClear();
  preview();
  const dirty = () =>
    screen.queryByRole("button", { name: "Close file README.md (unsaved changes)" });
  fireEvent.click(screen.getByText(/A paragraph/));
  expect(blockBox()?.value).toBe("A paragraph\nof two lines.");
  expect(dirty()).toBeNull();
  fireEvent.input(blockBox() as HTMLElement, { target: { value: "A *new* one." } });
  expect(dirty()).not.toBeNull();
  const edited = "# Title\n\nA *new* one.\n\n- one\n- two\n\nEnd [link](https://x.dev)\n";
  expect(useHive.getState().edit?.doc.toString()).toBe(edited);

  // ⌘S: saved, and it stays here, the block still open.
  fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  expect(save.mock.calls).toEqual([[refactor.path, "README.md", edited, "v"]]);
  expect(pressed()).toBe("true");
  expect(blockBox()).not.toBeNull();
  // Escape renders it again; every other block as it was.
  fireEvent.keyDown(blockBox() as HTMLElement, { key: "Escape" });
  expect(blockBox()).toBeNull();
  expect(body().querySelector(".markdown em")?.textContent).toBe("new");
  expect(body().querySelector("h1")?.textContent).toBe("Title");
  expect(body().querySelectorAll("li")).toHaveLength(2);
});

test("a click on a list opens the whole list; on its link, nothing", () => {
  preview();
  const link = screen.getByRole("link", { name: "link" });
  fireEvent.click(link);
  expect(blockBox()).toBeNull();
  fireEvent.click(screen.getByText("two"));
  expect(blockBox()?.value).toBe("- one\n- two");
});

test("a change on disk under edits made in the preview shows the banner, without View diff", () => {
  preview();
  fireEvent.click(screen.getByText("Title"));
  fireEvent.input(blockBox() as HTMLElement, { target: { value: "# New" } });
  answer("README.md", "agent\n");
  const banner = screen.getByRole("alert");
  expect(banner.textContent).toContain("Changed on disk.");
  expect(within(banner).queryByRole("button", { name: "View diff" })).toBeNull();
});

test("Ctrl+S with no block edited still shows the text (11.2)", () => {
  spyOn(transport, "saveFile").mockImplementation(async () => {});
  preview();
  fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  expect(pressed()).toBe("false");
});
