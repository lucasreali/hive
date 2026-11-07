import { SearchQuery } from "@codemirror/search";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  type CSSProperties,
  type KeyboardEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { flushSync } from "react-dom";
import type { Sheet, TableText } from "../protocol";
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
 * first row is the header, kept in view; only the rows in view are rendered. Read-only. ⌘F
 * (Ctrl+F off a Mac) in it opens the find bar over the shown sheet (24.12), kept across sheets.
 */
export function TableView({ table }: { table: TableText }) {
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
}

function SheetGrid({ name, sheet, find, onQuery, onClose }: SheetGridProps) {
  const scroller = useRef<HTMLDivElement>(null);
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
  // Set when a match becomes current: its cell is scrolled into view once it is rendered.
  const reveal = useRef(false);
  useEffect(() => {
    const cell = scroller.current?.querySelector(".table-match-current");
    if (!reveal.current || !cell) return;
    reveal.current = false;
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
    reveal.current = true;
  };
  const close = () => {
    onClose();
    scroller.current?.focus();
  };
  const note = rowsNote(sheet);
  if (!header) return <div className="hint">No rows.</div>;
  const style = { gridTemplateColumns: grid } as CSSProperties;
  const row = (cells: string[], at: number) => ({
    cells,
    index: at,
    cols,
    found: marks.get(at),
    current: current?.[0] === at ? current[1] : -1,
  });
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
      <div className="table-view hive-scroll" ref={scroller} tabIndex={-1}>
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
      {note && <div className="table-note">{note}</div>}
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
  style: CSSProperties;
  header?: boolean;
}

function Row({ cells, index, cols, found, current, style, header }: RowProps) {
  const Cell = header ? "th" : "td";
  const match = (i: number) =>
    i === current ? " table-match table-match-current" : found?.has(i) ? " table-match" : "";
  return (
    <tr className="table-row" aria-rowindex={index + 1} style={style}>
      <td className="table-number" aria-hidden="true">
        {index + 1}
      </td>
      {cols.map((col, i) => (
        <Cell
          key={col}
          className={`table-cell${match(i)}`}
          scope={header ? "col" : undefined}
          title={cells[i] || undefined}
        >
          {cells[i] ?? ""}
        </Cell>
      ))}
    </tr>
  );
}
