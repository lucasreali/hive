//! A CSV or TSV text changed through its table (25.4): the text the app is editing and one
//! [`TableEdit`], answered by the new text and its table. Nothing is read or written on disk here;
//! the app keeps the text in its edit buffer and saves it with `save_file`, as any other edit.
//!
//! Only the bytes of what changed are touched: an edit is a list of splices on the text, found
//! by [`scan`], so every other cell keeps its quoting, every line its ending, and the blank lines
//! stay. A new or changed cell is quoted only when it needs it (RFC 4180, [`encode`]).

use std::io;
use std::ops::Range;

use hive_protocol::{Control, MAX_PAYLOAD, Sheet, TableEdit};

use crate::file::TEXT_LIMIT;
use crate::projects::Projects;
use crate::table::{self, COUNT_LIMIT};

const BOM: &[u8] = b"\xef\xbb\xbf";
const QUOTE: u8 = b'"';
const CR: u8 = b'\r';
const LF: u8 = b'\n';
/// A row with one empty cell: unquoted it would be a blank line, which is no row.
const EMPTY_CELL: &[u8] = b"\"\"";
/// Why an edit is refused when part of the table is not known.
const NOT_WHOLE: &str = "The whole file was not read: edit it in the text";
const TOO_LARGE: &str = "The file is too large to edit as a table";
const NO_CELL: &str = "That cell is not in the table";
/// A row added after a quote never closed would land inside it.
const UNCLOSED: &str = "The file's last quote is never closed: edit it in the text";

/// `edit_table`: `edit` on the table of `content`, the text of the table file `path` of `worktree`.
pub fn answer(
    projects: &Projects,
    worktree: String,
    path: String,
    content: &str,
    edit: Option<TableEdit>,
) -> Control {
    let done = projects
        .worktree(&worktree)
        .and_then(|_| edited(&worktree, &path, content, edit.as_ref()));
    message(worktree, path, done.map_err(|err| err.to_string()))
}

/// The text after `edit` and its table, within one frame.
fn edited(
    worktree: &str,
    path: &str,
    content: &str,
    edit: Option<&TableEdit>,
) -> io::Result<(String, Sheet)> {
    let Some(delimiter) = table::delimiter(path) else {
        return Err(io::Error::other("not a table file"));
    };
    if content.len() as u64 > TEXT_LIMIT {
        return Err(io::Error::other(TOO_LARGE));
    }
    let next = match edit {
        Some(edit) => {
            if !whole(&parse(content, delimiter, MAX_PAYLOAD)?) {
                return Err(io::Error::other(NOT_WHOLE));
            }
            let next = apply(content, delimiter, edit).map_err(io::Error::other)?;
            // `save_file` would refuse it, and every later edit too.
            if next.len() as u64 > TEXT_LIMIT {
                return Err(io::Error::other(TOO_LARGE));
            }
            next
        }
        None => content.to_owned(),
    };
    let Some(budget) = budget(worktree, path, &next) else {
        return Err(io::Error::other(TOO_LARGE));
    };
    let sheet = parse(&next, delimiter, budget)?;
    if edit.is_some() && !whole(&sheet) {
        return Err(io::Error::other(TOO_LARGE));
    }
    Ok((next, sheet))
}

fn parse(text: &str, delimiter: u8, budget: usize) -> io::Result<Sheet> {
    table::parse(text.as_bytes(), delimiter, budget, COUNT_LIMIT)
}

/// Every row of the file is in the sheet, so what is written back loses none.
fn whole(sheet: &Sheet) -> bool {
    !sheet.partial && sheet.rows.len() as u64 == sheet.total
}

/// The bytes of one frame left for the rows of the answer carrying `content`; none when it does
/// not fit by itself.
fn budget(worktree: &str, path: &str, content: &str) -> Option<usize> {
    let empty = Sheet {
        name: String::new(),
        rows: Vec::new(),
        total: u64::MAX,
        partial: false,
    };
    let done = Ok((content.to_owned(), empty));
    let envelope = message(worktree.into(), path.into(), done);
    let used = serde_json::to_vec(&envelope).map_or(usize::MAX, |json| json.len());
    MAX_PAYLOAD.checked_sub(used)
}

/// The `table_edited` answer.
fn message(worktree: String, path: String, done: Result<(String, Sheet), String>) -> Control {
    let (content, sheet, error) = match done {
        Ok((content, sheet)) => (Some(content), Some(sheet), None),
        Err(error) => (None, None, Some(error)),
    };
    Control::TableEdited {
        worktree,
        path,
        content,
        sheet,
        error,
    }
}

