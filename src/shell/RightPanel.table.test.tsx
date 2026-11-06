import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { apply } from "../reduce";
import {
  initialState,
  isTable,
  isWorkbook,
  pinFile,
  select,
  setOpenFile,
  setTableShown,
  tableShown,
  useHive,
} from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { toText } from "../viewer/buffer";
import { RightPanel } from "./RightPanel";
import { TerminalArea } from "./TerminalArea";

// 23.6: a CSV or TSV file's tab shows as a table, or as its text. 23.7: a workbook's, always
// as its table, a tab per sheet.

beforeAll(() => {
  // happy-dom has no layout: give the table's scroller a height so its rows show.
  const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight")?.get;
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("table-view") ? 240 : original?.call(this);
    },
  });
});

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [, api] = MOCK_REPOS;
const refactor = api.worktrees[1];
const CSV = "data/orders.csv";
const TEXT = "id,total\n1,2\n";

function panel() {
  spyOn(transport, "listChanges").mockImplementation(async () => {});
  apply({ type: "projects", projects: MOCK_REPOS });
  render(
    <>
      <RightPanel />
      <TerminalArea />
    </>,
  );
  act(() => select(refactor.id));
}

const file = (path: string, content: string) =>
  act(() =>
    apply({
      type: "file",
      worktree: refactor.path,
      path,
      content,
      base: content,
      version: "v",
      binary: false,
      too_large: false,
      error: null,
    }),
  );

const table = (path: string, rows: string[][], total = rows.length) =>
  act(() =>
    apply({
      type: "table",
      worktree: refactor.path,
      path,
      sheets: [{ name: "", rows, total, partial: false }],
      error: null,
    }),
  );

const open = (path: string, editing = false) =>
  act(() => setOpenFile({ worktree: refactor.path, path }, editing));
const toggle = () => screen.queryByRole("button", { name: "Show as a table" });
const pressed = () => toggle()?.getAttribute("aria-pressed");
const body = () => document.querySelector(".file-view-body") as HTMLElement;
const grid = () => body().querySelector("table");
const editor = () => body().querySelector(".cm-editor");

test("table files are known by their name", () => {
  const tables = ["a.csv", "B.TSV", "d/c.Csv", "b.xlsx", "C.XLS", "d/e.Ods"];
  expect(tables.map(isTable)).toEqual(tables.map(() => true));
  const others = ["a.txt", "a.csv.bak", "csv", "a.xlsm", "a.xlsx.bak", "xls", "a.odt"];
  expect(others.map(isTable)).toEqual(others.map(() => false));
  expect(tables.map(isWorkbook)).toEqual([false, false, false, true, true, true]);
  expect(others.map(isWorkbook)).toEqual(others.map(() => false));
});

test("a workbook always shows as its table, a tab per sheet, with no text to switch to", () => {
  panel();
  const BOOK = "data/budget.xlsx";
  open(BOOK, true);
  expect(useHive.getState().openFile?.path).toBe(BOOK);
  expect(tableShown(useHive.getState())).toBe(true);
  expect(toggle()).toBeNull();
  act(() =>
    apply({
      type: "table",
      worktree: refactor.path,
      path: BOOK,
      sheets: [
        { name: "Orders", rows: [["Item"], ["Pen"]], total: 2, partial: false },
        { name: "Notes", rows: [["Hello"]], total: 1, partial: false },
      ],
      error: null,
    }),
  );
  const tabs = within(screen.getByRole("tablist", { name: "Sheets" })).getAllByRole("tab");
  expect(tabs.map((t) => t.textContent)).toEqual(["Orders", "Notes"]);
  expect(screen.getAllByRole("columnheader").map((c) => c.textContent)).toEqual(["Item"]);
  fireEvent.click(tabs[1]);
  expect(screen.getAllByRole("columnheader").map((c) => c.textContent)).toEqual(["Hello"]);
  // Asking for its text (as Edit does) changes nothing.
  act(() => setTableShown(false));
  expect(useHive.getState().openFiles.find((t) => t.path === BOOK)?.asText).toBe(true);
  expect(tableShown(useHive.getState())).toBe(true);
  expect(grid()).not.toBeNull();
  expect(editor()).toBeNull();
});

test("a broken workbook says why in its place", () => {
  panel();
  open("old.xls");
  act(() =>
    apply({
      type: "table",
      worktree: refactor.path,
      path: "old.xls",
      sheets: [],
      error: "the workbook could not be read",
    }),
  );
  expect(body().querySelector(".hint")?.textContent).toBe("the workbook could not be read");
  expect(toggle()).toBeNull();
});

test("a CSV file opens as its table and the button switches it to the text and back", () => {
  panel();
  open("src/middleware/auth.ts");
  expect(toggle()).toBeNull();

  open(CSV, true);
  file(CSV, TEXT);
  expect(pressed()).toBe("true");
  expect(toggle()?.title).toBe("Show the text");
  expect(editor()).toBeNull();
  // Nothing until the service's table arrives; another file's is not shown.
  expect(grid()).toBeNull();
  table("other.csv", [["x"]]);
  expect(grid()).toBeNull();
  table(
    CSV,
    [
      ["id", "total"],
      ["1", "2"],
    ],
    12_000,
  );
  expect(screen.getAllByRole("columnheader").map((c) => c.textContent)).toEqual(["id", "total"]);
  expect([...body().querySelectorAll("td.table-cell")].map((c) => c.textContent)).toEqual([
    "1",
    "2",
  ]);
  expect(body().querySelector(".table-note")?.textContent).toBe("Showing 2 of 12,000 rows");

  fireEvent.click(toggle() as HTMLElement);
  expect(pressed()).toBe("false");
  expect(toggle()?.title).toBe("Show as a table");
  expect(grid()).toBeNull();
  expect(editor()).not.toBeNull();

  fireEvent.click(toggle() as HTMLElement);
  expect(pressed()).toBe("true");
  expect(grid()).not.toBeNull();
});

test("the table's error shows in its place", () => {
  panel();
  open(CSV);
  act(() =>
    apply({
      type: "table",
      worktree: refactor.path,
      path: CSV,
      sheets: [],
      error: "data/orders.csv does not exist",
    }),
  );
  expect(body().querySelector(".hint")?.textContent).toBe("data/orders.csv does not exist");
});

test("the text view is the tab's choice; unsaved edits keep the text until saved", () => {
  panel();
  open(CSV, true);
  file(CSV, TEXT);
  fireEvent.click(toggle() as HTMLElement);
  act(() => pinFile({ worktree: refactor.path, path: CSV }));
  open("data/other.tsv");
  expect(pressed()).toBe("true");
  open(CSV, true);
  expect(pressed()).toBe("false");

  act(() => useHive.setState((s) => ({ edit: s.edit && { ...s.edit, doc: toText("id\n") } })));
  expect(toggle()?.hasAttribute("disabled")).toBe(true);
  expect(toggle()?.title).toBe("Save or reload the file first");
});

test("Edit on a changed CSV file shows its text, editable", () => {
  panel();
  const changed = { path: CSV, old_path: null, status: "modified", added: 1, removed: 0 } as const;
  act(() =>
    apply({
      type: "changes",
      path: refactor.path,
      base: "head",
      branch: null,
      base_error: null,
      files: [changed],
      added: 1,
      removed: 0,
      error: null,
    }),
  );
  open(CSV);
  file(CSV, TEXT);
  expect(pressed()).toBe("true");
  fireEvent.click(screen.getByRole("button", { name: "Edit" }));
  expect(pressed()).toBe("false");
  expect(useHive.getState().editing).toBe(true);
  expect(editor()).not.toBeNull();
});
