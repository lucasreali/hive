import { SearchQuery } from "@codemirror/search";
import { defaultRangeExtractor, useVirtualizer } from "@tanstack/react-virtual";
import {
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal, flushSync } from "react-dom";
import type { Sheet, TableEdit, TableText } from "../protocol";
import { ContextMenu } from "../shell/WorktreeMenu";
import { commandKey } from "../window";
import { FindBar } from "./searchPanel";

/** Each row's height in pixels: one line, so the virtualizer never measures. */
const ROW = 24;
/** A column's width in characters, at least and at most (a longer cell is cut, its title whole). */
const NARROWEST = 3;
const WIDEST = 40;
/** A cell's room around its text in pixels: its padding (`.table-cell`) and right border. */
const PAD = 17;

const count = (n: number) => n.toLocaleString("en-US");

/** Under a table that could be edited, but of which the service did not send every row (25.4). */
export const NOT_WHOLE = "Read-only: not all of the file was read. Edit it in the text.";

/** What the note under the table says when some rows are not shown, else null. */
export function rowsNote(sheet: Sheet): string | null {
  if (sheet.rows.length >= sheet.total && !sheet.partial) return null;
  const of = sheet.partial ? `more than ${count(sheet.total)}` : count(sheet.total);
  return `Showing ${count(sheet.rows.length)} of ${of} rows`;
}

/**
 * The CSS grid columns of `rows`: the row numbers', then one per cell of the longest row, each
 * as wide as its longest cell (in `ch`, within `NARROWEST` and `WIDEST`).
 */
export function columns(rows: string[][]): string {
  const widths: number[] = [];
  for (const row of rows) {
    row.forEach((cell, i) => {
      widths[i] = Math.max(widths[i] ?? NARROWEST, Math.min(cell.length, WIDEST));
    });
  }
  const width = (chars: number) => `calc(${chars}ch + ${PAD}px)`;
  return [width(String(rows.length).length), ...widths.map(width)].join(" ");
}

/** A word's character, as CodeMirror's default categorizer counts one. */
const WORD = /[\p{Alphabetic}\p{Number}_]/u;
const word = (c: string | undefined) => c !== undefined && WORD.test(c);
/** Plain text as CodeMirror reads it, `\n`, `\r`, `\t` and `\\` unquoted (its `unquoted`). */
const UNQUOTED: Record<string, string> = { n: "\n", r: "\r", t: "\t", "\\": "\\" };
const unquote = (query: SearchQuery) =>
  query.literal
    ? query.search
    : query.search.replace(/\\([nrt\\])/g, (_, c: string) => UNQUOTED[c] as string);

/**
 * Each cell of `rows` (the header's too) holding a match of `query`, as `[row, column]` in
 * reading order (24.12). A match is the editor's: CodeMirror's regexp flags, its `\n` / `\t`
 * unquoting of plain text and its whole-word test; an empty match never counts.
 */
export function findCells(rows: string[][], query: SearchQuery): [number, number][] {
  if (!query.valid) return [];
  const source = query.regexp
    ? query.search
    : unquote(query).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const re = new RegExp(source, `gmu${query.caseSensitive ? "" : "i"}`);
  const whole = (cell: string, from: number, to: number) =>
    (!word(cell[from - 1]) || !word(cell[from])) && (!word(cell[to]) || !word(cell[to - 1]));
  const holds = (cell: string) => {
    re.lastIndex = 0;
    // Each try starts past the last one's start: at most one per character.
    for (let m = re.exec(cell); m; m = re.exec(cell)) {
      if (m[0] && (!query.wholeWord || whole(cell, m.index, m.index + m[0].length))) return true;
      re.lastIndex = m.index + 1;
    }
    return false;
  };
  const found: [number, number][] = [];
  rows.forEach((row, r) => {
    row.forEach((cell, c) => {
      if (holds(cell)) found.push([r, c]);
    });
  });
  return found;
}

/** The find bar's count, "3 of 41" ("41 matches" before one is current), and what it searched. */
export function findText(index: number, total: number, sheet: Sheet): string {
  const matches = total === 1 ? "1 match" : `${count(total)} matches`;
  const found =
    total === 0 ? "No results" : index < 0 ? matches : `${index + 1} of ${count(total)}`;
  return rowsNote(sheet) ? `${found} in the first ${count(sheet.rows.length)} rows` : found;
}

/**
 * A table file (23.6) as the service read it: one sheet, or a tab per sheet of a workbook. The
 * first row is the header, kept in view; only the rows in view are rendered. ⌘F (Ctrl+F off a
 * Mac) in it opens the find bar over the shown sheet (24.12), kept across sheets. With `onEdit`
 * (25.4) a sheet the service sent whole can be edited: a cell on a double click or Enter, rows
 * and columns from its context menu; `readOnly` says why it cannot.
 */
