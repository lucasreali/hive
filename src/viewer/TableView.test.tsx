import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { SearchQuery } from "@codemirror/search";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { asMac } from "../../test/mac";
import type { Sheet, TableText } from "../protocol";
import { columns, findCells, findText, rowsNote, TableView } from "./TableView";

beforeAll(() => {
  // happy-dom has no layout: give the scroller a height so the virtualizer shows rows.
  const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight")?.get;
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("table-view") ? 240 : original?.call(this);
    },
  });
  // And its content's height, the grid's, so it can be scrolled to a row (24.12).
  const content = Object.getOwnPropertyDescriptor(Element.prototype, "scrollHeight")?.get;
  Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
    configurable: true,
    get(this: HTMLElement) {
      const grid = this.querySelector<HTMLElement>(".table-grid");
      return this.classList.contains("table-view") && grid
        ? Number.parseFloat(grid.style.height)
        : content?.call(this);
    },
  });
});

afterEach(cleanup);

const sheet = (rows: string[][], patch: Partial<Sheet> = {}): Sheet => ({
  name: "",
  rows,
  total: rows.length,
  partial: false,
  ...patch,
});

const table = (sheets: Sheet[], error: string | null = null): TableText => ({
  worktree: "/w",
  path: "data/a.csv",
  sheets,
  error,
});

/** Each rendered row's cells, the row number first. */
const cells = () =>
  [...document.querySelectorAll("tr")].map((r) =>
    [...r.querySelectorAll("th, td")].map((c) => c.textContent),
  );

const many = (n: number) => [
  ["id", "name"],
  ...Array.from({ length: n }, (_, i) => [String(i + 1), `n${i + 1}`]),
];

test("the note says how many rows are shown only when some are not", () => {
  expect(rowsNote(sheet(many(3)))).toBeNull();
  expect(rowsNote(sheet(many(9_999), { total: 25_000 }))).toBe("Showing 10,000 of 25,000 rows");
  const partial = sheet(many(9_999), { total: 1_234_567, partial: true });
  expect(rowsNote(partial)).toBe("Showing 10,000 of more than 1,234,567 rows");
  // Every row sent, but the file was counted only so far.
  expect(rowsNote(sheet(many(1), { partial: true }))).toBe("Showing 2 of more than 2 rows");
});

test("each column is as wide as its longest cell, within bounds, after the row numbers", () => {
  const cell = (w: number) => `calc(${w}ch + 17px)`;
  expect(columns([["a", "bbbb"], ["cc"], ["", "", "x".repeat(99)]])).toBe(
    [cell(1), cell(3), cell(4), cell(40)].join(" "),
  );
  expect(columns(many(99))).toBe([cell(3), cell(3), cell(4)].join(" "));
  expect(columns([])).toBe(cell(1));
});

test("the first row is the header and every row has the longest row's cells", () => {
  render(<TableView table={table([sheet([["a", "b"], ["1"], ["1", "2", "3"]])])} />);
  const grid = screen.getByRole("table", { name: "data/a.csv" });
  expect(grid.getAttribute("aria-rowcount")).toBe("3");
  expect(grid.getAttribute("aria-colcount")).toBe("3");
  expect(screen.getAllByRole("columnheader").map((c) => c.textContent)).toEqual(["a", "b", ""]);
  expect(cells()).toEqual([
    ["1", "a", "b", ""],
    ["2", "1", "", ""],
    ["3", "1", "2", "3"],
  ]);
  const rows = [...document.querySelectorAll("tr")];
  expect(rows.map((r) => r.getAttribute("aria-rowindex"))).toEqual(["1", "2", "3"]);
  // Below the header, each in its place.
  expect(rows.map((r) => r.style.transform)).toEqual(["", "translateY(24px)", "translateY(48px)"]);
  // A cell's whole text is its title; an empty one has none.
  const [a, , none] = screen.getAllByRole("columnheader");
  expect([a?.title, none?.getAttribute("title")]).toEqual(["a", null]);
  expect(document.querySelector(".table-note")).toBeNull();
});

