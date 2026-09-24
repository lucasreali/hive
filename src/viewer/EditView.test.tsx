import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { asMac } from "../../test/mac";
import { TerminalArea } from "../shell/TerminalArea";
import { apply, type FileText, initialState, setOpenFile, useHive } from "../store";
import { transport } from "../transport";
import { saveOpenFile } from "./EditView";

afterEach(() => {
  mock.restore();
  cleanup();
  useHive.setState(initialState, true);
});

const worktree = "/w";
const answer = (content: string | null, patch: Partial<FileText> = {}) =>
  apply({
    type: "file",
    worktree,
    path: "a.ts",
    content,
    base: "base\n",
    version: content && `v:${content}`,
    binary: false,
    too_large: false,
    error: null,
    ...patch,
  });
const view = () => EditorView.findFromDOM(document.querySelector(".cm-editor") as HTMLElement);
const type = (text: string) =>
  act(() => view()?.dispatch({ changes: { from: 0, to: view()?.state.doc.length, insert: text } }));
const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;
// The file tab's × turns into the unsaved dot, and says so in its name.
const dirty = () => screen.queryByRole("button", { name: /\(unsaved changes\)$/ });
const banner = () => screen.queryByRole("alert");

/** A file without changes, opened from the tree: editable once its text arrives. */
function editing(content = "one\n") {
  const saves = spyOn(transport, "saveFile").mockImplementation(async () => {});
  act(() => setOpenFile({ worktree, path: "a.ts" }, true));
  render(<TerminalArea />);
  act(() => answer(content));
  return saves;
}

test("an unchanged file is edited and saved with the version it was read at", () => {
  const saves = editing();
  expect(view()?.state.facet(EditorState.readOnly)).toBe(false);
  expect(screen.queryByText("No changes in this file.")).toBeNull();
  expect(button("Save").disabled).toBe(true);
  expect(dirty()).toBeNull();

  type("two\n");
  expect(dirty()).not.toBeNull();
  fireEvent.click(button("Save"));
  expect(saves.mock.calls).toEqual([[worktree, "a.ts", "two\n", "v:one\n"]]);
  expect(button("Save").disabled).toBe(true); // Until the service answers.
  saveOpenFile(); // Nothing more to send meanwhile.
  expect(saves).toHaveBeenCalledTimes(1);
  act(() => apply({ type: "file_saved", worktree, path: "a.ts", version: "v:two\n" }));
  expect(dirty()).toBeNull();

  // A failure is shown under the header.
  type("three\n");
  fireEvent.click(button("Save"));
  const failure = { type: "save_failed", worktree, path: "a.ts", error: "io" } as const;
  act(() => apply({ ...failure, message: "No space left on device" }));
  expect(screen.getByText("No space left on device")).toBeDefined();
  expect(button("Save").disabled).toBe(false);
});

test("a save the app cannot send fails at once instead of waiting forever", async () => {
  const saves = editing();
  saves.mockImplementation(() => Promise.reject("not connected to the hive service"));
  type("two\n");
  fireEvent.click(button("Save"));
  await waitFor(() => expect(screen.getByText("not connected to the hive service")).toBeDefined());
  expect(button("Save").disabled).toBe(false);

  // A late rejection leaves another open file's buffer alone.
  let reject: (reason: string) => void = () => {};
  saves.mockImplementation(() => new Promise((_, no) => (reject = no)));
  fireEvent.click(button("Save"));
  act(() => {
    setOpenFile({ worktree, path: "b.ts" }, true);
    answer("b\n", { path: "b.ts" });
  });
  await act(async () => reject("late"));
  expect(useHive.getState().edit?.path).toBe("b.ts");
  expect(useHive.getState().edit?.error).toBeNull();
});

