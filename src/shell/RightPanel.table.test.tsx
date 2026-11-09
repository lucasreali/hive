import { afterEach, beforeAll, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { followTableEdits } from "../follow";
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
  const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight")?.get;
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("table-view") ? 240 : original?.call(this);
    },
  });
});

afterEach(() => {
  mock.restore();
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
  expect(body().querySelector(".table-note")?.textContent).toBe(
    "Showing 2 of 12,000 rows Read-only: not all of the file was read. Edit it in the text.",
  );

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

test("the text view is the tab's choice, and unsaved edits can go back to the table (25.4)", () => {
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
  expect(toggle()?.hasAttribute("disabled")).toBe(false);
  expect(toggle()?.title).toBe("Show as a table");
  fireEvent.click(toggle() as HTMLElement);
  expect(pressed()).toBe("true");
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

// 25.4: a table with text to save is edited in place; the service writes the text back.

const at = { worktree: refactor.path, path: CSV };
const cell = (text: string) =>
  [...body().querySelectorAll(".table-cell")].find((c) => c.textContent === text) as HTMLElement;
const input = () => body().querySelector<HTMLTextAreaElement>(".table-cell-input");

/** The table of TEXT, its file answered, and what is sent to the service. */
function editable() {
  // Its own follower's calls only: `main.tsx`, imported by another test, follows `transport`.
  spyOn(transport, "editTable").mockImplementation(async () => {});
  const sent = mock(async (..._args: unknown[]) => {});
  const stop = followTableEdits({ editTable: sent } as unknown as typeof transport);
  apply({ type: "welcome", version: "0.1.0", distro: null });
  panel();
  open(CSV);
  file(CSV, TEXT);
  table(CSV, [
    ["id", "total"],
    ["1", "2"],
  ]);
  return { sent, stop };
}

const edited = (content: string, rows: string[][]) =>
  act(() =>
    apply({
      type: "table_edited",
      ...at,
      content,
      sheet: { name: "", rows, total: rows.length, partial: false },
      error: null,
    }),
  );

test("a cell edit asks the service, then the tab is dirty and saves through save_file", () => {
  const { sent, stop } = editable();
  const save = spyOn(transport, "saveFile").mockImplementation(async () => {});
  save.mockClear();
  expect(useHive.getState().edit).toBeNull();
  expect(screen.queryByRole("button", { name: "Save" })).toBeNull();

  fireEvent.doubleClick(cell("2"));
  expect(input()?.value).toBe("2");
  fireEvent.input(input() as HTMLElement, { target: { value: "3,5" } });
  fireEvent.keyDown(input() as HTMLElement, { key: "Enter" });
  expect(input()).toBeNull();
  const edit = { op: "set_cell", row: 1, column: 1, value: "3,5" } as const;
  expect(sent.mock.calls).toEqual([[refactor.path, CSV, TEXT, edit]]);
  expect(useHive.getState().editing).toBe(true);

  const next = 'id,total\n1,"3,5"\n';
  edited(next, [
    ["id", "total"],
    ["1", "3,5"],
  ]);
  expect(cell("3,5")).toBeDefined();
  const state = useHive.getState();
  expect(state.edit?.doc.toString()).toBe(next);
  expect(state.tableAsks).toEqual([]);
  // The tab is kept and marked, and its Save goes with the version it was read at.
  expect(state.openFiles.find((f) => f.path === CSV)?.preview).toBe(false);
  expect(
    screen.getByRole("button", { name: "Close file orders.csv (unsaved changes)" }),
  ).toBeDefined();
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(save.mock.calls).toEqual([[refactor.path, CSV, next, "v"]]);

  // The text shows the edit.
  fireEvent.click(toggle() as HTMLElement);
  expect(editor()?.textContent).toContain('"3,5"');
  stop();
});

test("a disk answer does not replace a table with unsaved edits; a clean one does", () => {
  const { stop } = editable();
  fireEvent.doubleClick(cell("2"));
  fireEvent.input(input() as HTMLElement, { target: { value: "9" } });
  fireEvent.keyDown(input() as HTMLElement, { key: "Enter" });
  edited("id,total\n1,9\n", [
    ["id", "total"],
    ["1", "9"],
  ]);
  table(CSV, [
    ["id", "total"],
    ["1", "2"],
  ]);
  expect(cell("9")).toBeDefined();
  // Saved: the disk's is the truth again.
  spyOn(transport, "saveFile").mockImplementation(async () => {});
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  act(() => apply({ type: "file_saved", ...at, version: "w" }));
  table(CSV, [["other"]]);
  expect(cell("other")).toBeDefined();
  stop();
});

test("⌘S with a cell open closes it and saves once the service answered", () => {
  const { stop } = editable();
  const save = spyOn(transport, "saveFile").mockImplementation(async () => {});
  save.mockClear();
  fireEvent.doubleClick(cell("2"));
  (input() as HTMLTextAreaElement).focus();
  fireEvent.input(input() as HTMLElement, { target: { value: "9" } });
  fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  expect(input()).toBeNull();
  // Not yet: the text is not back.
  expect(save).not.toHaveBeenCalled();
  expect(useHive.getState().saveAfterAsks).toEqual(at);
  edited("id,total\n1,9\n", [
    ["id", "total"],
    ["1", "9"],
  ]);
  expect(save.mock.calls).toEqual([[refactor.path, CSV, "id,total\n1,9\n", "v"]]);
  expect(useHive.getState().saveAfterAsks).toBeNull();
  stop();
});

test("the service's refusal shows, and the table and text stay", () => {
  const { sent, stop } = editable();
  fireEvent.doubleClick(cell("2"));
  fireEvent.input(input() as HTMLElement, { target: { value: "9" } });
  fireEvent.keyDown(input() as HTMLElement, { key: "Enter" });
  expect(sent).toHaveBeenCalledTimes(1);
  act(() =>
    apply({
      type: "table_edited",
      ...at,
      content: null,
      sheet: null,
      error: "That cell is not in the table",
    }),
  );
  expect(body().querySelector(".files-error")?.textContent).toBe("That cell is not in the table");
  expect(useHive.getState().edit?.doc.toString()).toBe(TEXT);
  expect(useHive.getState().tableAsks).toEqual([]);
  expect(cell("2")).toBeDefined();
  stop();
});

test("the file changed on disk under unsaved edits: the banner, without View diff", () => {
  const { stop } = editable();
  fireEvent.doubleClick(cell("2"));
  fireEvent.input(input() as HTMLElement, { target: { value: "9" } });
  fireEvent.keyDown(input() as HTMLElement, { key: "Enter" });
  edited("id,total\n1,9\n", [
    ["id", "total"],
    ["1", "9"],
  ]);
  file(CSV, "agent\n");
  const banner = screen.getByRole("alert");
  expect(banner.textContent).toContain("Changed on disk.");
  expect(within(banner).queryByRole("button", { name: "View diff" })).toBeNull();
  fireEvent.click(within(banner).getByRole("button", { name: "Reload" }));
  expect(useHive.getState().edit?.doc.toString()).toBe("agent\n");
  expect(screen.queryByRole("alert")).toBeNull();
  stop();
});

test("a table with no text to save is read-only and says so; a workbook says nothing", () => {
  panel();
  open(CSV);
  // Before the file's answer it cannot be edited yet, and that needs no note.
  table(CSV, [["a"], ["b"]]);
  fireEvent.doubleClick(cell("b"));
  expect(input()).toBeNull();
  expect(body().querySelector(".table-note")).toBeNull();
  // Not UTF-8, or over the text limit: no buffer.
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
  fireEvent.doubleClick(cell("b"));
  expect(input()).toBeNull();
  expect(body().querySelector(".table-note")?.textContent).toBe(
    "Read-only: this file has no text to edit.",
  );

  const BOOK = "data/budget.xlsx";
  open(BOOK, true);
  act(() =>
    apply({
      type: "file",
      worktree: refactor.path,
      path: BOOK,
      content: null,
      base: null,
      version: null,
      binary: true,
      too_large: false,
      error: null,
    }),
  );
  act(() =>
    apply({
      type: "table",
      worktree: refactor.path,
      path: BOOK,
      sheets: [{ name: "S", rows: [["a"], ["b"]], total: 2, partial: false }],
      error: null,
    }),
  );
  fireEvent.doubleClick(cell("b"));
  expect(input()).toBeNull();
  expect(body().querySelector(".table-note")).toBeNull();
  expect(useHive.getState().tableAsks).toEqual([]);
});

test("Diff from a table being edited shows the diff", () => {
  const { stop } = editable();
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
  fireEvent.doubleClick(cell("2"));
  fireEvent.keyDown(input() as HTMLElement, { key: "Escape" });
  act(() => useHive.setState({ editing: true }));
  fireEvent.click(screen.getByRole("button", { name: "Diff" }));
  expect(useHive.getState().editing).toBe(false);
  expect(pressed()).toBe("false");
  stop();
});