test("only the rows in view are rendered, and the note tells what is left out", () => {
  render(<TableView table={table([sheet(many(9_999), { total: 20_000 })])} />);
  const shown = cells();
  // The header, then the rows that fit in 240 px and the overscan.
  expect(shown[0]).toEqual(["1", "id", "name"]);
  expect(shown[1]).toEqual(["2", "1", "n1"]);
  expect(shown.length).toBeGreaterThan(5);
  expect(shown.length).toBeLessThan(40);
  const grid = screen.getByRole("table");
  expect(grid.style.height).toBe(`${24 * 10_000}px`);
  expect(grid.getAttribute("aria-rowcount")).toBe("20000");
  expect(document.querySelector(".table-note")?.textContent).toBe("Showing 10,000 of 20,000 rows");
});

test("an error, an empty sheet and no sheet say so", () => {
  const { rerender } = render(<TableView table={table([], "a.csv does not exist")} />);
  expect(document.querySelector(".hint")?.textContent).toBe("a.csv does not exist");
  rerender(<TableView table={table([sheet([])])} />);
  expect(document.querySelector(".hint")?.textContent).toBe("No rows.");
  rerender(<TableView table={table([])} />);
  expect(document.querySelector(".hint")?.textContent).toBe("No sheets.");
  expect(screen.queryByRole("tablist")).toBeNull();
});

test("a workbook's sheets each have a tab, the first shown", () => {
  const books = table([
    sheet([["first"]], { name: "Sales" }),
    sheet([["second"], ["x"]], { name: "Costs" }),
  ]);
  const { rerender } = render(<TableView table={books} />);
  const tabs = screen.getAllByRole("tab");
  expect(tabs.map((t) => [t.textContent, t.getAttribute("aria-selected")])).toEqual([
    ["Sales", "true"],
    ["Costs", "false"],
  ]);
  expect(cells()).toEqual([["1", "first"]]);
  fireEvent.click(tabs[1] as HTMLElement);
  expect(screen.getByRole("tab", { name: "Costs" }).getAttribute("aria-selected")).toBe("true");
  expect(cells()).toEqual([
    ["1", "second"],
    ["2", "x"],
  ]);
  // A new answer with fewer sheets shows the last one left.
  rerender(<TableView table={table([sheet([["only"]], { name: "Sales" })])} />);
  expect(cells()).toEqual([["1", "only"]]);
});

const query = (
  search: string,
  options: Partial<ConstructorParameters<typeof SearchQuery>[0]> = {},
) => new SearchQuery({ search, ...options });

test("a search finds the cells holding a match, the header's too, as the editor's does", () => {
  const rows = [
    ["Name", "Note"],
    ["foo bar", "Foobar"],
    ["a\tb", "food"],
  ];
  expect(findCells(rows, query("foo"))).toEqual([
    [1, 0],
    [1, 1],
    [2, 1],
  ]);
  expect(findCells(rows, query("foo", { caseSensitive: true }))).toEqual([
    [1, 0],
    [2, 1],
  ]);
  expect(findCells(rows, query("foo", { wholeWord: true }))).toEqual([[1, 0]]);
  // A word further in, after a match that is not a whole word.
  expect(findCells([["foobar foo"]], query("foo", { wholeWord: true }))).toEqual([[0, 0]]);
  expect(findCells(rows, query("^n", { regexp: true }))).toEqual([
    [0, 0],
    [0, 1],
  ]);
  expect(findCells(rows, query("o{2}d$", { regexp: true, caseSensitive: true }))).toEqual([[2, 1]]);
  // Plain text is not a regexp, but `\t` is a tab and `\\` a backslash, as in the editor.
  expect(findCells(rows, query("a.b"))).toEqual([]);
  expect(findCells(rows, query("a\\tb"))).toEqual([[2, 0]]);
  expect(findCells([["a\\b"]], query("a\\\\b"))).toEqual([[0, 0]]);
  expect(findCells([["a\\tb"]], query("a\\tb", { literal: true }))).toEqual([[0, 0]]);
  // An empty match never counts; one later in the cell does.
  expect(findCells([["x"], ["boo"]], query("o*", { regexp: true }))).toEqual([[1, 0]]);
  expect(findCells(rows, query("("))).toEqual([]);
  expect(findCells(rows, query("(", { regexp: true }))).toEqual([]);
  expect(findCells(rows, query(""))).toEqual([]);
});