export function TableView({
  table,
  onEdit,
  readOnly = null,
}: {
  table: TableText;
  onEdit?: (edit: TableEdit) => void;
  readOnly?: string | null;
}) {
  const [picked, setPicked] = useState(0);
  const [query, setQuery] = useState(() => new SearchQuery({ search: "" }));
  const [finding, setFinding] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  if (table.error) return <div className="hint">{table.error}</div>;
  const at = Math.min(picked, table.sheets.length - 1);
  const sheet = table.sheets[at];
  const find = (e: KeyboardEvent) => {
    const f = e.key.toLowerCase() === "f" || e.code === "KeyF";
    if (!f || !commandKey(e.nativeEvent) || e.shiftKey || e.altKey) return;
    e.preventDefault();
    flushSync(() => setFinding(true));
    const field = box.current?.querySelector<HTMLInputElement>(".table-find [main-field]");
    field?.focus();
    field?.select();
  };
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: ⌘F from any of its parts, as in the text
    <div className="table-file" ref={box} onKeyDown={find}>
      {table.sheets.length > 1 && (
        <div className="table-sheets" role="tablist" aria-label="Sheets">
          {table.sheets.map((s, i) => (
            <button
              key={s.name}
              type="button"
              role="tab"
              className="table-sheet"
              aria-selected={i === at}
              onClick={() => setPicked(i)}
            >
              {s.name}
            </button>
          ))}
        </div>
      )}
      {sheet ? (
        <SheetGrid
          key={at}
          name={table.path}
          sheet={sheet}
          find={finding ? query : null}
          onQuery={setQuery}
          onClose={() => setFinding(false)}
          onEdit={onEdit}
          readOnly={readOnly}
        />
      ) : (
        <div className="hint">No sheets.</div>
      )}
    </div>
  );
}

interface SheetGridProps {
  name: string;
  sheet: Sheet;
  /** The find bar's query while it is open. */
  find: SearchQuery | null;
  onQuery: (query: SearchQuery) => void;
  onClose: () => void;
  onEdit?: (edit: TableEdit) => void;
  readOnly: string | null;
}

/** The cell under `target`: its row and column; a row's number counts as its first column. */
function cellAt(target: Element): [number, number] | null {
  const row = target.closest("tr");
  if (!row) return null;
  const column = target.closest<HTMLElement>("[data-column]")?.dataset.column ?? 0;
  return [Number(row.getAttribute("aria-rowindex")) - 1, Number(column)];
}