/// A row of the text, as the `csv` reader sees it (rows of any length, blank lines no rows).
struct Record {
    /// Each cell's bytes, its quotes included.
    cells: Vec<Range<usize>>,
    /// Past its last cell.
    end: usize,
    /// Past its line ending; `end` when it has none (the last line).
    next: usize,
    /// Its last cell's quote is never closed, so `end` is inside that quote.
    unclosed: bool,
}

impl Record {
    fn start(&self) -> usize {
        self.cells.first().map_or(self.end, |cell| cell.start)
    }

    fn cell(&self, column: usize) -> Option<&Range<usize>> {
        self.cells.get(column)
    }
}

/// The rows of `text`, cell by cell, by the same rules the `csv` reader decodes them with
/// ([`table::parse`]): a cell starting with a quote is quoted up to the next quote not doubled,
/// and what follows it, up to the delimiter, is plain; a quote elsewhere is plain; `\r`, `\n` or
/// `\r\n` end a row, a run of them is one row's ending and blank lines; a UTF-8 byte order mark
/// is left out.
///
/// Each loop is bounded by the input, so that no mistake in a step can make it endless: a row
/// takes a byte at least, and a cell but the last one its delimiter.
fn scan(text: &[u8], delimiter: u8) -> Vec<Record> {
    let mut at = if text.starts_with(BOM) { BOM.len() } else { 0 };
    let mut records = Vec::new();
    for _ in 0..text.len() {
        at += text
            .iter()
            .skip(at)
            .take_while(|&&b| b == CR || b == LF)
            .count();
        if at == text.len() {
            break;
        }
        let mut cells = Vec::new();
        let mut unclosed = false;
        for _ in 0..=text.len() {
            let (end, open) = cell_end(text, at, delimiter);
            unclosed = open;
            cells.push(at..end);
            at = end;
            if text.get(at) != Some(&delimiter) {
                break;
            }
            at += 1;
        }
        let end = at;
        if text.get(at) == Some(&CR) {
            at += 1;
        }
        // Past a `\r\n`, or a `\n` alone; a `\r` alone is an ending too.
        if text.get(at) == Some(&LF) {
            at += 1;
        }
        records.push(Record {
            cells,
            end,
            next: at,
            unclosed,
        });
    }
    records
}

/// Past the cell starting at `start`: on the delimiter, a line ending or the end of `text`; and
/// whether its quote was never closed.
fn cell_end(text: &[u8], start: usize, delimiter: u8) -> (usize, bool) {
    let mut at = start;
    let mut open = false;
    if text.get(at) == Some(&QUOTE) {
        // Its quotes end at the first byte after an odd run of quotes (doubled ones pair up), or
        // with the text.
        let mut odd = false;
        let closed = text.iter().enumerate().skip(at + 1).find(|&(_, &byte)| {
            if byte == QUOTE {
                odd = !odd;
            }
            byte != QUOTE && odd
        });
        at = closed.map_or(text.len(), |(after, _)| after);
        open = closed.is_none() && !odd;
    }
    let plain = |&&byte: &&u8| byte != delimiter && byte != CR && byte != LF;
    (at + text.iter().skip(at).take_while(plain).count(), open)
}

/// `value` as a cell of the file: in quotes only when it holds the delimiter, a quote or a line
/// break, its quotes doubled.
fn encode(value: &str, delimiter: u8) -> Vec<u8> {
    let special = |&byte: &u8| byte == delimiter || byte == QUOTE || byte == CR || byte == LF;
    if !value.as_bytes().iter().any(special) {
        return value.as_bytes().to_vec();
    }
    format!("\"{}\"", value.replace('"', "\"\"")).into_bytes()
}

/// `text` with each range replaced; the ranges are in order and do not overlap.
fn spliced(text: &[u8], splices: Vec<(Range<usize>, Vec<u8>)>) -> String {
    let mut out = Vec::with_capacity(text.len());
    let mut at = 0;
    for (range, bytes) in splices {
        out.extend(text.get(at..range.start).unwrap_or_default());
        out.extend(bytes);
        at = range.end;
    }
    out.extend(text.get(at..).unwrap_or_default());
    // Whole cells and rows of a UTF-8 text: split on ASCII bytes only, so still UTF-8.
    String::from_utf8_lossy(&out).into_owned()
}

