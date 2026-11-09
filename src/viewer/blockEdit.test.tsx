import { afterEach, beforeEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { apply } from "../reduce";
import { initialState, setEdit, setOpenFile, useHive } from "../store";
import { blockSource, clickedBlock, useBlockEdit, withBlock } from "./blockEdit";
import { resolve, toText } from "./buffer";

beforeEach(() => window.getSelection()?.removeAllRanges());

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const doc = (text: string) => toText(text);

test("a block's source is its lines, whatever their ending", () => {
  const lf = doc("# T\n\none\ntwo\n\nend");
  expect(blockSource(lf, 3, 4)).toEqual({
    first: 3,
    last: 4,
    source: "one\ntwo",
    crlf: false,
    closed: false,
  });
  expect(blockSource(lf, 1, 1).source).toBe("# T");
  // A last line past the text is the text's last.
  expect(blockSource(lf, 6, 99)).toMatchObject({ last: 6, source: "end" });

  const crlf = doc("# T\r\n\r\none\r\ntwo\r\n\r\nend");
  expect(blockSource(crlf, 3, 4)).toEqual({
    first: 3,
    last: 4,
    source: "one\ntwo",
    crlf: true,
    closed: true,
  });
  expect(blockSource(crlf, 1, 1)).toMatchObject({ source: "# T", crlf: true, closed: true });
  // The file's last line has no line ending, but what comes before it says which.
  const last = doc("a\r\nb\r\nc");
  expect(blockSource(last, 2, 3)).toMatchObject({ source: "b\nc", crlf: true, closed: false });
  expect(blockSource(last, 3, 3)).toMatchObject({ source: "c", crlf: false, closed: false });
  // Mixed endings are shown as they are.
  expect(blockSource(doc("a\nb\r\nc\n"), 1, 2)).toMatchObject({ source: "a\nb\r", crlf: false });
});

test("writing a block back changes its lines and every other byte stays", () => {
  const edit = (text: string, first: number, last: number, draft: string) => {
    const before = doc(text);
    return withBlock(before, blockSource(before, first, last), draft).toString();
  };
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

const at = { worktree: "/w", path: "a.md" };

function answer(content: string) {
  apply({
    type: "file",
    ...at,
    content,
    base: content,
    version: "v",
    binary: false,
    too_large: false,
    error: null,
  });
}

/** The hook over two plain blocks; the buffer is what the view would pass as its text. */
function View({ text }: { text: string }) {
  const { shown, onClick, editor } = useBlockEdit(text);
  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: as RenderedView's
    // biome-ignore lint/a11y/noStaticElementInteractions: as RenderedView's
    <div onClick={onClick} data-testid="view" data-shown={shown}>
      <p data-line="1" data-line-end="1">
        one
      </p>
      <p data-line="3" data-line-end="4" data-testid="two">
        two
      </p>
      {editor}
    </div>
  );
}

const TEXT = "one\n\ntwo\nlines\n";
const box = () => screen.queryByRole("textbox") as HTMLTextAreaElement | null;
const two = () => screen.getByTestId("two");
const open = () => fireEvent.click(two());

function Live() {
  const doc = useHive((s) => s.edit?.doc);
  return <View text={doc ? doc.toString() : TEXT} />;
}

function start() {
  act(() => setOpenFile(at));
  act(() => answer(TEXT));
  render(<Live />);
}

const buffer = () => useHive.getState().edit;
const shown = () => screen.getByTestId("view").getAttribute("data-shown");

test("a block opens as its source, hidden, and the keystrokes reach the buffer at once", () => {
  start();
  expect(buffer()).toBeNull();
  open();
  // The buffer starts with the first block opened, as a click on Edit does.
  expect(useHive.getState().editing).toBe(true);
  expect(box()?.value).toBe("two\nlines");
  expect(box()?.getAttribute("aria-label")).toBe("Markdown of lines 3 to 4");
  expect(document.activeElement).toBe(box());
  expect(two().style.display).toBe("none");
  // The editor comes right after its block.
  expect(two().nextElementSibling?.contains(box())).toBe(true);

  fireEvent.input(box() as HTMLElement, { target: { value: "2" } });
  expect(buffer()?.doc.toString()).toBe("one\n\n2\n");
  // Rendered as it was until it closes.
  expect(shown()).toBe(TEXT);
  // Back to where it was: the buffer is clean again.
  fireEvent.input(box() as HTMLElement, { target: { value: "two\nlines" } });
  expect(buffer()?.doc.eq(toText(TEXT))).toBe(true);
});

test("Escape closes the block and renders the edit", () => {
  start();
  open();
  fireEvent.input(box() as HTMLElement, { target: { value: "2" } });
  fireEvent.keyDown(box() as HTMLElement, { key: "a" });
  expect(box()).not.toBeNull();
  fireEvent.keyDown(box() as HTMLElement, { key: "Escape" });
  expect(box()).toBeNull();
  expect(two().style.display).toBe("");
  expect(two().nextElementSibling).toBeNull();
  expect(shown()).toBe("one\n\n2\n");
  expect(buffer()?.doc.toString()).toBe("one\n\n2\n");
});

test("a click outside closes it; that click opens no other block, the next one does", () => {
  start();
  open();
  fireEvent.click(box() as HTMLElement);
  expect(box()).not.toBeNull();
  // On another block: only closes.
  fireEvent.click(screen.getByText("one"));
  expect(box()).toBeNull();
  fireEvent.click(screen.getByText("one"));
  expect(box()?.value).toBe("one");
  // Outside the view too.
  fireEvent.click(document.body);
  expect(box()).toBeNull();
});

test("a click on nothing that is a block opens nothing, a file with no text neither", () => {
  start();
  fireEvent.click(screen.getByTestId("view"));
  expect(box()).toBeNull();
  expect(buffer()).toBeNull();
  // Binary: no buffer to edit.
  act(() =>
    apply({
      type: "file",
      ...at,
      content: null,
      base: null,
      version: null,
      binary: true,
      too_large: false,
      error: null,
    }),
  );
  open();
  expect(box()).toBeNull();
});

test("a reload from disk closes the block: its lines are no longer the block's", () => {
  start();
  open();
  fireEvent.input(box() as HTMLElement, { target: { value: "2" } });
  act(() => answer("agent\n"));
  // Unsaved edits against a newer file: still open, the banner asks.
  expect(buffer()?.conflict).not.toBeNull();
  expect(box()).not.toBeNull();
  act(() => setEdit(resolve(buffer() as NonNullable<ReturnType<typeof buffer>>, false)));
  expect(box()).toBeNull();
  expect(shown()).toBe("agent\n");
});

test("an edit with the buffer gone is dropped", () => {
  start();
  open();
  act(() => useHive.setState({ edit: null }));
  // Closed by the buffer going away, as a closed tab does.
  expect(box()).toBeNull();
});