function SheetGrid(props: SheetGridProps) {
  const { name, sheet, find, onQuery, onClose, readOnly } = props;
  const note = rowsNote(sheet);
  // Writing back needs every row (25.4).
  const onEdit = note ? undefined : props.onEdit;
  const scroller = useRef<HTMLDivElement>(null);
  // The selected cell Enter edits, and whether it is being edited.
  const [cursor, setCursor] = useState<[number, number] | null>(null);
  const [typing, setTyping] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number; at: [number, number] } | null>(null);
  const [header, ...body] = sheet.rows;
  const grid = useMemo(() => columns(sheet.rows), [sheet.rows]);
  // A row's cells never move: each column's name is its key.
  const cols = useMemo(() => {
    const width = Math.max(1, ...sheet.rows.map((r) => r.length));
    return Array.from({ length: width }, (_, i) => `c${i}`);
  }, [sheet.rows]);
  const virtual = useVirtualizer({
    count: body.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => ROW,
    overscan: 10,
    scrollMargin: ROW,
    // Scrolled to, a row stops below the header kept in view.
    scrollPaddingStart: ROW,
    // The row being edited stays rendered, with its text, wherever the table scrolls.
    rangeExtractor: (range) => {
      const rows = defaultRangeExtractor(range);
      const at = typing && cursor ? cursor[0] - 1 : -1;
      return at < 0 || rows.includes(at) ? rows : [...rows, at].sort((a, b) => a - b);
    },
  });
  const found = useMemo(() => (find ? findCells(sheet.rows, find) : []), [find, sheet.rows]);
  // The current match, -1 before one is picked; a new search starts again.
  const [step, setStep] = useState({ found, index: -1 });
  const index = step.found === found ? step.index : -1;
  const current = found[index];
  const marks = useMemo(() => {
    const byRow = new Map<number, Set<number>>();
    for (const [r, c] of found) byRow.set(r, (byRow.get(r) ?? new Set()).add(c));
    return byRow;
  }, [found]);
  // Set when a match or the selected cell moves: that cell is scrolled into view once rendered.
  const reveal = useRef<string | null>(null);
  useEffect(() => {
    const cell = reveal.current && scroller.current?.querySelector(reveal.current);
    if (!cell) return;
    reveal.current = null;
    cell.scrollIntoView({ block: "nearest", inline: "nearest" });
  });
  // Next (1) or previous (-1), wrapping; the first move goes to the first or the last match.
  const move = (by: number) => {
    const total = found.length;
    if (total === 0) return;
    const next = index < 0 ? (by > 0 ? 0 : total - 1) : (index + by + total) % total;
    setStep({ found, index: next });
    const [r] = found[next] as [number, number];
    if (r > 0) virtual.scrollToIndex(r - 1, { align: "auto" });
    reveal.current = ".table-match-current";
  };
  // The selected cell goes to `r`, `c`, kept inside the table.
  const select = (r: number, c: number) => {
    const row = Math.max(0, Math.min(r, sheet.rows.length - 1));
    const column = Math.max(0, Math.min(c, cols.length - 1));
    setCursor([row, column]);
    if (row > 0) virtual.scrollToIndex(row - 1, { align: "auto" });
    reveal.current = ".table-cursor";
  };
  const KEYS: Record<string, [number, number]> = {
    ArrowUp: [-1, 0],
    ArrowDown: [1, 0],
    ArrowLeft: [0, -1],
    ArrowRight: [0, 1],
  };
  // From the table itself, not from a cell's editor.
  const onTableKey = (e: KeyboardEvent) => {
    if (!onEdit || !cursor || e.target !== scroller.current) return;
    const by = KEYS[e.key];
    if (by) {
      e.preventDefault();
      select(cursor[0] + by[0], cursor[1] + by[1]);
    } else if (e.key === "Enter") {
      e.preventDefault();
      setTyping(true);
    }
  };
  const pick = (e: MouseEvent) => {
    const at = onEdit && cellAt(e.target as Element);
    if (!at) return;
    setCursor(at);
    return at;
  };
  // The cell's editor closed with `value` (null: its edit dropped); Tab moves on by `step`.
  const typed = (value: string | null, step: number) => {
    setTyping(false);
    const [row, column] = cursor as [number, number];
    // A text area reads every line break as "\n": a cell left as it was is no edit.
    const before = (sheet.rows[row]?.[column] ?? "").replace(/\r\n?/g, "\n");
    if (value !== null && value !== before) {
      onEdit?.({ op: "set_cell", row, column, value });
    }
    if (step) select(row, column + step);
    scroller.current?.focus();
  };
  const close = () => {
    onClose();
    scroller.current?.focus();
  };
  if (!header) {
    return (
      <div className="hint">
        No rows.
        {onEdit && (
          <button
            type="button"
            className="ghost text"
            onClick={() => onEdit({ op: "insert_row", at: 0 })}
          >
            Add a row
          </button>
        )}
      </div>
    );
  }
  const style = { gridTemplateColumns: grid } as CSSProperties;
  const row = (cells: string[], at: number) => ({
    cells,
    index: at,
    cols,
    found: marks.get(at),
    current: current?.[0] === at ? current[1] : -1,
    cursor: cursor?.[0] === at ? cursor[1] : -1,
    typing,
    onTyped: typed,
  });
  const menuItem = (label: string, edit: TableEdit | null, danger = false) => (
    <button
      type="button"
      role="menuitem"
      className={danger ? "danger" : undefined}
      disabled={!edit}
      onClick={() => {
        setMenu(null);
        if (edit) onEdit?.(edit);
      }}
    >
      {label}
    </button>
  );
  return (
    <>
      {find && (
        // biome-ignore lint/a11y/useSemanticElements: <search> is unknown to older WebKit
        <div
          className="table-find find-bar"
          role="search"
          aria-label="Find in the table"
          onKeyDown={(e) => {
            if (e.key !== "Escape") return;
            e.preventDefault();
            close();
          }}
        >
          <FindBar
            query={find}
            onQuery={onQuery}
            onNext={() => move(1)}
            onPrevious={() => move(-1)}
            onClose={close}
            status={find.search ? findText(index, found.length, sheet) : undefined}
          />
        </div>
      )}
      {/* Focused by a click (and given back by the find bar), so ⌘F and the scroll keys reach it. */}
      {/* biome-ignore lint/a11y/noStaticElementInteractions: the cells' clicks and keys, as one */}
      <div
        className="table-view hive-scroll"
        ref={scroller}
        tabIndex={-1}
        onClick={pick}
        onDoubleClick={(e) => {
          if (pick(e)) setTyping(true);
        }}
        onContextMenu={(e) => {
          // The Menu key's event is on the table itself: the selected cell's.
          const at = onEdit && (cellAt(e.target as Element) ?? cursor);
          if (!at) return;
          e.preventDefault();
          setCursor(at);
          setMenu({ x: e.clientX, y: e.clientY, at });
        }}
        onKeyDown={onTableKey}
      >
        <table
          className="table-grid"
          aria-label={name}
          aria-rowcount={sheet.total}
          aria-colcount={cols.length}
          style={{ height: ROW + virtual.getTotalSize() }}
        >
          <thead>
            <Row {...row(header, 0)} style={style} header />
          </thead>
          <tbody>
            {virtual.getVirtualItems().map((item) => (
              <Row
                key={item.index}
                {...row(body[item.index] ?? [], item.index + 1)}
                style={{ ...style, transform: `translateY(${item.start}px)` }}
              />
            ))}
          </tbody>
        </table>
      </div>
      {(note || readOnly) && (
        <div className="table-note">
          {[note, readOnly ?? (note && props.onEdit && NOT_WHOLE)].filter(Boolean).join(" ")}
        </div>
      )}
      {menu &&
        createPortal(
          <ContextMenu at={menu} label="Table" onClose={() => setMenu(null)}>
            {menuItem("Insert Row Above", { op: "insert_row", at: menu.at[0] })}
            {menuItem("Insert Row Below", { op: "insert_row", at: menu.at[0] + 1 })}
            {menuItem("Delete Row", { op: "remove_row", at: menu.at[0] }, true)}
            {menuItem("Insert Column Left", { op: "insert_column", at: menu.at[1] })}
            {menuItem("Insert Column Right", { op: "insert_column", at: menu.at[1] + 1 })}
            {menuItem(
              "Delete Column",
              cols.length > 1 ? { op: "remove_column", at: menu.at[1] } : null,
              true,
            )}
          </ContextMenu>,
          document.body,
        )}
    </>
  );
}

