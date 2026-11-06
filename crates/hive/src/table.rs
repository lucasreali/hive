//! A CSV or TSV file of a worktree as a table (23.6): its first [`TABLE_ROWS`] rows and how
//! many it has, for the viewer's table. Read on disk with the file viewer's checks
//! ([`file::open_inside`]), whatever its size: a file over `file::TEXT_LIMIT` still shows its
//! first rows. A workbook (23.7, [`workbook`]) answers the same `table` message, one [`Sheet`]
//! per sheet.

use std::io::{self, Read};
use std::path::Path;

use hive_protocol::{Control, MAX_PAYLOAD, Sheet};

use crate::file;
use crate::projects::Projects;
use crate::workbook::{self, Format, Worker};

/// Most rows sent of a sheet.
pub const TABLE_ROWS: usize = 10_000;
/// Most bytes of a file read to count its rows (64 MiB); past them the count is partial.
pub const COUNT_LIMIT: u64 = 67_108_864;
/// Longest row read, in bytes (1 MiB): the reading stops at a longer one, as at the count limit.
pub const RECORD_LIMIT: u64 = 1_048_576;

/// The field delimiter of a table file by its extension, in any case: `,` for `.csv`, a tab
/// for `.tsv`.
pub fn delimiter(path: &str) -> Option<u8> {
    let extension = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "csv" => Some(b','),
        "tsv" => Some(b'\t'),
        _ => None,
    }
}

/// `open_table`: `path` of `worktree` as a table, its rows within one frame. A workbook is
/// read by this binary run again ([`workbook::Worker`]).
pub fn answer(projects: &Projects, worktree: String, path: String) -> Control {
    let budget = budget(&worktree, &path);
    let read = projects.worktree(&worktree).and_then(|dir| {
        let worker = Worker::new(std::env::current_exe()?);
        read(&dir, &path, budget, &worker)
    });
    message(worktree, path, read)
}

/// The bytes of one frame left for the sheets of the `table` answer for `path` of `worktree`.
fn budget(worktree: &str, path: &str) -> usize {
    let envelope = message(worktree.into(), path.into(), Ok(Vec::new()));
    let used = serde_json::to_vec(&envelope).map_or(MAX_PAYLOAD, |json| json.len());
    MAX_PAYLOAD.saturating_sub(used)
}

/// The bytes of a CSV's sheet record with no rows, as large as its count can make it.
fn record() -> usize {
    let empty = Sheet {
        name: String::new(),
        rows: Vec::new(),
        total: u64::MAX,
        partial: false,
    };
    serde_json::to_vec(&empty).map_or(MAX_PAYLOAD, |json| json.len())
}

/// The sheets of the table file `path` of the worktree at `dir` (checked as the viewer's
/// files are), taking at most `budget` bytes of JSON: a CSV or TSV file's one, or a
/// workbook's, read by `worker`. The path is checked before its extension, and no error
/// repeats it: the answer has it already.
pub fn read(dir: &Path, path: &str, budget: usize, worker: &Worker) -> io::Result<Vec<Sheet>> {
    let file = file::open_inside(dir, path)?;
    let kind = delimiter(path)
        .map(Kind::Text)
        .or_else(|| Format::of(path).map(Kind::Workbook));
    let Some(kind) = kind else {
        return Err(io::Error::other("not a table file"));
    };
    let Some(file) = file else {
        return Err(io::Error::other(format!("{path} does not exist")));
    };
    match kind {
        Kind::Text(delimiter) => {
            let rows = budget.saturating_sub(record());
            Ok(vec![parse(file, delimiter, rows, COUNT_LIMIT)?])
        }
        Kind::Workbook(format) => {
            let size = file.metadata()?.len();
            workbook::read(file, size, format, budget, worker)
        }
    }
}

/// How a table file is read, by its extension.
enum Kind {
    /// CSV or TSV, with its delimiter.
    Text(u8),
    Workbook(Format),
}

/// The sheet of CSV `input` (TSV with a tab `delimiter`), quoted fields as RFC 4180 has them:
/// rows of any length, each cell decoded as UTF-8 (an invalid byte shows as U+FFFD); at most
/// [`TABLE_ROWS`] rows, while their JSON takes at most `budget` bytes. The rest is only
/// counted, over at most `limit` bytes of `input`; a row longer than [`RECORD_LIMIT`] bytes
/// ends the reading there, as the limit does.
pub fn parse(input: impl Read, delimiter: u8, budget: usize, limit: u64) -> io::Result<Sheet> {
    parse_with(input, delimiter, budget, limit, RECORD_LIMIT).map(|(sheet, _)| sheet)
}