/// `edit` on the table of `content`.
pub fn apply(content: &str, delimiter: u8, edit: &TableEdit) -> Result<String, String> {
    let text = content.as_bytes();
    let records = scan(text, delimiter);
    let width = records.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    let splices = match edit {
        TableEdit::SetCell { row, column, value } => set_cell(
            &records,
            width,
            *row as usize,
            *column as usize,
            value,
            delimiter,
        ),
        TableEdit::InsertRow { at } => insert_row(text, &records, width, *at as usize, delimiter),
        TableEdit::RemoveRow { at } => remove_row(&records, *at as usize),
        TableEdit::InsertColumn { at } => insert_column(&records, width, *at as usize, delimiter),
        TableEdit::RemoveColumn { at } => remove_column(&records, width, *at as usize),
    };
    let Some(splices) = splices else {
        return Err(NO_CELL.to_owned());
    };
    let inside = |record: &Record| record.unclosed.then_some(record.end);
    let open_end = records.last().and_then(inside);
    if splices
        .iter()
        .any(|(range, _)| Some(range.start) == open_end)
    {
        return Err(UNCLOSED.to_owned());
    }
    Ok(spliced(text, splices))
}

type Splices = Option<Vec<(Range<usize>, Vec<u8>)>>;

fn set_cell(
    records: &[Record],
    width: usize,
    row: usize,
    column: usize,
    value: &str,
    delimiter: u8,
) -> Splices {
    let record = records.get(row)?;
    if column >= width {
        return None;
    }
    let encoded = encode(value, delimiter);
    Some(vec![match record.cell(column) {
        Some(_) if record.cells.len() == 1 && encoded.is_empty() => {
            (record.start()..record.end, EMPTY_CELL.to_vec())
        }
        Some(cell) => (cell.clone(), encoded),
        // Past the end of a short row: it grows to reach the cell.
        None => {
            let missing = column - record.cells.len() + 1;
            let bytes = [vec![delimiter; missing], encoded].concat();
            (record.end..record.end, bytes)
        }
    }])
}

/// The file's line ending: its first row's, else `\n`.
fn line_ending<'a>(text: &'a [u8], records: &[Record]) -> &'a [u8] {
    records
        .first()
        .and_then(|r| text.get(r.end..r.next))
        .filter(|ending| !ending.is_empty())
        .unwrap_or(b"\n")
}

fn insert_row(text: &[u8], records: &[Record], width: usize, at: usize, delimiter: u8) -> Splices {
    let row = if width > 1 {
        vec![delimiter; width - 1]
    } else {
        EMPTY_CELL.to_vec()
    };
    let ending = line_ending(text, records);
    let (place, bytes) = match (records.get(at), records.last()) {
        (Some(record), _) => (record.start(), [row.as_slice(), ending].concat()),
        (None, _) if at > records.len() => return None,
        // After the last row, whose own ending goes after the new one: a last line with no ending
        // stays so.
        (None, Some(last)) => (last.end, [ending, row.as_slice()].concat()),
        (None, None) => (text.len(), [row.as_slice(), ending].concat()),
    };
    Some(vec![(place..place, bytes)])
}

fn remove_row(records: &[Record], at: usize) -> Splices {
    let record = records.get(at)?;
    let before = at.checked_sub(1).and_then(|row| records.get(row));
    let start = match before {
        // A last line with no ending: the row before it becomes one.
        Some(before) if record.next == record.end => before.end,
        _ => record.start(),
    };
    Some(vec![(start..record.next, Vec::new())])
}

fn insert_column(records: &[Record], width: usize, at: usize, delimiter: u8) -> Splices {
    if at > width {
        return None;
    }
    // A shorter row is left alone: the cells it lacks already read as empty.
    let place = |record: &Record| match record.cell(at) {
        Some(cell) => Some(cell.start),
        None if at == width && record.cells.len() == width => Some(record.end),
        None => None,
    };
    let splice = |place: usize| (place..place, vec![delimiter]);
    Some(records.iter().filter_map(place).map(splice).collect())
}

