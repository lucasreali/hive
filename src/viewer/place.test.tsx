import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { EditorView } from "@codemirror/view";
import { act, cleanup, render } from "@testing-library/react";
import { apply } from "../reduce";
import { TerminalArea } from "../shell/TerminalArea";
import {
  initialState,
  type OpenFile,
  pinFile,
  select,
  setEditing,
  setOpenFile,
  setRendered,
  useHive,
} from "../store";
import { MOCK_REPOS } from "../transport/mock";
import { snapshot } from "./editor";
import { type Block, blocksOf, lineCount, lineForScroll, scrollForLine } from "./place";

// 21.3: the same place in a Markdown file's text and in its rendered view.

// A heading (line 1), a list (3-5), a code block (7-10) and a table (12-14), in a 14-line text.
const BLOCKS: Block[] = [
  { line: 1, top: 0, height: 40 },
  { line: 3, top: 60, height: 60 },
  { line: 7, top: 140, height: 100 },
  { line: 12, top: 260, height: 90 },
];

test("a line maps to the block holding it, proportionally by line", () => {
  // The heading, its first line, and the blank line after it.
  expect(scrollForLine(BLOCKS, 1, 14)).toBe(0);
  expect(scrollForLine(BLOCKS, 2, 14)).toBe(20);
  // The list: three items and the blank line, 4 lines in 60 px.
  expect(scrollForLine(BLOCKS, 4, 14)).toBe(75);
  // Halfway into the code block's second line.
  expect(scrollForLine(BLOCKS, 8.5, 14)).toBe(170);
  // The table, the last block, runs to the end of the text.
  expect(scrollForLine(BLOCKS, 13, 14)).toBe(290);
  expect(scrollForLine(BLOCKS, 99, 14)).toBe(350);
});

test("a scroll maps to the line of the top visible block, and back", () => {
  expect(lineForScroll(BLOCKS, 0, 14)).toBe(1);
  expect(lineForScroll(BLOCKS, 75, 14)).toBe(4);
  expect(lineForScroll(BLOCKS, 170, 14)).toBe(8.5);
  expect(lineForScroll(BLOCKS, 290, 14)).toBe(13);
  // In the margin under the list: its end, the code block's line.
  expect(lineForScroll(BLOCKS, 130, 14)).toBe(7);
  // A pixel short of a block's top (rounding) is still its top.
  expect(lineForScroll(BLOCKS, 259.5, 14)).toBe(12);
  // Scrolled past every block: the end of the last.
  expect(lineForScroll(BLOCKS, 400, 14)).toBe(15);
  for (const line of [1, 3, 5, 7, 9.25, 12, 14])
    expect(lineForScroll(BLOCKS, scrollForLine(BLOCKS, line, 14), 14)).toBe(line);
});

test("a text with no blocks is the top; a place above the first block is that block", () => {
  expect(scrollForLine([], 30, 40)).toBe(0);
  expect(lineForScroll([], 300, 40)).toBe(1);
  const later: Block[] = [{ line: 5, top: 10, height: 0 }];
  expect(scrollForLine(later, 2, 9)).toBe(0);
  // Above the first block, that block is the top one.
  expect(lineForScroll(later, 0, 9)).toBe(5);
  // A block with no height is its first line.
  expect(lineForScroll(later, 50, 9)).toBe(5);
  expect(lineCount("a\nb\n")).toBe(3);
});

// Each rendered block is laid out 10 px per line from its first line; the scroller at 100.
const LINE = 10;
const measure = HTMLElement.prototype.getBoundingClientRect;
function layout() {
  return spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    const scroller = this.closest(".markdown-view") as HTMLElement | null;
    // Anything else (the editor) as happy-dom measures it.
    if (!scroller) return measure.call(this);
    const line = Number(this.dataset.line);
    if (!this.dataset.line) return { top: 100, height: 400 } as DOMRect;
    return { top: 100 - scroller.scrollTop + (line - 1) * LINE, height: 2 * LINE } as DOMRect;
  });
}