/// [`parse`] with rows of at most `record_limit` bytes, their line break included. The input is
/// read through two `Take`s: the outer one holds the count limit, the inner one is set after
/// each row so the next one gets one byte more than `record_limit` (the csv reader's memory
/// grows with a row: its bytes and a `usize` a field); a row that took it is cut there. Also
/// how many bytes of `input` were read.
fn parse_with(
    input: impl Read,
    delimiter: u8,
    budget: usize,
    limit: u64,
    record_limit: u64,
) -> io::Result<(Sheet, u64)> {
    let most = limit.saturating_add(1);
    let per_row = record_limit.saturating_add(1);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(input.take(most).take(per_row));
    let given =
        |reader: &csv::Reader<io::Take<io::Take<_>>>| most - reader.get_ref().get_ref().limit();
    let mut record = csv::ByteRecord::new();
    let (mut rows, mut total, mut left, mut full) = (Vec::new(), 0, budget, false);
    let (mut start, mut cut) = (0, false);
    while reader
        .read_byte_record(&mut record)
        .map_err(io::Error::other)?
    {
        // A row over the limit is not one: the reading stops before it.
        let end = reader.position().byte();
        cut = end - start > record_limit;
        if cut {
            break;
        }
        start = end;
        let room = per_row.saturating_sub(given(&reader) - start);
        reader.get_mut().set_limit(room);
        total += 1;
        // Its JSON takes at least its bytes, two quotes and a comma a cell, and its brackets: a
        // row that cannot fit is never decoded.
        let least = (record.as_slice().len() + 2).saturating_add(record.len().saturating_mul(3));
        full = full || rows.len() == TABLE_ROWS || least > left;
        if full {
            continue;
        }
        let row: Vec<String> = record
            .iter()
            .map(|cell| String::from_utf8_lossy(cell).into_owned())
            .collect();
        // With the comma before it in the list.
        let size = serde_json::to_vec(&row).map_or(usize::MAX, |json| json.len().saturating_add(1));
        full = size > left;
        if !full {
            left -= size;
            rows.push(row);
        }
    }
    let read = given(&reader);
    let sheet = Sheet {
        name: String::new(),
        rows,
        total,
        partial: cut || read > limit,
    };
    Ok((sheet, read))
}