fn remove_column(records: &[Record], width: usize, at: usize) -> Splices {
    if at >= width {
        return None;
    }
    let splice = |record: &Record| {
        let cell = record.cell(at)?;
        // What is left of the row would be a blank line, which is no row.
        let blank = match record.cells.len() {
            1 => true,
            2 => record.cell(1 - at).is_some_and(Range::is_empty),
            _ => false,
        };
        if blank {
            return Some((record.start()..record.end, EMPTY_CELL.to_vec()));
        }
        Some(match record.cell(at + 1) {
            Some(next) => (cell.start..next.start, Vec::new()),
            // The last cell: the delimiter before it goes with it.
            None => (record.cell(at.wrapping_sub(1))?.end..cell.end, Vec::new()),
        })
    };
    Some(records.iter().filter_map(splice).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::parse_with;

    fn set(row: u32, column: u32, value: &str) -> TableEdit {
        TableEdit::SetCell {
            row,
            column,
            value: value.into(),
        }
    }

    fn csv(content: &str, edit: &TableEdit) -> String {
        apply(content, b',', edit).unwrap()
    }

    /// The cells `scan` finds, unquoted as the reader does.
    fn decoded(text: &[u8], delimiter: u8) -> Vec<Vec<String>> {
        let unquote = |cell: &[u8]| {
            let mut out = Vec::new();
            let mut quoted = cell.first() == Some(&QUOTE);
            let mut at = usize::from(quoted);
            while let Some(&byte) = cell.get(at) {
                if quoted && byte == QUOTE {
                    if cell.get(at + 1) == Some(&QUOTE) {
                        out.push(QUOTE);
                        at += 1;
                    } else {
                        quoted = false;
                    }
                } else {
                    out.push(byte);
                }
                at += 1;
            }
            String::from_utf8_lossy(&out).into_owned()
        };
        scan(text, delimiter)
            .iter()
            .map(|r| r.cells.iter().map(|c| unquote(&text[c.clone()])).collect())
            .collect()
    }

    #[test]
    fn the_scan_finds_the_cells_the_reader_decodes() {
        for (input, delimiter) in [
            (&b"a,b\n1,2\n"[..], b','),
            (b"a,b\r\n1,2\r\n", b','),
            (b"a,b\r1,2\r", b','),
            (b"a,b\n1,2", b','),
            (b"", b','),
            (b"\n\n", b','),
            (b"\r\n\r\na\r\n\r\n\nb\n\n", b','),
            (b"a,\n,b\n,\n", b','),
            (b"a,b,c\n1\n1,2,3,4\n\n,\n", b','),
            (
                b"\"Doe, Jane\",\"say \"\"hi\"\"\"\n\"two\nlines\",x\n",
                b',',
            ),
            (b"a\"b,c\"d\n", b','),
            (b"\"a\"b,c\n", b','),
            (b"\"a\"\"\"b\",c\n", b','),
            (b"\"a\"\",b\",c\n", b','),
            (b"\"a\"\"\nb\",c\n", b','),
            (b"\"open,\nnever closed", b','),
            (b"\"a\"", b','),
            (b"\"\"\n\"\",\"\"\n", b','),
            (b"\xef\xbb\xbfa,b\n", b','),
            (b"\xef\xbb\xbf\"a\",b\n", b','),
            (b"a\xef\xbb\xbf,b\n", b','),
            (b"a\tb,c\n1\t\"2\t3\"\n", b'\t'),
            ("é,ü\n\"ñ,\",ç\n".as_bytes(), b','),
            (b"a\0b,c\n", b','),
        ] {
            let sheet = table::parse(input, delimiter, MAX_PAYLOAD, COUNT_LIMIT).unwrap();
            let shown = String::from_utf8_lossy(input);
            assert_eq!(decoded(input, delimiter), sheet.rows, "{shown:?}");
        }
    }

    #[test]
    fn a_row_knows_its_ending() {
        let at = |text: &[u8]| {
            scan(text, b',')
                .iter()
                .map(|r| (r.start(), r.end, r.next))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            at(b"a\r\nb\nc\rd"),
            [(0, 1, 3), (3, 4, 5), (5, 6, 7), (7, 8, 8)]
        );
        assert_eq!(at(b"\n\na\n\nb"), [(2, 3, 4), (5, 6, 6)]);
        assert_eq!(at(b"\xef\xbb\xbfa\n"), [(3, 4, 5)]);
    }

    #[test]
    fn a_value_is_quoted_only_when_it_needs_it() {
        let cell = |value: &str, delimiter| String::from_utf8(encode(value, delimiter)).unwrap();
        for value in ["plain", "", " spaced ", "semi;colon", "é"] {
            assert_eq!(cell(value, b','), value);
        }
        assert_eq!(cell("a,b", b','), "\"a,b\"");
        assert_eq!(cell("say \"hi\"", b','), "\"say \"\"hi\"\"\"");
        assert_eq!(cell("a\nb", b','), "\"a\nb\"");
        assert_eq!(cell("a\rb", b','), "\"a\rb\"");
        // The delimiter is the file's: a comma in a TSV needs none, a tab does.
        assert_eq!(cell("a,b", b'\t'), "a,b");
        assert_eq!(cell("a\tb", b'\t'), "\"a\tb\"");
        assert_eq!(cell("a\tb", b','), "a\tb");
    }

    #[test]
    fn an_edit_of_a_cell_changes_only_that_cells_bytes() {
        // Needless quotes, CRLF, blank lines, spaces and no final line ending: all kept.
        let file = "\"id\", name ,\"x\"\r\n\r\n1,\"Doe, Jane\",\r\n\n2,b,c";
        assert_eq!(
            csv(file, &set(1, 2, "new")),
            "\"id\", name ,\"x\"\r\n\r\n1,\"Doe, Jane\",new\r\n\n2,b,c"
        );
        assert_eq!(
            csv(file, &set(0, 0, "ID")),
            "ID, name ,\"x\"\r\n\r\n1,\"Doe, Jane\",\r\n\n2,b,c"
        );
        assert_eq!(
            csv(file, &set(2, 1, "a,b")),
            "\"id\", name ,\"x\"\r\n\r\n1,\"Doe, Jane\",\r\n\n2,\"a,b\",c"
        );
        // Each thing that needs quotes.
        assert_eq!(csv("a,b\n", &set(0, 1, "x,y")), "a,\"x,y\"\n");
        assert_eq!(csv("a,b\n", &set(0, 1, "x\"y")), "a,\"x\"\"y\"\n");
        assert_eq!(csv("a,b\n", &set(0, 1, "x\ny")), "a,\"x\ny\"\n");
        // No more than it needs: a quoted cell given a plain value loses its quotes.
        assert_eq!(csv("\"a\",b\n", &set(0, 0, "c")), "c,b\n");
    }

    #[test]
    fn a_byte_order_mark_and_other_scripts_are_kept() {
        assert_eq!(
            csv("\u{feff}é,ü\nñ,ç\n", &set(1, 0, "日本")),
            "\u{feff}é,ü\n日本,ç\n"
        );
    }

    #[test]
    fn a_tsv_keeps_its_tabs() {
        let edit = |content, edit: &TableEdit| apply(content, b'\t', edit).unwrap();
        let file = "a\tb,c\n1\t2\n";
        assert_eq!(edit(file, &set(1, 1, "x,y")), "a\tb,c\n1\tx,y\n");
        assert_eq!(edit(file, &set(1, 1, "x\ty")), "a\tb,c\n1\t\"x\ty\"\n");
        assert_eq!(
            edit(file, &TableEdit::InsertRow { at: 1 }),
            "a\tb,c\n\t\n1\t2\n"
        );
        assert_eq!(
            edit(file, &TableEdit::InsertColumn { at: 1 }),
            "a\t\tb,c\n1\t\t2\n"
        );
        assert_eq!(edit(file, &TableEdit::RemoveColumn { at: 0 }), "b,c\n2\n");
    }

    #[test]
    fn a_cell_past_a_short_row_lengthens_it() {
        assert_eq!(csv("a,b,c\n1\n", &set(1, 2, "x")), "a,b,c\n1,,x\n");
        assert_eq!(csv("a,b,c\n1\n", &set(1, 1, "x")), "a,b,c\n1,x\n");
        // A final line with no ending.
        assert_eq!(csv("a,b\n1", &set(1, 1, "x")), "a,b\n1,x");
    }

    #[test]
    fn a_cell_outside_the_table_is_refused() {
        let refused = |edit: TableEdit| apply("a,b\n1\n", b',', &edit).unwrap_err();
        assert_eq!(refused(set(2, 0, "x")), NO_CELL);
        assert_eq!(refused(set(0, 2, "x")), NO_CELL);
        assert_eq!(refused(set(1, u32::MAX, "x")), NO_CELL);
        assert_eq!(refused(TableEdit::InsertRow { at: 3 }), NO_CELL);
        assert_eq!(refused(TableEdit::RemoveRow { at: 2 }), NO_CELL);
        assert_eq!(refused(TableEdit::InsertColumn { at: 3 }), NO_CELL);
        assert_eq!(refused(TableEdit::RemoveColumn { at: 2 }), NO_CELL);
    }

    #[test]
    fn a_lone_empty_cell_stays_a_row() {
        // Unquoted it would be a blank line, and the row would be gone.
        let remove = |at| TableEdit::RemoveColumn { at };
        assert_eq!(csv("a\nb\n", &set(1, 0, "")), "a\n\"\"\n");
        assert_eq!(csv("a,b\nc\n", &set(1, 0, "")), "a,b\n\"\"\n");
        assert_eq!(csv("a,b\nc,d\n", &set(1, 0, "")), "a,b\n,d\n");
        assert_eq!(
            csv("a\nb\n", &TableEdit::InsertRow { at: 1 }),
            "a\n\"\"\nb\n"
        );
        assert_eq!(csv("a,\n,b\n", &remove(0)), "\"\"\nb\n");
        assert_eq!(csv("a,\n,b\n", &remove(1)), "a\n\"\"\n");
        assert_eq!(csv("a,b\nc\n", &remove(0)), "b\n\"\"\n");
        // The only column: its rows stay, empty.
        assert_eq!(csv("a\nb\n", &remove(0)), "\"\"\n\"\"\n");
    }

    #[test]
    fn a_new_row_takes_the_files_line_ending_and_the_tables_width() {
        let insert = |at| TableEdit::InsertRow { at };
        assert_eq!(csv("a,b,c\n1,2,3\n", &insert(0)), ",,\na,b,c\n1,2,3\n");
        assert_eq!(csv("a,b,c\n1,2,3\n", &insert(1)), "a,b,c\n,,\n1,2,3\n");
        assert_eq!(csv("a,b,c\n1,2,3\n", &insert(2)), "a,b,c\n1,2,3\n,,\n");
        assert_eq!(csv("a,b\r\n1,2\r\n", &insert(1)), "a,b\r\n,\r\n1,2\r\n");
        assert_eq!(csv("a,b\r\n1,2\r\n", &insert(2)), "a,b\r\n1,2\r\n,\r\n");
        assert_eq!(csv("a,b\r1,2\r", &insert(1)), "a,b\r,\r1,2\r");
        // A final line with no ending stays so.
        assert_eq!(csv("a,b\n1,2", &insert(2)), "a,b\n1,2\n,");
        assert_eq!(csv("a,b\r\n1,2", &insert(2)), "a,b\r\n1,2\r\n,");
        assert_eq!(csv("a,b", &insert(1)), "a,b\n,");
        // Blank lines stay where they were.
        assert_eq!(csv("a,b\n\n1,2\n\n", &insert(1)), "a,b\n\n,\n1,2\n\n");
        assert_eq!(csv("a,b\n\n1,2\n\n", &insert(2)), "a,b\n\n1,2\n,\n\n");
        // The width is the longest row's.
        assert_eq!(csv("a\n1,2,3\n", &insert(0)), ",,\na\n1,2,3\n");
        // An empty file starts one.
        assert_eq!(csv("", &insert(0)), "\"\"\n");
        assert_eq!(csv("\u{feff}", &insert(0)), "\u{feff}\"\"\n");
        assert_eq!(csv("\n", &insert(0)), "\n\"\"\n");
    }

    #[test]
    fn a_removed_row_takes_its_line_ending_with_it() {
        let remove = |at| TableEdit::RemoveRow { at };
        assert_eq!(csv("a\nb\nc\n", &remove(0)), "b\nc\n");
        assert_eq!(csv("a\nb\nc\n", &remove(1)), "a\nc\n");
        assert_eq!(csv("a\nb\nc\n", &remove(2)), "a\nb\n");
        assert_eq!(csv("a\r\nb\r\nc\r\n", &remove(1)), "a\r\nc\r\n");
        // A final line with no ending stays so.
        assert_eq!(csv("a\nb\nc", &remove(2)), "a\nb");
        assert_eq!(csv("a\nb", &remove(0)), "b");
        assert_eq!(csv("a", &remove(0)), "");
        assert_eq!(csv("a\n", &remove(0)), "");
        // Blank lines around it stay; a byte order mark too.
        assert_eq!(csv("a\n\nb\n\nc\n", &remove(1)), "a\n\n\nc\n");
        assert_eq!(csv("\u{feff}a\nb\n", &remove(0)), "\u{feff}b\n");
    }

    #[test]
    fn a_new_column_reaches_every_row_that_holds_it() {
        let insert = |at| TableEdit::InsertColumn { at };
        assert_eq!(csv("a,b\n1,2\n", &insert(0)), ",a,b\n,1,2\n");
        assert_eq!(csv("a,b\n1,2\n", &insert(1)), "a,,b\n1,,2\n");
        assert_eq!(csv("a,b\r\n1,2\r\n", &insert(2)), "a,b,\r\n1,2,\r\n");
        assert_eq!(csv("a,b\n1,2", &insert(2)), "a,b,\n1,2,");
        assert_eq!(csv("\"a,x\",b\n", &insert(1)), "\"a,x\",,b\n");
        // A shorter row has no cells to move: the ones it lacks already read as empty.
        assert_eq!(csv("a,b,c\n1\n", &insert(2)), "a,b,,c\n1\n");
        assert_eq!(csv("a,b,c\n1\n", &insert(1)), "a,,b,c\n1\n");
        assert_eq!(csv("a,b,c\n1\n", &insert(3)), "a,b,c,\n1\n");
    }

    #[test]
    fn a_removed_column_takes_one_delimiter_with_it() {
        let remove = |at| TableEdit::RemoveColumn { at };
        assert_eq!(csv("a,b,c\n1,2,3\n", &remove(0)), "b,c\n2,3\n");
        assert_eq!(csv("a,b,c\n1,2,3\n", &remove(1)), "a,c\n1,3\n");
        assert_eq!(csv("a,b,c\n1,2,3\n", &remove(2)), "a,b\n1,2\n");
        assert_eq!(csv("a,\"b,c\",d\r\n", &remove(1)), "a,d\r\n");
        // A shorter row is left alone.
        assert_eq!(csv("a,b,c\n1\n", &remove(1)), "a,c\n1\n");
        assert_eq!(csv("a,b,c\n1,2\n", &remove(1)), "a,c\n1\n");
        assert_eq!(csv("a,b,c\n1,2\n", &remove(2)), "a,b\n1,2\n");
    }

    #[test]
    fn only_a_whole_sheet_can_be_edited() {
        let sheet = |rows: usize, total, partial| Sheet {
            name: String::new(),
            rows: vec![vec![]; rows],
            total,
            partial,
        };
        assert!(whole(&sheet(2, 2, false)));
        assert!(!whole(&sheet(2, 3, false)));
        assert!(!whole(&sheet(2, 2, true)));
        // A row over the limit, as the table reads it: partial, so refused.
        let input = &b"a\nbbbbbbbbbb\n"[..];
        let (cut, _) = parse_with(input, b',', MAX_PAYLOAD, COUNT_LIMIT, 4).unwrap();
        assert!(!whole(&cut));
    }

    fn done(content: &str, edit: Option<&TableEdit>) -> Result<(String, Sheet), String> {
        edited("/w", "a.csv", content, edit).map_err(|err| err.to_string())
    }

    #[test]
    fn the_answer_holds_the_new_text_and_its_table() {
        let (text, sheet) = done("a,b\n1,2\n", Some(&set(1, 1, "x,y"))).unwrap();
        assert_eq!(text, "a,b\n1,\"x,y\"\n");
        assert_eq!(sheet.rows[1], ["1", "x,y"]);
        assert_eq!((sheet.total, sheet.partial), (2, false));
        // With no edit, the text's table: its text as it was.
        let (text, sheet) = done("\"a\"\n", None).unwrap();
        assert_eq!(
            (text.as_str(), sheet.rows),
            ("\"a\"\n", vec![vec!["a".to_owned()]])
        );
    }

    #[test]
    fn the_path_says_the_delimiter_and_only_a_table_file_is_edited() {
        let tsv = edited("/w", "a.TSV", "a\tb\n", Some(&set(0, 0, "x")));
        assert_eq!(tsv.unwrap().0, "x\tb\n");
        for path in ["a.txt", "a.xlsx", "a"] {
            let err = edited("/w", path, "a\n", None).unwrap_err();
            assert_eq!(err.to_string(), "not a table file");
        }
    }

    #[test]
    fn nothing_lands_inside_a_quote_never_closed() {
        let file = "a,b\n\"open";
        let refused = |edit: TableEdit| apply(file, b',', &edit).unwrap_err();
        // After its end, which is inside the quote: the row, the short row's cell, the column.
        assert_eq!(refused(TableEdit::InsertRow { at: 2 }), UNCLOSED);
        assert_eq!(refused(set(1, 1, "x")), UNCLOSED);
        // A row too short for the new column is left alone, so its end is not touched.
        assert_eq!(
            csv(file, &TableEdit::InsertColumn { at: 2 }),
            "a,b,\n\"open"
        );
        let closes = "\"open";
        let column = TableEdit::InsertColumn { at: 1 };
        assert_eq!(apply(closes, b',', &column).unwrap_err(), UNCLOSED);
        let doubled = "\"open\"\"";
        assert_eq!(apply(doubled, b',', &column).unwrap_err(), UNCLOSED);
        // Whatever does not touch its end is fine: its own cell closes it, or it goes.
        assert_eq!(csv(file, &set(1, 0, "x")), "a,b\nx");
        assert_eq!(csv(file, &set(0, 1, "x")), "a,x\n\"open");
        assert_eq!(csv(file, &TableEdit::RemoveRow { at: 1 }), "a,b");
        assert_eq!(csv(file, &TableEdit::InsertRow { at: 1 }), "a,b\n,\n\"open");
        assert_eq!(
            csv(file, &TableEdit::InsertColumn { at: 0 }),
            ",a,b\n,\"open"
        );
        // A quote that is closed, at the very end of the text, is no such thing.
        let closed = "a,b\n\"c\"";
        assert_eq!(
            csv(closed, &TableEdit::InsertRow { at: 2 }),
            "a,b\n\"c\"\n,"
        );
        assert_eq!(csv(closed, &set(1, 1, "x")), "a,b\n\"c\",x");
        let earlier = "\"a\"\"\"";
        assert_eq!(csv(earlier, &column), "\"a\"\"\",");
    }

    #[test]
    fn an_edit_past_the_text_limit_is_refused() {
        let limit = TEXT_LIMIT as usize;
        // A cell that brings the text to the limit, line break included; one byte more.
        let fits = set(0, 0, &"x".repeat(limit - 1));
        let (text, _) = done("a\n", Some(&fits)).unwrap();
        assert_eq!(text.len(), limit);
        let over = set(0, 0, &"x".repeat(limit));
        let refused = done("a\n", Some(&over)).err();
        assert_eq!(refused.as_deref(), Some(TOO_LARGE));
    }

    #[test]
    fn a_table_with_rows_not_sent_refuses_an_edit() {
        let long: String = (0..=table::TABLE_ROWS).map(|i| format!("{i}\n")).collect();
        assert_eq!(done(&long, Some(&set(0, 0, "x"))).unwrap_err(), NOT_WHOLE);
        // Asked for its table alone, it answers what can be sent.
        let (text, sheet) = done(&long, None).unwrap();
        assert_eq!(text, long);
        assert_eq!(sheet.rows.len(), table::TABLE_ROWS);
        assert_eq!(sheet.total, table::TABLE_ROWS as u64 + 1);
    }

    #[test]
    fn an_edit_that_adds_a_row_past_the_cap_is_refused() {
        let full: String = (0..table::TABLE_ROWS).map(|i| format!("{i}\n")).collect();
        let (text, _) = done(&full, Some(&set(0, 0, "x"))).unwrap();
        assert!(text.starts_with("x\n1\n"));
        let at = table::TABLE_ROWS as u32;
        let added = done(&full, Some(&TableEdit::InsertRow { at })).unwrap_err();
        assert_eq!(added, TOO_LARGE);
    }

    #[test]
    fn a_text_over_the_limit_is_refused() {
        let limit = TEXT_LIMIT as usize;
        let at_limit = format!("{}\n", "a".repeat(limit - 1));
        assert!(done(&at_limit, None).is_ok());
        let over = format!("{}\n", "a".repeat(limit));
        assert_eq!(done(&over, None).unwrap_err(), TOO_LARGE);
    }

    #[test]
    fn an_answer_that_cannot_fit_a_frame_is_refused() {
        // Each control byte is six in JSON: within the text limit, over the frame's.
        let text = "\u{1}".repeat(TEXT_LIMIT as usize);
        assert_eq!(done(&text, None).unwrap_err(), TOO_LARGE);
        let empty = || Sheet {
            name: String::new(),
            rows: Vec::new(),
            total: u64::MAX,
            partial: false,
        };
        let envelope = |content: &str| {
            let done = Ok((content.to_owned(), empty()));
            serde_json::to_vec(&message("/w".into(), "a.csv".into(), done))
                .unwrap()
                .len()
        };
        assert_eq!(
            budget("/w", "a.csv", "abc"),
            Some(MAX_PAYLOAD - envelope("abc"))
        );
        // To the byte: a text that fills the frame, and one byte more.
        let fill = "a".repeat(MAX_PAYLOAD - envelope(""));
        assert_eq!(budget("/w", "a.csv", &fill), Some(0));
        assert_eq!(budget("/w", "a.csv", &format!("{fill}a")), None);
    }

    #[test]
    fn a_table_that_stops_fitting_is_refused() {
        // Each control byte is six in JSON, once in the text and once in the rows: these fit a
        // frame with their rows alone or their text alone, not with both.
        let cell = "\u{1}".repeat(1000);
        let text: String = (0..420).map(|_| format!("{cell}\n")).collect();
        let edit = set(0, 0, "x");
        let refused = done(&text, Some(&edit)).err();
        assert_eq!(refused.as_deref(), Some(TOO_LARGE));
    }

    #[test]
    fn a_worktree_not_followed_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let spaces = dir.path().join("spaces.json");
        let projects = Projects::load(spaces, &dir.path().join("projects.json"));
        let edit = Some(set(0, 0, "x"));
        let answer = answer(&projects, "/nowhere".into(), "a.csv".into(), "a\n", edit);
        assert_eq!(
            answer,
            Control::TableEdited {
                worktree: "/nowhere".into(),
                path: "a.csv".into(),
                content: None,
                sheet: None,
                error: Some("/nowhere is not a worktree of a followed project".into()),
            }
        );
    }
}