test("the count says which match is current and what was searched", () => {
  const all = sheet(many(3));
  expect(findText(0, 0, all)).toBe("No results");
  expect(findText(-1, 1, all)).toBe("1 match");
  expect(findText(-1, 1_041, all)).toBe("1,041 matches");
  expect(findText(2, 41, all)).toBe("3 of 41");
  const partial = sheet(many(9_999), { total: 20_000 });
  expect(findText(2, 41, partial)).toBe("3 of 41 in the first 10,000 rows");
  expect(findText(0, 0, sheet(many(1), { partial: true }))).toBe("No results in the first 2 rows");
});

const press = (target: Element, init: KeyboardEventInit) =>
  act(() => fireEvent.keyDown(target, init));
const findField = () => screen.getByRole("textbox", { name: "Find" }) as HTMLInputElement;
const typed = (text: string) =>
  act(() => fireEvent.change(findField(), { target: { value: text } }));
const click = (name: string) => act(() => fireEvent.click(screen.getByRole("button", { name })));
const status = () => document.querySelector(".find-status")?.textContent ?? null;
const scroller = () => document.querySelector(".table-view") as HTMLElement;
/** The cells with class `name`: their row's place and text. */
const marked = (name: string) =>
  [...document.querySelectorAll(`.${name}`)].map((c) => [
    c.closest("tr")?.getAttribute("aria-rowindex"),
    c.textContent,
  ]);

test("⌘F from the table opens the find bar over its cells; Escape gives the table back", () => {
  const rows = [
    ["id", "name"],
    ["1", "ann"],
    ["2", "bob"],
    ["3", "Anna"],
  ];
  render(<TableView table={table([sheet(rows)])} />);
  // Another key, or ⌘⇧F, opens nothing.
  press(scroller(), { key: "g", ctrlKey: true });
  press(scroller(), { key: "F", ctrlKey: true, shiftKey: true });
  expect(screen.queryByRole("search")).toBeNull();
  press(scroller(), { key: "f", ctrlKey: true });
  const bar = screen.getByRole("search", { name: "Find in the table" });
  expect([...bar.classList]).toEqual(["table-find", "find-bar"]);
  expect(document.activeElement).toBe(findField());
  // The text's bar, with no replace row: the table is read-only.
  expect(screen.queryByRole("textbox", { name: "Replace" })).toBeNull();
  expect(status()).toBeNull();

  typed("an");
  expect(marked("table-match")).toEqual([
    ["2", "ann"],
    ["4", "Anna"],
  ]);
  expect(status()).toBe("2 matches");
  click("Match case");
  expect(marked("table-match")).toEqual([["2", "ann"]]);
  expect(status()).toBe("1 match");
  click("Match case");
  click("Match whole word");
  expect(status()).toBe("No results");
  click("Match whole word");
  click("Use regular expression");
  typed("^(id|bob)$");
  expect(marked("table-match")).toEqual([
    ["1", "id"],
    ["3", "bob"],
  ]);

  // ⌘F again, in the bar, selects its text to type a new search.
  findField().setSelectionRange(0, 0);
  press(findField(), { key: "f", ctrlKey: true });
  expect([findField().selectionStart, findField().selectionEnd]).toEqual([0, 10]);
  press(findField(), { key: "a" });
  expect(screen.getByRole("search")).toBeTruthy();
  press(findField(), { key: "Escape" });
  expect(screen.queryByRole("search")).toBeNull();
  expect(document.querySelector(".table-match")).toBeNull();
  expect(document.activeElement).toBe(scroller());
  // Opened again, it keeps its search.
  press(scroller(), { key: "f", ctrlKey: true });
  expect(findField().value).toBe("^(id|bob)$");
  click("Close search");
  expect(screen.queryByRole("search")).toBeNull();
  expect(document.activeElement).toBe(scroller());
});

