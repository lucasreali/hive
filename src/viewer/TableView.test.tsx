import { afterEach, beforeAll, expect, test } from "bun:test";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { Sheet, TableText } from "../protocol";
import { columns, rowsNote, TableView } from "./TableView";

beforeAll(() => {
  // happy-dom has no layout: give the scroller a height so the virtualizer shows rows.
  const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight")?.get;
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("table-view") ? 240 : original?.call(this);
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