test("blocks are measured in the scroller's content, whatever its scroll", () => {
  const rect = layout();
  const { container } = render(
    <div className="markdown-view">
      <div className="markdown">
        <h1 data-line="1">a</h1>
        <p>no line</p>
        <p data-line="4">b</p>
      </div>
    </div>,
  );
  const scroller = container.firstChild as HTMLElement;
  scroller.scrollTop = 25;
  expect(blocksOf(scroller)).toEqual([
    { line: 1, top: 0, height: 20 },
    { line: 4, top: 30, height: 20 },
  ]);
  rect.mockRestore();
  cleanup();
});

// The view itself, in the terminal area as the user sees it.
const [shop] = MOCK_REPOS;
const [, fixLogin] = shop.worktrees;
const W = fixLogin.path;
// 100 sections of 4 lines: a heading on line 4k+1, a paragraph on 4k+3.
const DOC = Array.from({ length: 100 }, (_, k) => `## Heading ${k}\n\ntext ${k}\n`).join("\n");
const file = (path: string): OpenFile => ({ worktree: W, path });
const answer = (path: string, content = DOC) =>
  apply({
    type: "file",
    worktree: W,
    path,
    content,
    base: content,
    version: `v:${content}`,
    binary: false,
    too_large: false,
    error: null,
  });
function open(path: string) {
  setOpenFile(file(path), false);
  pinFile(file(path));
  answer(path);
}
const scroller = () => document.querySelector(".markdown-view") as HTMLElement;
function scrollRendered(top: number) {
  scroller().scrollTop = top;
  scroller().dispatchEvent(new Event("scroll"));
}
const text = () =>
  EditorView.findFromDOM(
    document.querySelector(".file-view .cm-editor") as HTMLElement,
  ) as EditorView;
const textTop = () =>
  text().state.doc.lineAt((snapshot(text()).scroll.value as { range: { head: number } }).range.head)
    .number;

let rect: ReturnType<typeof layout>;
beforeEach(() => {
  useHive.setState(initialState, true);
  apply({ type: "projects", projects: [shop] });
  select(fixLogin.id);
  rect = layout();
});
afterEach(() => {
  cleanup();
  rect.mockRestore();
  useHive.setState(initialState, true);
});

test("the rendered view keeps its scroll per file, another file and back", () => {
  render(<TerminalArea />);
  act(() => open("a.md"));
  act(() => setRendered(true));
  expect(scroller().scrollTop).toBe(0);
  act(() => scrollRendered(400));
  act(() => open("b.md"));
  act(() => setRendered(true));
  expect(scroller().scrollTop).toBe(0);
  act(() => setOpenFile(file("a.md"), false));
  // Once its text is there to scroll in.
  act(() => answer("a.md"));
  expect(scroller().scrollTop).toBe(400);
  // The other one kept its own.
  act(() => setOpenFile(file("b.md"), false));
  act(() => answer("b.md"));
  expect(scroller().scrollTop).toBe(0);
});

test("switching between text and rendered keeps the same line at the top", () => {
  render(<TerminalArea />);
  act(() => open("a.md"));
  act(() => setEditing(true));
  // The text scrolled down, as the user does.
  act(() => {
    const view = text();
    view.scrollDOM.scrollTop = 3000;
    view.scrollDOM.dispatchEvent(new Event("scroll"));
  });
  const top = snapshot(text()).top;
  expect(top).toBeGreaterThan(100);
  act(() => setRendered(true));
  // Rendered: that line on top (this layout puts line n at (n - 1) * LINE).
  expect(scroller().scrollTop).toBeCloseTo((top - 1) * LINE);
  act(() => setRendered(false));
  expect(textTop()).toBe(Math.floor(top));
  // Scrolled in the rendered view to the paragraph of line 83, then the text.
  act(() => setRendered(true));
  act(() => scrollRendered(82 * LINE));
  act(() => setRendered(false));
  expect(textTop()).toBe(83);
  // That place is the one kept: rendered again, then the text again, without a scroll.
  act(() => setRendered(true));
  expect(scroller().scrollTop).toBe(82 * LINE);
  act(() => setRendered(false));
  expect(textTop()).toBe(83);
});
