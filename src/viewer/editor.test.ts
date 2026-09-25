import { afterEach, expect, test } from "bun:test";
import { syntaxTree } from "@codemirror/language";
import { getOriginalDoc } from "@codemirror/merge";
import { EditorState } from "@codemirror/state";
import { runScopeHandlers } from "@codemirror/view";
import { toText } from "./buffer";
import {
  createEditor,
  createViewer,
  type Editor,
  revealLine,
  selectedLines,
  type Viewer,
} from "./editor";

let viewer: Viewer | null = null;
afterEach(() => {
  viewer?.destroy();
  viewer = null;
  document.body.innerHTML = "";
});

const until = async (check: () => boolean) => {
  for (let i = 0; i < 200 && !check(); i++) await new Promise((r) => setTimeout(r, 5));
  expect(check()).toBe(true);
};

test("shows a file read-only, as a diff only when it has an original", () => {
  viewer = createViewer(document.body, "notes.unknown-extension");
  const { view } = viewer;
  expect(document.body.contains(view.dom)).toBe(true);

  viewer.show({ content: "a\nb\n", original: null });
  expect(view.state.doc.toString()).toBe("a\nb\n");
  expect(view.state.facet(EditorState.readOnly)).toBe(true);
  expect(view.dom.classList.contains("cm-merge-b")).toBe(false);

  view.scrollDOM.scrollTop = 40;
  viewer.show({ content: "a\nB\n", original: "a\nb\n" });
  expect(view.state.doc.toString()).toBe("a\nB\n");
  expect(getOriginalDoc(view.state).toString()).toBe("a\nb\n");
  expect(view.scrollDOM.scrollTop).toBe(40);
});

test("highlights by the file name's language once it is loaded", async () => {
  viewer = createViewer(document.body, "src/auth/session.ts");
  const { view } = viewer;
  viewer.show({ content: "const a = 1;\n", original: null });
  await until(() => syntaxTree(view.state).length > 0);
  // A later answer keeps the language.
  viewer.show({ content: "let b = 2;\n", original: "const a = 1;\n" });
  expect(syntaxTree(view.state).topNode.name).toBe("Script");
});

test("a viewer closed before its language loads is left alone", async () => {
  const closed = createViewer(document.body, "a.rs");
  closed.show({ content: "fn main() {}\n", original: null });
  closed.destroy();
  viewer = createViewer(document.body, "b.rs");
  viewer.show({ content: "fn main() {}\n", original: null });
  const { view } = viewer;
  await until(() => syntaxTree(view.state).length > 0);
  expect(syntaxTree(closed.view.state).length).toBe(0);
});

test("the selected lines: none when empty, a line ending the range only when entered", () => {
  const seen: unknown[] = [];
  viewer = createViewer(document.body, "a.txt", (lines) => seen.push(lines));
  const { view } = viewer;
  viewer.show({ content: "one\ntwo\nthree\n", original: "one\n" });
  expect(seen).toEqual([null]);
  // From inside line 1 to the start of line 3: lines 1-2.
  view.dispatch({ selection: { anchor: 1, head: 8 } });
  view.dispatch({ selection: { anchor: 9, head: 10 } });
  view.dispatch({ selection: { anchor: 5 } });
  expect(seen).toEqual([null, { from: 1, to: 2 }, { from: 3, to: 3 }, null]);
  expect(selectedLines(view.state)).toBeNull();
});

test("commented lines are marked, only those the text has, and kept across shows", () => {
  viewer = createViewer(document.body, "a.txt");
  const { view } = viewer;
  const marked = () =>
    [...view.contentDOM.querySelectorAll(".cm-line")].map((l) =>
      l.classList.contains("cm-commented") ? l.textContent : null,
    );
  // Marked before the first show: taken by it.
  viewer.mark([{ from: 2, to: 2 }]);
  viewer.show({ content: "one\ntwo\nthree\nfour", original: null });
  expect(marked()).toEqual([null, "two", null, null]);
  // Overlapping and out-of-range ranges: each line once, lines past the end ignored.
  viewer.mark([
    { from: 3, to: 9 },
    { from: 0, to: 1 },
    { from: 1, to: 1 },
  ]);
  expect(marked()).toEqual(["one", null, "three", "four"]);
  viewer.show({ content: "one\ntwo", original: null });
  expect(marked()).toEqual(["one", null]);
  viewer.mark([]);
  expect(marked()).toEqual([null, null]);
});

