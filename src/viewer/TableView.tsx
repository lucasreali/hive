import { useVirtualizer } from "@tanstack/react-virtual";
import { type CSSProperties, useMemo, useRef, useState } from "react";
import type { Sheet, TableText } from "../protocol";

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

/**
 * A table file (23.6) as the service read it: one sheet, or a tab per sheet of a workbook. The
 * first row is the header, kept in view; only the rows in view are rendered. Read-only.
 */
export function TableView({ table }: { table: TableText }) {
  const [picked, setPicked] = useState(0);
  if (table.error) return <div className="hint">{table.error}</div>;
  const at = Math.min(picked, table.sheets.length - 1);
  const sheet = table.sheets[at];
  return (
    <div className="table-file">
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
        <SheetGrid key={at} name={table.path} sheet={sheet} />
      ) : (
        <div className="hint">No sheets.</div>
      )}
    </div>
  );
}

function SheetGrid({ name, sheet }: { name: string; sheet: Sheet }) {
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
  });
  const note = rowsNote(sheet);
  if (!header) return <div className="hint">No rows.</div>;
  const style = { gridTemplateColumns: grid } as CSSProperties;
  return (
    <>
      <div className="table-view hive-scroll" ref={scroller}>
        <table
          className="table-grid"
          aria-label={name}
          aria-rowcount={sheet.total}
          aria-colcount={cols.length}
          style={{ height: ROW + virtual.getTotalSize() }}
        >
          <thead>
            <Row cells={header} index={0} cols={cols} style={style} header />
          </thead>
          <tbody>
            {virtual.getVirtualItems().map((item) => (
              <Row
                key={item.index}
                cells={body[item.index] ?? []}
                index={item.index + 1}
                cols={cols}
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
  style: CSSProperties;
  header?: boolean;
}

function Row({ cells, index, cols, style, header }: RowProps) {
  const Cell = header ? "th" : "td";
  return (
    <tr className="table-row" aria-rowindex={index + 1} style={style}>
      <td className="table-number" aria-hidden="true">
        {index + 1}
      </td>
      {cols.map((col, i) => (
        <Cell
          key={col}
          className="table-cell"
          scope={header ? "col" : undefined}
          title={cells[i] || undefined}
        >
          {cells[i] ?? ""}
        </Cell>
      ))}
    </tr>
  );
}
