import { afterEach, expect, spyOn, test } from "bun:test";
import { undo } from "@codemirror/commands";
import { Chunk } from "@codemirror/merge";
import { toText } from "./buffer";
import { createEditor, type Editor } from "./editor";
import { lineDiff, MARKER_DIFF, markerLines } from "./gutterMarkers";

const marks = (base: string, text: string) =>
  markerLines(Chunk.build(toText(base), toText(text), MARKER_DIFF), toText(text));

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

// 25.7: the markers diff whole lines, so a large file of long lines opens at once.

const changes = (a: string, b: string) =>
  lineDiff(a, b).map((c) => [c.fromA, c.toA, c.fromB, c.toB]);

test("lineDiff gives the changed whole lines as offsets into the texts", () => {
  expect(changes("", "")).toEqual([]);
  expect(changes("a\nb\n", "a\nb\n")).toEqual([]);
  expect(changes("", "a\n")).toEqual([[0, 0, 0, 2]]);
  expect(changes("a\n", "")).toEqual([[0, 2, 0, 0]]);
  // A line changed by one character is the whole line, on both sides.
  expect(changes("one\ntwo\nthree\n", "one\ntwO\nthree\n")).toEqual([[4, 8, 4, 8]]);
  // Two far apart, at their own offsets: the second moved by the first's growth.
  expect(changes("a\nb\nc\nd\n", "a\nBBB\nc\nD\n")).toEqual([
    [2, 4, 2, 6],
    [6, 8, 8, 10],
  ]);
  // A last line without a newline differs from the same text with one.
  expect(changes("a\nb", "a\nb\n")).toEqual([[2, 3, 2, 4]]);
});

/** `lines` long lines of `size` characters, each different from the others throughout. */
const longLines = (lines: number, size: number, tag: string) =>
  Array.from({ length: lines }, (_, i) => {
    const words = Array.from({ length: Math.ceil(size / 4) }, (_, w) => `${tag}${i}w${w}`);
    return words.join(" ").slice(0, size);
  });

test("a 250 KB file of 5 KB lines gets its exact markers, without a diff of its characters", () => {
  const before = longLines(50, 5000, "x");
  const after = [...before];
  // A task ticked, four rewritten and two new: the characters' diff took 10 s on these (25.7).
  after[10] = (after[10] as string).replace("x10w5", "x10w5!");
  for (const i of [15, 25, 35, 45]) after[i] = longLines(1, 5000, `n${i}`)[0] as string;
  after.splice(30, 0, ...longLines(2, 5000, "new"));
  const [base, text] = [`${before.join("\n")}\n`, `${after.join("\n")}\n`];
  expect(base.length).toBeGreaterThan(250_000);

  const started = performance.now();
  expect(marks(base, text)).toEqual([
    { line: 11, kind: "modified" },
    { line: 16, kind: "modified" },
    { line: 26, kind: "modified" },
    { line: 31, kind: "added" },
    { line: 32, kind: "added" },
    { line: 38, kind: "modified" },
    { line: 48, kind: "modified" },
  ]);
  // Far above what it takes (under 1 ms, so load cannot reach it), far below the characters' 10 s.
  expect(performance.now() - started).toBeLessThan(2000);
});

test("the editor's markers diff by lines, when built and when typed", () => {
  const build = spyOn(Chunk, "build");
  const update = spyOn(Chunk, "updateB");
  const editor = editing("a\nb\n");
  editor.setBase("a\n");
  expect(build.mock.lastCall?.[2]).toBe(MARKER_DIFF);
  editor.view.dispatch({ changes: { from: 0, insert: "x" } });
  expect(update.mock.lastCall?.[4]).toBe(MARKER_DIFF);
  expect(shown(editor)).toEqual(["modified", "modified"]);
  build.mockRestore();
  update.mockRestore();
});

test("every line changed marks them all", () => {
  const count = 3000;
  const base = `${longLines(count, 20, "a").join("\n")}\n`;
  const text = `${longLines(count, 20, "b").join("\n")}\n`;
  const all = marks(base, text);
  expect(all).toHaveLength(count);
  expect(all.every((m) => m.kind === "modified")).toBe(true);
});

test("more distinct lines than ids fall back to the characters, and stay right", () => {
  const lines = Array.from({ length: 0xd800 + 10 }, (_, i) => `line ${i}`);
  const base = `${lines.join("\n")}\n`;
  const changed = [...lines];
  changed[20] = "line twenty";
  expect(marks(base, `${changed.join("\n")}\n`)).toEqual([{ line: 21, kind: "modified" }]);
  // Exactly as many as there are ids, the new line among them, still goes by lines (whole-line
  // offsets).
  const fit = `${lines.slice(0, 0xd800 - 1).join("\n")}\n`;
  const at = fit.indexOf("line 20\n");
  expect(changes(fit, fit.replace("line 20\n", "line 2O\n"))).toEqual([[at, at + 8, at, at + 8]]);
});