interface RowProps {
  cells: string[];
  /** Its place among the sheet's rows, from 0. */
  index: number;
  /** A key per cell of the sheet's longest row: a shorter row is padded. */
  cols: string[];
  /** The columns of its cells holding a match of the find bar's query. */
  found?: Set<number>;
  /** The column of the current match when it is in this row, else -1. */
  current: number;
  /** The column of the selected cell when it is in this row, else -1. */
  cursor: number;
  /** The selected cell is being edited. */
  typing: boolean;
  onTyped: (value: string | null, step: number) => void;
  style: CSSProperties;
  header?: boolean;
}

/**
 * A cell's editor: one line and more as ⇧Enter adds them, over the rows below. Enter or leaving
 * it keeps the text, Tab too and moves on, Escape drops it.
 */
function CellEditor({ value, onDone }: { value: string; onDone: RowProps["onTyped"] }) {
  const box = useRef<HTMLTextAreaElement>(null);
  const closed = useRef(false);
  useEffect(() => {
    box.current?.focus();
    box.current?.select();
  }, []);
  const done = (text: string | null, step = 0) => {
    if (closed.current) return;
    closed.current = true;
    onDone(text, step);
  };
  return (
    <textarea
      ref={box}
      className="table-cell-input"
      aria-label="Cell value"
      defaultValue={value}
      rows={value.split("\n").length}
      spellCheck={false}
      autoCapitalize="off"
      autoCorrect="off"
      onInput={(e) => {
        e.currentTarget.rows = e.currentTarget.value.split("\n").length;
      }}
      onBlur={(e) => done(e.currentTarget.value)}
      onKeyDown={(e) => {
        if (e.key === "Escape") done(null);
        else if (e.key === "Tab") done(e.currentTarget.value, e.shiftKey ? -1 : 1);
        else if (e.key === "Enter" && !e.shiftKey) done(e.currentTarget.value);
        else return;
        e.preventDefault();
        e.stopPropagation();
      }}
    />
  );
}

function Row(props: RowProps) {
  const { cells, index, cols, found, current, cursor, typing, onTyped, style, header } = props;
  const Cell = header ? "th" : "td";
  const match = (i: number) =>
    i === current ? " table-match table-match-current" : found?.has(i) ? " table-match" : "";
  const editing = typing && cursor >= 0;
  return (
    <tr
      className="table-row"
      data-editing={editing || undefined}
      aria-rowindex={index + 1}
      style={style}
    >
      <td className="table-number" aria-hidden="true">
        {index + 1}
      </td>
      {cols.map((col, i) => (
        <Cell
          key={col}
          className={`table-cell${match(i)}${i === cursor ? " table-cursor" : ""}`}
          data-column={i}
          scope={header ? "col" : undefined}
          title={cells[i] || undefined}
        >
          {cells[i] ?? ""}
          {editing && i === cursor && <CellEditor value={cells[i] ?? ""} onDone={onTyped} />}
        </Cell>
      ))}
    </tr>
  );
}
