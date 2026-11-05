//! A CSV or TSV file of a worktree as a table (23.6): its first [`TABLE_ROWS`] rows and how
//! many it has, for the viewer's table. Read on disk with the file viewer's checks
//! ([`file::open_inside`]), whatever its size: a file over `file::TEXT_LIMIT` still shows its
//! first rows. A workbook (23.7) answers the same `table` message, one [`Sheet`] per sheet.

use std::io::{self, Read};
use std::path::Path;

use hive_protocol::{Control, MAX_PAYLOAD, Sheet};

use crate::file;
use crate::projects::Projects;

/// Most rows sent of a sheet.
pub const TABLE_ROWS: usize = 10_000;
/// Most bytes of a file read to count its rows (64 MiB); past them the count is partial.
pub const COUNT_LIMIT: u64 = 67_108_864;

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

/// `open_table`: `path` of `worktree` as a table, its rows within one frame.
pub fn answer(projects: &Projects, worktree: String, path: String) -> Control {
    let budget = budget(&worktree, &path);
    let read = projects
        .worktree(&worktree)
        .and_then(|dir| read(&dir, &path, budget));
    message(worktree, path, read)
}

/// The bytes of one frame left for the rows of the `table` answer for `path` of `worktree`.
fn budget(worktree: &str, path: &str) -> usize {
    let empty = Sheet {
        name: String::new(),
        rows: Vec::new(),
        total: u64::MAX,
        partial: false,
    };
    let envelope = message(worktree.into(), path.into(), Ok(empty));
    let used = serde_json::to_vec(&envelope).map_or(MAX_PAYLOAD, |json| json.len());
    MAX_PAYLOAD.saturating_sub(used)
}

/// The sheet of the table file `path` of the worktree at `dir` (checked as the viewer's
/// files are), its rows taking at most `budget` bytes of JSON.
pub fn read(dir: &Path, path: &str, budget: usize) -> io::Result<Sheet> {
    let Some(delimiter) = delimiter(path) else {
        return Err(io::Error::other(format!("{path} is not a CSV or TSV file")));
    };
    match file::open_inside(dir, path)? {
        Some(file) => parse(file, delimiter, budget, COUNT_LIMIT),
        None => Err(io::Error::other(format!("{path} does not exist"))),
    }
}

/// The sheet of CSV `input` (TSV with a tab `delimiter`), quoted fields as RFC 4180 has them:
/// rows of any length, each cell decoded as UTF-8 (an invalid byte shows as U+FFFD); at most
/// [`TABLE_ROWS`] rows, while their JSON takes at most `budget` bytes. The rest is only
/// counted, over at most `limit` bytes of `input`.
pub fn parse(input: impl Read, delimiter: u8, budget: usize, limit: u64) -> io::Result<Sheet> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(input.take(limit.saturating_add(1)));
    let mut record = csv::ByteRecord::new();
    let (mut rows, mut total, mut left, mut full) = (Vec::new(), 0, budget, false);
    while reader
        .read_byte_record(&mut record)
        .map_err(io::Error::other)?
    {
        total += 1;
        if full {
            continue;
        }
        let row: Vec<String> = record
            .iter()
            .map(|cell| String::from_utf8_lossy(cell).into_owned())
            .collect();
        // With the comma before it in the list.
        let size = serde_json::to_vec(&row).map_or(usize::MAX, |json| json.len().saturating_add(1));
        full = rows.len() == TABLE_ROWS || size > left;
        if !full {
            left -= size;
            rows.push(row);
        }
    }
    Ok(Sheet {
        name: String::new(),
        rows,
        total,
        partial: reader.position().byte() > limit,
    })
}

/// The `table` answer for `path`: its sheet, or why it could not be read.
pub fn message(worktree: String, path: String, read: io::Result<Sheet>) -> Control {
    let (sheets, error) = match read {
        Ok(sheet) => (vec![sheet], None),
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
        let read = |path: &str| read(dir.path(), path, MAX_PAYLOAD);
        let error = |path: &str| read(path).unwrap_err().to_string();
        assert_eq!(
            read("a.csv").unwrap().rows,
            [row(&["a", "b"]), row(&["1", "2"])]
        );
        assert_eq!(error("a.txt"), "a.txt is not a CSV or TSV file");
        assert_eq!(error("gone.csv"), "gone.csv does not exist");
        assert_eq!(error("out.csv"), "the file resolves outside the worktree");
        assert_eq!(error("../x.csv"), "not a relative path inside the worktree");
    }

    #[test]
    fn the_answer_holds_one_sheet_or_the_error() {
        let ok = Sheet {
            name: String::new(),
            rows: vec![row(&["a"])],
            total: 1,
            partial: false,
        };
        assert_eq!(
            message("/w".into(), "a.csv".into(), Ok(ok.clone())),
            Control::Table {
                worktree: "/w".into(),
                path: "a.csv".into(),
                sheets: vec![ok],
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
        let envelope = r#"{"type":"table","worktree":"/w","path":"a.csv","sheets":[{"name":"","rows":[],"total":18446744073709551615,"partial":false}],"error":null}"#;
        assert_eq!(budget("/w", "a.csv"), MAX_PAYLOAD - envelope.len());
        let huge = "x".repeat(MAX_PAYLOAD);
        assert_eq!(budget(&huge, ""), 0);
    }
}
