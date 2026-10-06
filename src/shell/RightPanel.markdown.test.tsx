import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
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
  // Sanitised as #43: only http(s)/mailto links, raw HTML as text, no image loaded.
  expect([...shown.querySelectorAll("a")].map((a) => a.getAttribute("href"))).toEqual([
    "https://x.dev",
  ]);
  expect(shown.textContent).toContain("bad");
  expect(shown.textContent).toContain("<b>raw</b>");
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
