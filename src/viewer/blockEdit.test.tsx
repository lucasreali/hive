import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { apply } from "../reduce";
import { RightPanel } from "../shell/RightPanel";
import { TerminalArea } from "../shell/TerminalArea";
import { initialState, select, setEdit, setOpenFile, setRendered, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { type BlockSource, blockSource, clickedBlock, hasLoneCr, withBlock } from "./blockEdit";
import { resolve, toText } from "./buffer";

beforeEach(() => window.getSelection()?.removeAllRanges());

afterEach(() => {
  mock.restore();
  cleanup();
  useHive.setState(initialState, true);
});

const doc = (text: string) => toText(text);
const source = (text: string, first: number, last: number) =>
  blockSource(doc(text), first, last) as BlockSource;

test("a block's source is its lines, whatever their ending", () => {
  const lf = "# T\n\none\ntwo\n\nend";
  expect(source(lf, 3, 4)).toEqual({
    first: 3,
    last: 4,
    source: "one\ntwo",
    crlf: false,
    closed: false,
  });
  expect(source(lf, 1, 1).source).toBe("# T");
  // A last line past the text is the text's last.
  expect(source(lf, 6, 99)).toMatchObject({ last: 6, source: "end" });

  const crlf = "# T\r\n\r\none\r\ntwo\r\n\r\nend";
  expect(source(crlf, 3, 4)).toEqual({
    first: 3,
    last: 4,
    source: "one\ntwo",
    crlf: true,
    closed: true,
  });
  expect(source(crlf, 1, 1)).toMatchObject({ source: "# T", crlf: true, closed: true });
  // The file's last line has no line ending, but what comes before it says which.
  const last = "a\r\nb\r\nc";
  expect(source(last, 2, 3)).toMatchObject({ source: "b\nc", crlf: true, closed: false });
  expect(source(last, 3, 3)).toMatchObject({ source: "c", crlf: true, closed: false });
  // Only the last line: another with no "\r" says what it says. Nor is there one before the first.
  expect(source("a\r\nb\nc\n", 2, 2)).toMatchObject({ source: "b", crlf: false });
  expect(source("c", 1, 1)).toMatchObject({ source: "c", crlf: false });
  expect(source("a\nb", 2, 2)).toMatchObject({ source: "b", crlf: false });
  // Mixed endings are shown as they are.
  expect(source("a\nb\r\nc\n", 1, 2)).toMatchObject({ source: "a\nb\r", crlf: false });
});

test("lines the text does not have are no block", () => {
  const text = doc("a\nb\n");
  expect(blockSource(text, 3, 3)).not.toBeNull();
  expect(blockSource(text, 0, 1)).toBeNull();
  expect(blockSource(text, 4, 4)).toBeNull();
  expect(blockSource(text, 2, 1)).toBeNull();
  expect(blockSource(text, Number.NaN, 1)).toBeNull();
  expect(blockSource(text, 1, Number.NaN)).toBeNull();
});

test("a \\r alone is told from a \\r\\n", () => {
  expect(hasLoneCr("a\r\nb\r\n")).toBe(false);
  expect(hasLoneCr("a\nb")).toBe(false);
  expect(hasLoneCr("a\rb\n")).toBe(true);
  expect(hasLoneCr("a\r")).toBe(true);
});

test("writing a block back changes its lines and every other byte stays", () => {
  const edit = (text: string, first: number, last: number, draft: string) =>
    withBlock(doc(text), source(text, first, last), draft).toString();
  const text = "# T  \n\n\none\ntwo\n\n\tend\n";
  expect(edit(text, 4, 5, "new")).toBe("# T  \n\n\nnew\n\n\tend\n");
  expect(edit(text, 4, 5, "one\ntwo")).toBe(text);
  expect(edit(text, 4, 5, "a\nb\nc")).toBe("# T  \n\n\na\nb\nc\n\n\tend\n");
  expect(edit(text, 4, 5, "")).toBe("# T  \n\n\n\n\n\tend\n");
  // CRLF: the lines written keep it, and typed ones get it.
  const crlf = "# T\r\n\r\none\r\ntwo\r\n\r\nend";
  expect(edit(crlf, 3, 4, "one\ntwo")).toBe(crlf);
  expect(edit(crlf, 3, 4, "a\nb\nc")).toBe("# T\r\n\r\na\r\nb\r\nc\r\n\r\nend");
  expect(edit("a\r\nb\r\nc", 2, 3, "x\ny")).toBe("a\r\nx\r\ny");
  expect(edit("a\r\nb", 2, 2, "x")).toBe("a\r\nx");
  // The file's last line alone keeps it too, and still ends with none.
  expect(edit("a\r\nb\r\nc", 3, 3, "x\ny")).toBe("a\r\nb\r\nx\r\ny");
  expect(edit("a\nb", 2, 2, "x\ny")).toBe("a\nx\ny");
});

test("a click opens a block, unless it is on a control or ends a selection", () => {
  document.body.innerHTML = `
    <div data-line="1" data-line-end="1" id="block">
      <span id="text">a</span>
      <a id="link" href="#">l</a><button id="button"></button>
      <details><summary id="summary"></summary></details><input id="input" />
    </div>
    <p id="other"></p>`;
  const at = (id: string) => document.getElementById(id) as Element;
  const block = at("block");
  // Compared here: a failure would print the whole document.
  const opens = (target: Element) => clickedBlock(target) === block;
  expect(opens(at("text"))).toBe(true);
  expect(opens(block)).toBe(true);
  for (const id of ["link", "button", "summary", "input", "other"]) {
    expect(clickedBlock(at(id)) === null).toBe(true);
  }
  // Text selected: the drag that made it ends in a click.
  const range = document.createRange();
  range.selectNodeContents(at("text"));
  window.getSelection()?.addRange(range);
  expect(clickedBlock(at("text")) === null).toBe(true);
  document.body.innerHTML = "";
});

const [, api] = MOCK_REPOS;
const refactor = api.worktrees[1];
const at = { worktree: refactor.path, path: "a.md" };
const TEXT = "one\n\ntwo\nlines\n";

function answer(content: string | null, patch = {}) {
  apply({
    type: "file",
    ...at,
    content,
    base: content,
    version: "v",
    binary: false,
    too_large: false,
    error: null,
    ...patch,
  });
}

/** The file rendered in its tab, as the app shows it. */
function preview(text = TEXT) {
  spyOn(transport, "listChanges").mockImplementation(async () => {});
  apply({ type: "projects", projects: MOCK_REPOS });
  render(
    <>
      <RightPanel />
      <TerminalArea />
    </>,
  );
  act(() => select(refactor.id));
  act(() => setOpenFile(at));
  act(() => answer(text));
  act(() => setRendered(true));
}

const body = () => document.querySelector(".file-view-body") as HTMLElement;
const box = () => body().querySelector<HTMLTextAreaElement>(".md-block-editor");
const para = (text: string) =>
  [...body().querySelectorAll("p")].find((p) => p.textContent?.includes(text)) as HTMLElement;
const rendered = () =>
  [...body().querySelectorAll(".markdown > p")]
    .filter((p) => (p as HTMLElement).style.display !== "none")
    .map((p) => p.textContent);
const buffer = () => useHive.getState().edit;

test("a block opens as its source, hidden, and the keystrokes reach the buffer at once", () => {
  preview();
  expect(buffer()).toBeNull();
  fireEvent.click(para("two"));
  // The buffer starts with the first block opened, as a click on Edit does.
  expect(useHive.getState().editing).toBe(true);
  expect(box()?.value).toBe("two\nlines");
  expect(box()?.getAttribute("aria-label")).toBe("Markdown of lines 3 to 4");
  expect(document.activeElement).toBe(box());
  expect(para("two").style.display).toBe("none");
  // The editor comes right after its block.
  expect(para("two").nextElementSibling?.contains(box())).toBe(true);

  fireEvent.input(box() as HTMLElement, { target: { value: "2" } });
  expect(buffer()?.doc.toString()).toBe("one\n\n2\n");
  // Rendered as it was until it closes.
  expect(para("two")).toBeDefined();
  // Back to where it was: the buffer is clean again.
  fireEvent.input(box() as HTMLElement, { target: { value: "two\nlines" } });
  expect(buffer()?.doc.eq(toText(TEXT))).toBe(true);
});

test("Escape closes the block and renders the edit", () => {
  preview();
  fireEvent.click(para("two"));
  fireEvent.input(box() as HTMLElement, { target: { value: "2" } });
  fireEvent.keyDown(box() as HTMLElement, { key: "a" });
  expect(box()).not.toBeNull();
  fireEvent.keyDown(box() as HTMLElement, { key: "Escape" });
  expect(box()).toBeNull();
  expect(rendered()).toEqual(["one", "2"]);
  expect(body().querySelectorAll(".markdown > div")).toHaveLength(0);
  expect(buffer()?.doc.toString()).toBe("one\n\n2\n");
});

test("a click outside closes it and renders the edit; it opens no other block, the next does", () => {
  preview();
  fireEvent.click(para("two"));
  fireEvent.click(box() as HTMLElement);
  expect(box()).not.toBeNull();
  fireEvent.input(box() as HTMLElement, { target: { value: "edited" } });
  // On another block: only closes.
  fireEvent.click(para("one"));
  expect(box()).toBeNull();
  expect(rendered()).toEqual(["one", "edited"]);
  expect(buffer()?.doc.toString()).toBe("one\n\nedited\n");
  fireEvent.click(para("one"));
  expect(box()?.value).toBe("one");
  // Outside the view too.
  fireEvent.input(box() as HTMLElement, { target: { value: "first" } });
  fireEvent.click(document.body);
  expect(box()).toBeNull();
  expect(rendered()).toEqual(["first", "edited"]);
});

test("a click beside the blocks opens nothing and starts no buffer", () => {
  preview();
  fireEvent.click(body().querySelector(".markdown-view") as HTMLElement);
  expect(box()).toBeNull();
  expect(buffer()).toBeNull();
});

test("a reload from disk closes the block: its lines are no longer the block's", () => {
  preview();
  fireEvent.click(para("two"));
  fireEvent.input(box() as HTMLElement, { target: { value: "2" } });
  act(() => answer("agent\n"));
  // Unsaved edits against a newer file: still open, the banner asks.
  expect(buffer()?.conflict).not.toBeNull();
  expect(box()).not.toBeNull();
  act(() => setEdit(resolve(buffer() as NonNullable<ReturnType<typeof buffer>>, false)));
  expect(box()).toBeNull();
  expect(rendered()).toEqual(["agent"]);
});

test("a keystroke that finds another buffer, or none, changes nothing", () => {
  preview();
  fireEvent.click(para("two"));
  const open = box() as HTMLElement;
  // Before React has rendered it, as an input right after a reload would.
  const reloaded = toText("agent\n");
  const other = { ...(buffer() as NonNullable<ReturnType<typeof buffer>>), doc: reloaded };
  useHive.setState({ edit: other });
  fireEvent.input(open, { target: { value: "lost" } });
  expect(buffer()?.doc).toBe(reloaded);
  useHive.setState({ edit: null });
  act(() => {});
  expect(buffer()).toBeNull();
  expect(box()).toBeNull();
});

test("the same with no buffer left", () => {
  preview();
  fireEvent.click(para("two"));
  const open = box() as HTMLElement;
  useHive.setState({ edit: null });
  fireEvent.input(open, { target: { value: "lost" } });
  expect(buffer()).toBeNull();
});

test("a \\r alone: its lines are not the buffer's, so a click opens nothing and says why", () => {
  // The parser's line 5 is the buffer's line 4: the wrong block.
  preview("x\ry\n\nTARGET\n\nOTHER\n");
  fireEvent.click(para("TARGET"));
  expect(box()).toBeNull();
  expect(para("TARGET").style.display).toBe("");
  expect(buffer()).toBeNull();
  expect(useHive.getState().notices.at(-1)?.text).toContain("edit it in the text");
});

test("a \\r alone: the parser's lines past the buffer's end throw nothing, hide nothing", () => {
  // Its line 5; the buffer has two.
  preview("a\rb\r\rc\n");
  fireEvent.click(para("c"));
  expect(box()).toBeNull();
  expect(para("c").style.display).toBe("");
  expect(buffer()).toBeNull();
});