test("a clean buffer follows the disk; a dirty one shows the conflict banner", () => {
  editing();
  const editor = view();
  act(() => answer("agent\n"));
  expect(view()).toBe(editor); // Reloaded in place.
  expect(view()?.state.doc.toString()).toBe("agent\n");
  expect(banner()).toBeNull();

  type("mine\n");
  act(() => answer("agent 2\n"));
  expect(banner()?.textContent).toContain("Changed on disk.");
  expect(view()?.state.doc.toString()).toBe("mine\n");
  fireEvent.click(button("View diff"));
  expect(view()?.dom.classList.contains("cm-merge-b")).toBe(true);
  expect(view()?.state.facet(EditorState.readOnly)).toBe(true);
  fireEvent.click(button("View diff"));
  expect(view()?.dom.classList.contains("cm-merge-b")).toBe(false);

  fireEvent.click(button("View diff"));
  fireEvent.click(button("Reload"));
  expect(banner()).toBeNull();
  expect(view()?.state.doc.toString()).toBe("agent 2\n");
  expect(view()?.dom.classList.contains("cm-merge-b")).toBe(false);
  expect(dirty()).toBeNull();

  type("mine again\n");
  act(() => answer(null));
  expect(banner()?.textContent).toContain("Deleted on disk.");
  fireEvent.click(button("Keep mine"));
  expect(banner()).toBeNull();
  expect(view()?.state.doc.toString()).toBe("mine again\n");
  expect(dirty()).not.toBeNull();
});

test("a changed file switches between its diff and editable text", () => {
  spyOn(transport, "saveFile").mockImplementation(async () => {});
  apply({
    type: "changes",
    path: worktree,
    files: [{ path: "a.ts", status: "modified", old_path: null, added: 1, removed: 1 }],
    added: 1,
    removed: 1,
    error: null,
  });
  act(() => setOpenFile({ worktree, path: "a.ts" }));
  render(<TerminalArea />);
  expect(button("Edit").disabled).toBe(true); // No text yet.
  act(() => answer("one\n"));
  expect(view()?.state.facet(EditorState.readOnly)).toBe(true);
  fireEvent.click(button("Edit"));
  expect(view()?.state.facet(EditorState.readOnly)).toBe(false);
  expect(button("Diff").disabled).toBe(false);
  type("two\n");
  expect(button("Diff").disabled).toBe(true);
  expect(button("Diff").title).toBe("Save or reload the file first");
  type("one\n");
  fireEvent.click(button("Diff"));
  expect(view()?.dom.classList.contains("cm-merge-b")).toBe(true);
  expect(button("Edit").disabled).toBe(false);
});

test("the header warns of a working agent and opens the file elsewhere", () => {
  const opened = spyOn(transport, "openInEditor").mockImplementation(async () => {});
  editing();
  expect(screen.queryByText("Agent working here")).toBeNull();
  act(() => {
    apply({ type: "agent_detected", channel: 1, id: "s", project: "/p", worktree, cwd: worktree });
    apply({
      type: "agent_state",
      id: "s",
      state: "working",
      urgency: 2,
      pending: false,
      subagents: [],
    });
  });
  expect(screen.getByText("Agent working here")).toBeDefined();

  fireEvent.click(button("Open in external editor"));
  expect(opened.mock.calls).toEqual([[worktree, "a.ts"]]);
  act(() => useHive.setState({ editorNotice: "Only the Hive app opens an external editor" }));
  expect(screen.getByText("Only the Hive app opens an external editor")).toBeDefined();
});

test("the file view's tooltips name each platform's keys and default app", () => {
  editing();
  expect(button("Save").title).toBe("Save (Ctrl+S)");
  expect(button("Open in external editor").title).toBe(
    "Open in external editor (the Windows default app for the file)",
  );
  cleanup();
  asMac();
  editing();
  expect(button("Save").title).toBe("Save (⌘S)");
  expect(button("Open in external editor").title).toBe(
    "Open in external editor (the default app for the file)",
  );
});

test("unsaved edits are dropped only when the user agrees", () => {
  editing();
  const confirm = spyOn(window, "confirm").mockImplementation(() => false);
  fireEvent.click(button("Close file a.ts"));
  expect(confirm).not.toHaveBeenCalled(); // Clean: closes at once.
  expect(useHive.getState().openFile).toBeNull();

  cleanup();
  editing();
  type("mine\n");
  fireEvent.click(button("Close file a.ts (unsaved changes)"));
  expect(confirm.mock.calls).toEqual([["Discard your unsaved changes to a.ts?"]]);
  expect(useHive.getState().openFile).not.toBeNull();
  confirm.mockImplementation(() => true);
  fireEvent.click(button("Close file a.ts (unsaved changes)"));
  expect(useHive.getState().openFile).toBeNull();
  expect(screen.queryByRole("region", { name: "a.ts" })).toBeNull();
});