/** An editor on `content` that records what it tells its owner. */
function editing(content: string, path = "a.txt") {
  const heard = { changes: [] as string[], saves: 0, selections: [] as unknown[] };
  const editor = createEditor(document.body, path, toText(content), {
    change: (doc) => heard.changes.push(doc.toString()),
    save: () => heard.saves++,
    select: (lines) => heard.selections.push(lines),
  });
  editors.push(editor);
  return { editor, view: editor.view, heard };
}
const editors: Editor[] = [];
afterEach(() => {
  for (const editor of editors.splice(0)) editor.destroy();
});

test("the editor is editable text that reports edits, selections and Ctrl+S", () => {
  const { view, heard } = editing("one\r\ntwo\n");
  expect(view.state.facet(EditorState.readOnly)).toBe(false);
  // Split on "\n" only: the "\r" stays in the line.
  expect(view.state.doc.lines).toBe(3);
  view.dispatch({ changes: { from: 0, insert: "x" } });
  expect(heard.changes).toEqual(["xone\r\ntwo\n"]);
  view.dispatch({ selection: { anchor: 0, head: 8 } });
  expect(heard.selections).toEqual([{ from: 1, to: 2 }]);
  const key = new KeyboardEvent("keydown", { key: "s", ctrlKey: true });
  expect(runScopeHandlers(view, key, "editor")).toBe(true);
  expect(heard.saves).toBe(1);
  // Undo is there.
  const undo = new KeyboardEvent("keydown", { key: "z", ctrlKey: true });
  runScopeHandlers(view, undo, "editor");
  expect(view.state.doc.toString()).toBe("one\r\ntwo\n");
});

test("a reload swaps the text, keeping scroll and a clamped selection", () => {
  const { editor, view, heard } = editing("0123456789\n");
  const doc = view.state.doc;
  editor.load(doc);
  expect(view.state.doc).toBe(doc); // Its own text: nothing to do.
  view.dispatch({ selection: { anchor: 2, head: 9 } });
  view.scrollDOM.scrollTop = 30;
  heard.selections.length = 0;
  editor.load(toText("abcde"));
  expect(view.state.doc.toString()).toBe("abcde");
  expect([view.state.selection.main.anchor, view.state.selection.main.head]).toEqual([2, 5]);
  expect(view.scrollDOM.scrollTop).toBe(30);
  expect(heard.selections).toEqual([{ from: 1, to: 1 }]);
  expect(heard.changes).toEqual([]); // A reload is not an edit.
  // Still editable, with its keys.
  view.dispatch({ changes: { from: 0, insert: "!" } });
  expect(heard.changes).toEqual(["!abcde"]);
});

test("compare shows the text against the disk's, read-only, until editing again", () => {
  const { editor, view } = editing("mine\n");
  editor.compare("disk\n");
  expect(view.state.facet(EditorState.readOnly)).toBe(true);
  expect(getOriginalDoc(view.state).toString()).toBe("disk\n");
  expect(view.dom.classList.contains("cm-merge-b")).toBe(true);
  editor.compare(null);
  expect(view.state.facet(EditorState.readOnly)).toBe(false);
  expect(view.dom.classList.contains("cm-merge-b")).toBe(false);
});

test("the editor highlights by the file name's language", async () => {
  const { view } = editing("const a = 1;\n", "src/a.ts");
  await until(() => syntaxTree(view.state).length > 0);
  expect(syntaxTree(view.state).topNode.name).toBe("Script");
});

test("a line is revealed selected, clamped to the text", () => {
  viewer = createViewer(document.body, "a.txt");
  viewer.show({ content: "one\ntwo\nthree", original: null });
  const { view } = viewer;
  const selected = () =>
    view.state.sliceDoc(view.state.selection.main.from, view.state.selection.main.to);
  revealLine(view, 2);
  expect(selected()).toBe("two");
  revealLine(view, 99);
  expect(selected()).toBe("three");
  revealLine(view, 0);
  expect(selected()).toBe("one");
});