/// The `table` answer for `path`: its sheets, or why it could not be read.
pub fn message(worktree: String, path: String, read: io::Result<Vec<Sheet>>) -> Control {
    let (sheets, error) = match read {
        Ok(sheets) => (sheets, None),
        Err(err) => (Vec::new(), Some(err.to_string())),
    };
    Control::Table {
        worktree,
        path,
        sheets,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(input: &[u8], delimiter: u8) -> Sheet {
        parse(input, delimiter, MAX_PAYLOAD, COUNT_LIMIT).unwrap()
    }

    fn rows(input: &[u8]) -> Vec<Vec<String>> {
        sheet(input, b',').rows
    }

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|&c| c.to_owned()).collect()
    }

    #[test]
    fn table_files_are_known_by_their_extension() {
        assert_eq!(delimiter("a.csv"), Some(b','));
        assert_eq!(delimiter("dir/A.CSV"), Some(b','));
        assert_eq!(delimiter("b.tsv"), Some(b'\t'));
        assert_eq!(delimiter("b.Tsv"), Some(b'\t'));
        for path in ["a.txt", "csv", ".csv", "a.csv.bak", "a"] {
            assert_eq!(delimiter(path), None, "{path}");
        }
    }

    #[test]
    fn quoted_fields_keep_their_commas_quotes_and_newlines() {
        let input = b"name,note\r\n\"Doe, Jane\",\"say \"\"hi\"\"\"\n\"two\nlines\",x\n";
        assert_eq!(
            rows(input),
            [
                row(&["name", "note"]),
                row(&["Doe, Jane", "say \"hi\""]),
                row(&["two\nlines", "x"]),
            ]
        );
        assert_eq!(sheet(input, b',').total, 3);
    }

    #[test]
    fn a_tsv_splits_on_tabs_only() {
        let input = b"a\tb,c\n1\t\"2\t3\"\n";
        assert_eq!(
            sheet(input, b'\t').rows,
            [row(&["a", "b,c"]), row(&["1", "2\t3"])]
        );
    }

    #[test]
    fn ragged_rows_keep_their_own_length() {
        let input = b"a,b,c\n1\n1,2,3,4\n\n,\n";
        assert_eq!(
            rows(input),
            [
                row(&["a", "b", "c"]),
                row(&["1"]),
                row(&["1", "2", "3", "4"]),
                row(&["", ""]),
            ]
        );
    }

    #[test]
    fn a_file_not_in_utf8_shows_what_it_can() {
        // Latin-1 "café", and a UTF-8 byte order mark that is not shown.
        assert_eq!(rows(b"caf\xe9,ok\n"), [row(&["caf\u{fffd}", "ok"])]);
        assert_eq!(rows(b"\xef\xbb\xbfa,b\n"), [row(&["a", "b"])]);
        let nul = sheet(b"a\0b,c\n", b',');
        assert_eq!((nul.rows, nul.total), (vec![row(&["a\0b", "c"])], 1));
    }

    #[test]
    fn an_empty_file_has_no_rows() {
        let empty = sheet(b"", b',');
        assert_eq!(
            (empty.rows.len(), empty.total, empty.partial),
            (0, 0, false)
        );
    }

    #[test]
    fn at_most_the_first_rows_are_sent_and_every_row_counted() {
        let input: String = (0..TABLE_ROWS + 5).map(|i| format!("{i},x\n")).collect();
        let capped = sheet(input.as_bytes(), b',');
        assert_eq!(capped.rows.len(), TABLE_ROWS);
        assert_eq!(capped.rows[TABLE_ROWS - 1], row(&["9999", "x"]));
        assert_eq!(
            (capped.total, capped.partial),
            (TABLE_ROWS as u64 + 5, false)
        );
        let input: String = (0..TABLE_ROWS).map(|i| format!("{i}\n")).collect();
        let exact = sheet(input.as_bytes(), b',');
        assert_eq!(
            (exact.rows.len(), exact.total),
            (TABLE_ROWS, TABLE_ROWS as u64)
        );
    }

    #[test]
    fn rows_stop_where_the_frame_would_overflow() {
        // `["ab"]` and its comma: 7 bytes a row.
        let input = b"ab\nab\nab\n";
        let fits = |budget| parse(&input[..], b',', budget, COUNT_LIMIT).unwrap();
        assert_eq!(fits(21).rows.len(), 3);
        assert_eq!(fits(20).rows.len(), 2);
        assert_eq!(fits(14).rows.len(), 2);
        assert_eq!(fits(13).rows.len(), 1);
        assert_eq!(fits(6).total, 3);
        assert!(fits(6).rows.is_empty());
        // Once a row did not fit, a smaller one after it is not sent either.
        let after = parse(&b"abcdef\na\n"[..], b',', 8, COUNT_LIMIT).unwrap();
        assert_eq!((after.rows.len(), after.total), (0, 2));
    }

    #[test]
    fn past_the_count_limit_the_total_is_partial() {
        let input = b"a\nb\nc\n";
        let counted = |limit| parse(&input[..], b',', MAX_PAYLOAD, limit).unwrap();
        let all = counted(6);
        assert_eq!((all.rows.len(), all.total, all.partial), (3, 3, false));
        let cut = counted(5);
        assert_eq!((cut.total, cut.partial), (3, true));
        let cut = counted(3);
        assert_eq!(
            (cut.rows, cut.total, cut.partial),
            (vec![row(&["a"]), row(&["b"])], 2, true)
        );
    }

    #[test]
    fn a_row_past_the_record_limit_ends_the_reading_before_it() {
        let read = |input: &[u8], record| {
            parse_with(input, b',', MAX_PAYLOAD, COUNT_LIMIT, record).map(|(sheet, _)| sheet)
        };
        let cut = read(b"a,b\ncccccccccc\nd\n", 6).unwrap();
        assert_eq!(
            (cut.rows, cut.total, cut.partial),
            (vec![row(&["a", "b"])], 1, true)
        );
        // A row of exactly the limit is read whole.
        let whole = read(b"a,b\nccccc\n", 6).unwrap();
        assert_eq!(
            (whole.rows, whole.total, whole.partial),
            (vec![row(&["a", "b"]), row(&["ccccc"])], 2, false)
        );
        let whole = read(b"abcde\nabcde\n", 6).unwrap();
        assert_eq!((whole.total, whole.partial), (2, false));
        // A last row without its newline that fills the limit, then the end of the input.
        let last = read(b"a\nabcdef", 6).unwrap();
        assert_eq!(
            (last.rows, last.total, last.partial),
            (vec![row(&["a"]), row(&["abcdef"])], 2, false)
        );
        // Its newline past the limit: cut.
        let over = read(b"abcdef\n", 6).unwrap();
        assert_eq!((over.total, over.partial), (0, true));
    }

    #[test]
    fn a_row_is_read_no_further_than_one_byte_past_the_limit() {
        // After `a,b\n` (4 bytes), the long row gets 7 bytes: 11 read in all.
        let input = [b"a,b\n".as_slice(), &[b'c'; 20]].concat();
        let (sheet, read) = parse_with(&input[..], b',', MAX_PAYLOAD, COUNT_LIMIT, 6).unwrap();
        assert_eq!((sheet.total, sheet.partial, read), (1, true, 11));
        let (_, read) = parse_with(&b"ab\ncd\n"[..], b',', MAX_PAYLOAD, COUNT_LIMIT, 6).unwrap();
        assert_eq!(read, 6);
    }

    #[test]
    fn hostile_rows_cost_no_more_than_the_record_limit() {
        // Commas only (a field each), NUL bytes, invalid UTF-8: each a single row far over
        // the limit, first or after a row.
        let limit = RECORD_LIMIT as usize;
        for bytes in [
            vec![b','; 4 * limit],
            vec![0; 2 * limit],
            vec![0xff; 2 * limit],
        ] {
            let alone = sheet(&bytes, b',');
            assert_eq!((alone.rows.len(), alone.total, alone.partial), (0, 0, true));
            let after = sheet(&[b"a\n".as_slice(), &bytes].concat(), b',');
            assert_eq!(
                (after.rows, after.total, after.partial),
                (vec![row(&["a"])], 1, true)
            );
        }
    }

    #[test]
    fn a_row_that_cannot_fit_is_not_decoded() {
        // Its least JSON is exact for plain cells: `["ab","c"]` and its comma, 11 bytes.
        let fits = |budget| parse(&b"ab,c\n"[..], b',', budget, COUNT_LIMIT).unwrap();
        assert_eq!(fits(11).rows.len(), 1);
        assert!(fits(10).rows.is_empty());
        // Escapes make it longer than the least: decoded, then left out.
        let quoted = parse(&b"\"a\"\"\"\n"[..], b',', 7, COUNT_LIMIT).unwrap();
        assert_eq!((quoted.rows.len(), quoted.total), (0, 1));
        let quoted = parse(&b"\"a\"\"\"\n"[..], b',', 8, COUNT_LIMIT).unwrap();
        assert_eq!(quoted.rows, [row(&["a\""])]);
    }

    struct Broken;

    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("disk on fire"))
        }
    }

    #[test]
    fn a_read_error_says_so() {
        let err = parse(Broken, b',', MAX_PAYLOAD, COUNT_LIMIT).unwrap_err();
        assert!(err.to_string().contains("disk on fire"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_table_is_read_inside_the_worktree_only() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("s.csv"), "secret\n").unwrap();
        std::os::unix::fs::symlink(outside.path().join("s.csv"), dir.path().join("out.csv"))
            .unwrap();
        std::fs::write(dir.path().join("a.csv"), "a,b\n1,2\n").unwrap();
        std::fs::write(dir.path().join("a.txt"), "a,b\n").unwrap();
        std::fs::write(dir.path().join("a.xlsx"), "PK").unwrap();
        // A workbook goes to the worker, which is missing here.
        let worker = Worker::new("/nonexistent/hive".into());
        let read = |path: &str| read(dir.path(), path, MAX_PAYLOAD, &worker);
        let error = |path: &str| read(path).unwrap_err().to_string();
        assert_eq!(
            read("a.csv").unwrap()[0].rows,
            [row(&["a", "b"]), row(&["1", "2"])]
        );
        assert_eq!(error("a.txt"), "not a table file");
        assert_eq!(error("gone.xlsx"), "gone.xlsx does not exist");
        let missing = read("a.xlsx").unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        let long = format!("{}.csv", "a".repeat(5000));
        assert_eq!(error(&long), "not a relative path inside the worktree");
        assert_eq!(error("gone.csv"), "gone.csv does not exist");
        assert_eq!(error("out.csv"), "the file resolves outside the worktree");
        assert_eq!(error("../x.csv"), "not a relative path inside the worktree");
    }

    #[test]
    fn the_answer_holds_the_sheets_or_the_error() {
        let ok = Sheet {
            name: String::new(),
            rows: vec![row(&["a"])],
            total: 1,
            partial: false,
        };
        assert_eq!(
            message(
                "/w".into(),
                "a.csv".into(),
                Ok(vec![ok.clone(), ok.clone()])
            ),
            Control::Table {
                worktree: "/w".into(),
                path: "a.csv".into(),
                sheets: vec![ok.clone(), ok],
                error: None,
            }
        );
        assert_eq!(
            message("/w".into(), "a.csv".into(), Err(io::Error::other("no"))),
            Control::Table {
                worktree: "/w".into(),
                path: "a.csv".into(),
                sheets: Vec::new(),
                error: Some("no".into()),
            }
        );
    }

    #[test]
    fn the_rows_get_what_the_frame_leaves() {
        let envelope =
            r#"{"type":"table","worktree":"/w","path":"a.csv","sheets":[],"error":null}"#;
        assert_eq!(budget("/w", "a.csv"), MAX_PAYLOAD - envelope.len());
        let record = r#"{"name":"","rows":[],"total":18446744073709551615,"partial":false}"#;
        assert_eq!(super::record(), record.len());
        let huge = "x".repeat(MAX_PAYLOAD);
        assert_eq!(budget(&huge, ""), 0);
    }
}
