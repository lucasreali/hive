import { afterEach, expect, test } from "bun:test";
import { undo } from "@codemirror/commands";
import { Chunk } from "@codemirror/merge";
import { toText } from "./buffer";
import { createEditor, type Editor } from "./editor";
import { markerLines } from "./gutterMarkers";

const marks = (base: string, text: string) =>
  markerLines(Chunk.build(toText(base), toText(text)), toText(text));

test("chunks become added, modified and deleted lines at the start, middle and end", () => {
  const base = "a\nb\nc\n";
  expect(marks(base, base)).toEqual([]);
  expect(marks(base, "x\na\nb\nc\n")).toEqual([{ line: 1, kind: "added" }]);
  expect(marks(base, "a\nx\ny\nb\nc\n")).toEqual([
    { line: 2, kind: "added" },
    { line: 3, kind: "added" },
  ]);
  expect(marks(base, "a\nb\nc\nx\n")).toEqual([{ line: 4, kind: "added" }]);
  expect(marks(base, "X\nb\nc\n")).toEqual([{ line: 1, kind: "modified" }]);
  expect(marks(base, "a\nX\nY\nc\n")).toEqual([
    { line: 2, kind: "modified" },
    { line: 3, kind: "modified" },
  ]);
  expect(marks(base, "a\nb\nX\n")).toEqual([{ line: 3, kind: "modified" }]);
  // A deletion marks the line the removed ones sat before.
  expect(marks(base, "b\nc\n")).toEqual([{ line: 1, kind: "deleted" }]);
  expect(marks(base, "a\nc\n")).toEqual([{ line: 2, kind: "deleted" }]);
  expect(marks(base, "a\nb\n")).toEqual([{ line: 3, kind: "deleted" }]);
  // A last line without a final newline, changed or extended, is modified (as in git).
  expect(marks("a\nb", "a\nb\nc")).toEqual([
    { line: 2, kind: "modified" },
    { line: 3, kind: "modified" },
  ]);
});

const editors: Editor[] = [];
afterEach(() => {
  for (const editor of editors.splice(0)) editor.destroy();
  document.body.innerHTML = "";
});

function editing(content: string) {
  const ignore = () => {};
  const editor = createEditor(document.body, "notes.txt", toText(content), {
    change: ignore,
    save: ignore,
    select: ignore,
  });
  editors.push(editor);
  return editor;
}

/** The kind of each marker the view draws, top to bottom. */
const shown = ({ view }: Editor) =>
  [...view.dom.querySelectorAll(".cm-changeGutter [class*='cm-change-']")].map(
    (e) => e.className.match(/cm-change-(\w+)/)?.[1],
  );

test("the markers follow the buffer as it is typed and undone", () => {
  const editor = editing("a\nb\n");
  expect(shown(editor)).toEqual([]);
  editor.setBase("a\nb\n");
  expect(shown(editor)).toEqual([]);
  // Their gutter sits left of the line numbers.
  const gutters = [...editor.view.dom.querySelectorAll(".cm-gutter")].map((g) => g.className);
  expect(gutters[0]).toContain("cm-changeGutter");
  expect(gutters[1]).toContain("cm-lineNumbers");

  editor.view.dispatch({ changes: { from: 2, insert: "new\n" } });
  expect(shown(editor)).toEqual(["added"]);
  undo(editor.view);
  expect(shown(editor)).toEqual([]);

  editor.view.dispatch({ changes: { from: 0, to: 2 } });
  expect(shown(editor)).toEqual(["deleted"]);
  // A reload (new text from disk) keeps them.
  editor.load(toText("A\nb\n"));
  expect(shown(editor)).toEqual(["modified"]);
});

test("a new base redraws the markers, and none hides them", () => {
  const editor = editing("a\nb\n");
  editor.setBase("a\n");
  expect(shown(editor)).toEqual(["added"]);
  // A commit moved the base to the file's text.
  editor.setBase("a\nb\n");
  expect(shown(editor)).toEqual([]);
  editor.setBase("x\nb\n");
  expect(shown(editor)).toEqual(["modified"]);
  // A new or untracked file has no base.
  editor.setBase(null);
  expect(shown(editor)).toEqual([]);
});

test("comparing with the disk hides the markers until it ends", () => {
  const editor = editing("a\nb\n");
  editor.setBase("a\n");
  editor.compare("a\nc\n");
  expect(shown(editor)).toEqual([]);
  // A new base meanwhile shows once the diff closes.
  editor.setBase("x\nb\n");
  expect(shown(editor)).toEqual([]);
  editor.compare(null);
  expect(shown(editor)).toEqual(["modified"]);
});
