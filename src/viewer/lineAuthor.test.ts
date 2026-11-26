import { afterEach, expect, spyOn, test } from "bun:test";
import { type BlockInfo, BlockType, type EditorView } from "@codemirror/view";
import { initialState, useHive } from "../store";
import { createViewer, type Viewer } from "./editor";
import { cursorAsk, lineAtHeight, lineMenu, trackView } from "./lineAuthor";

let viewer: Viewer | null = null;
afterEach(() => {
  viewer?.destroy();
  viewer = null;
  document.body.innerHTML = "";
  useHive.setState(initialState, true);
});

const file = { worktree: "/r", path: "a.ts" };

/** A block of the view's height map, as CodeMirror gives it. */
const block = (type: BlockType, from: number, top: number, height: number) =>
  ({ type, from, top, height, bottom: top + height }) as unknown as BlockInfo;
/** A line made of `parts` (a widget above its text). */
const line = (...parts: BlockInfo[]) => ({ ...parts[0], type: parts }) as unknown as BlockInfo;

/** A unified diff of `content` against `original`, its height map answering with `at`. */
function diff(content: string, original: string, at: BlockInfo, oldSide = true) {
  viewer = createViewer(document.body, "a.ts", () => {}, lineMenu(file, oldSide));
  viewer.show({ content, original });
  spyOn(viewer.view, "lineBlockAtHeight").mockReturnValue(at);
  return viewer.view;
}

test("a text line counts on the worktree side, a removed one on the base side", () => {
  // "b" became "B": its removed line is a widget above the new line 2 (offset 2).
  const at = line(block(BlockType.WidgetBefore, 2, 14, 14), block(BlockType.Text, 2, 28, 14));
  const view = diff("a\nB\nc\n", "a\nb\nc\n", at);
  expect(lineAtHeight(view, 20, true)).toEqual({ line: 2, side: "base" });
  expect(lineAtHeight(view, 30, true)).toEqual({ line: 2, side: "worktree" });
  // Past the line's parts: its last.
  expect(lineAtHeight(view, 99, true)).toEqual({ line: 2, side: "worktree" });
  // "View diff" of the editor compares with the disk: its removed lines are not asked about.
  expect(lineAtHeight(view, 20, false)).toBeNull();
});

test("a removed chunk's lines are told apart by height; a widget of no chunk is nothing", () => {
  // Lines "b" and "c" removed: a widget of two lines above "d" (offset 2).
  const view = diff("a\nd\n", "a\nb\nc\nd\n", block(BlockType.WidgetBefore, 2, 14, 28));
  expect(lineAtHeight(view, 14, true)).toEqual({ line: 2, side: "base" });
  expect(lineAtHeight(view, 35, true)).toEqual({ line: 3, side: "base" });
  expect(lineAtHeight(view, 41.9, true)).toEqual({ line: 3, side: "base" });
  const plain = viewer as Viewer;
  spyOn(plain.view, "lineBlockAtHeight").mockReturnValue(block(BlockType.WidgetBefore, 0, 0, 14));
  expect(lineAtHeight(plain.view, 5, true)).toBeNull();
  spyOn(plain.view, "lineBlockAtHeight").mockReturnValue(block(BlockType.WidgetAfter, 2, 0, 14));
  expect(lineAtHeight(plain.view, 5, true)).toBeNull();
});

test("without a diff, a widget is nothing", () => {
  viewer = createViewer(document.body, "a.ts");
  viewer.show({ content: "a\n", original: null });
  spyOn(viewer.view, "lineBlockAtHeight").mockReturnValue(block(BlockType.WidgetBefore, 0, 0, 9));
  expect(lineAtHeight(viewer.view, 5, true)).toBeNull();
  spyOn(viewer.view, "lineBlockAtHeight").mockReturnValue(block(BlockType.Text, 2, 0, 9));
  expect(lineAtHeight(viewer.view, 5, true)).toEqual({ line: 2, side: "worktree" });
});

/** A right click at `y` on `target`; whether it was taken from the native menu. */
function rightClick(target: Element, y: number) {
  const event = new MouseEvent("contextmenu", {
    bubbles: true,
    cancelable: true,
    clientX: 7,
    clientY: y,
  });
  target.dispatchEvent(event);
  return event.defaultPrevented;
}

test("a right click on a line number opens the menu there; the text keeps its native menu", () => {
  const view = diff("a\nB\nc\n", "a\nb\nc\n", block(BlockType.Text, 2, 14, 14));
  const gutter = view.dom.querySelector(".cm-gutters") as Element;
  expect(rightClick(view.contentDOM, 20)).toBe(false);
  expect(useHive.getState().lineMenu).toBeNull();

  useHive.setState({
    provenance: { ...file, line: 1, side: "worktree", x: 0, y: 0, origin: null },
  });
  expect(rightClick(gutter, 20)).toBe(true);
  expect(useHive.getState().lineMenu).toEqual({ ...file, line: 2, side: "worktree", x: 7, y: 20 });
  // A new question replaces the last answer.
  expect(useHive.getState().provenance).toBeNull();

  // No line there (a widget of no chunk): nothing.
  useHive.setState({ lineMenu: null });
  spyOn(view, "lineBlockAtHeight").mockReturnValue(block(BlockType.WidgetAfter, 0, 0, 14));
  expect(rightClick(gutter, 5)).toBe(false);
  expect(useHive.getState().lineMenu).toBeNull();

  // Gone with its view.
  (viewer as Viewer).destroy();
  viewer = null;
  expect(rightClick(gutter, 20)).toBe(false);
});

test("the palette asks about the cursor's line of a tracked view", () => {
  expect(cursorAsk(file)).toBeNull();
  viewer = createViewer(document.body, "a.ts");
  viewer.show({ content: "a\nb\nc\n", original: null });
  const view: EditorView = viewer.view;
  const untrack = trackView(file, view);
  view.dispatch({ selection: { anchor: 3 } });
  const coords = spyOn(view, "coordsAtPos").mockReturnValue(null);
  expect(cursorAsk(file)).toEqual({ ...file, line: 2, side: "worktree", x: 0, y: 0 });
  coords.mockReturnValue({ left: 40, right: 41, top: 30, bottom: 48 });
  expect(cursorAsk(file)).toEqual({ ...file, line: 2, side: "worktree", x: 40, y: 48 });
  // Another view of the same file took its place: letting the first go keeps it.
  const other = createViewer(document.body, "a.ts");
  other.show({ content: "z\n", original: null });
  const untrackOther = trackView(file, other.view);
  untrack();
  expect(cursorAsk(file)?.line).toBe(1);
  untrackOther();
  expect(cursorAsk(file)).toBeNull();
  other.destroy();
});