test("⌘F is the Mac's command key there", () => {
  asMac();
  render(<TableView table={table([sheet([["a"]])])} />);
  press(scroller(), { key: "f", ctrlKey: true });
  expect(screen.queryByRole("search")).toBeNull();
  press(scroller(), { key: "f", metaKey: true });
  expect(screen.getByRole("search")).toBeTruthy();
});

test("next and previous go through the matches and wrap, the current one marked and shown", () => {
  render(<TableView table={table([sheet(many(500))])} />);
  press(scroller(), { key: "f", ctrlKey: true });
  typed("n");
  // The header's "name" and every row's name.
  expect(status()).toBe("501 matches");
  press(findField(), { key: "Enter" });
  expect(status()).toBe("1 of 501");
  expect(marked("table-match-current")).toEqual([["1", "name"]]);
  press(findField(), { key: "Enter", shiftKey: true });
  expect(status()).toBe("501 of 501");
  // The last row, far below the rows first rendered, is scrolled to the bottom of the view
  // (happy-dom sends no scroll event: sent here), rendered and current.
  expect(scroller().scrollTop).toBe(24 + 500 * 24 - 240);
  const shown = spyOn(HTMLElement.prototype, "scrollIntoView");
  act(() => fireEvent.scroll(scroller()));
  expect(marked("table-match-current")).toEqual([["501", "n500"]]);
  expect(marked("table-match")).toContainEqual(["500", "n499"]);
  // Then its cell, for its column, once rendered.
  expect(shown.mock.contexts).toEqual([document.querySelector(".table-match-current")]);
  expect(shown.mock.calls).toEqual([[{ block: "nearest", inline: "nearest" }]]);
  shown.mockRestore();
  click("Next match");
  expect(status()).toBe("1 of 501");
  click("Previous match");
  expect(status()).toBe("501 of 501");
  // A new search starts again; with nothing found the keys do nothing.
  typed("zzz");
  expect(status()).toBe("No results");
  press(findField(), { key: "Enter" });
  expect(status()).toBe("No results");
  // While composing, Enter is the input method's.
  typed("n4");
  press(findField(), { key: "Enter", isComposing: true });
  expect(status()).toBe("111 matches");
});

test("a regexp error is shown as the text's bar shows it, and the partial note", () => {
  render(<TableView table={table([sheet(many(9_999), { total: 30_000, partial: true })])} />);
  press(scroller(), { key: "f", ctrlKey: true });
  typed("n9999");
  expect(status()).toBe("1 match in the first 10,000 rows");
  click("Use regular expression");
  typed("n(");
  expect(findField().getAttribute("aria-invalid")).toBe("true");
  expect(status()).toBe("Invalid regular expression");
  expect(document.querySelector(".table-match")).toBeNull();
});

test("the find bar stays open across a workbook's sheets, over the one shown", () => {
  const books = table([
    sheet([["apple"], ["pear"]], { name: "Fruit" }),
    sheet([["pea"], ["pepper"], ["bean"]], { name: "Veg" }),
  ]);
  render(<TableView table={books} />);
  press(scroller(), { key: "f", ctrlKey: true });
  typed("pe");
  expect(status()).toBe("1 match");
  act(() => fireEvent.click(screen.getByRole("tab", { name: "Veg" })));
  expect(findField().value).toBe("pe");
  expect(status()).toBe("2 matches");
});
