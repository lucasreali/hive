//! An Excel or OpenDocument workbook of a worktree as a table (23.7): one [`Sheet`] per
//! worksheet, read with `calamine`, its values as last computed (never the formulas).
//!
//! A workbook is untrusted input, and `calamine` sizes some of its memory by what the file
//! claims (an `.xls` sheet's dimensions reserve rows times columns at once; a sparse sheet is
//! stored dense; a zip member inflates without a cap). No check of the file can bound that, so
//! the service never parses one itself: it starts its own binary as `hive sheets <format>
//! <budget>` ([`worker`]), gives it the file's bytes on stdin (at most [`WORKBOOK_LIMIT`]) and
//! reads its answer. A child that crashes, takes more than [`MEMORY_LIMIT`] (Linux: its own
//! `RLIMIT_AS`; macOS: its footprint, watched) or more than [`TIME_LIMIT`] is the error "could
//! not be read", and the service keeps running.

use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use calamine::{Data, DataRef, ExcelDateTime, Ods, Range, Reader, SheetType, Xls, Xlsx};
use hive_protocol::Sheet;

use crate::table::TABLE_ROWS;

/// Largest workbook read, in bytes (32 MiB).
pub const WORKBOOK_LIMIT: u64 = 33_554_432;
/// Most memory the reading may take (1 GiB).
pub const MEMORY_LIMIT: u64 = 1_073_741_824;
/// Longest the reading may take.
pub const TIME_LIMIT: Duration = Duration::from_secs(30);
/// How often the reading child is checked.
const POLL: Duration = Duration::from_millis(20);
/// Most bytes of the child's answer read (8 MiB, two frames): past them it is cut, so not
/// JSON, and the child's next write fails.
const OUTPUT_LIMIT: u64 = 8_388_608;
/// Longest error kept from `calamine`, in characters (it may quote the file).
const ERROR_CHARS: usize = 200;

/// The formats read, by their extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Xlsx,
    Xls,
    Ods,
}

impl Format {
    /// The format named by `extension`, in any case.
    pub fn named(extension: &str) -> Option<Format> {
        match extension.to_ascii_lowercase().as_str() {
            "xlsx" => Some(Format::Xlsx),
            "xls" => Some(Format::Xls),
            "ods" => Some(Format::Ods),
            _ => None,
        }
    }

    /// The format of `path`, by its extension.
    pub fn of(path: &str) -> Option<Format> {
        Format::named(Path::new(path).extension()?.to_str()?)
    }

    fn name(self) -> &'static str {
        match self {
            Format::Xlsx => "xlsx",
            Format::Xls => "xls",
            Format::Ods => "ods",
        }
    }
}

/// How a workbook is read: the program run as `<program> <args…> sheets …` (the service's own
/// binary, with no arguments before), and the limits it runs within.
pub struct Worker {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub memory_limit: u64,
    pub time_limit: Duration,
    /// The memory a process takes, by its pid, where the service watches it (macOS).
    pub memory: Option<fn(u32) -> Option<u64>>,
}

impl Worker {
    /// `program` within [`MEMORY_LIMIT`] and [`TIME_LIMIT`].
    pub fn new(program: PathBuf) -> Worker {
        Worker {
            program,
            args: Vec::new(),
            memory_limit: MEMORY_LIMIT,
            time_limit: TIME_LIMIT,
            memory: FOOTPRINT,
        }
    }
}

#[cfg(target_os = "macos")]
const FOOTPRINT: Option<fn(u32) -> Option<u64>> = Some(crate::macos::footprint);
/// Linux bounds the child by its own `RLIMIT_AS`; Windows only by time.
#[cfg(not(target_os = "macos"))]
const FOOTPRINT: Option<fn(u32) -> Option<u64>> = None;

/// One workbook read at a time: a viewer asking again while agents change the worktree, or
/// several workbooks open, never run more than one child (and its memory) at once.
static READING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The sheets of the workbook `file` in `format`, their rows taking at most `budget` bytes of
/// JSON in all, read by `worker`.
pub fn read(
    file: impl Read,
    size: u64,
    format: Format,
    budget: usize,
    worker: &Worker,
) -> io::Result<Vec<Sheet>> {
    if size > WORKBOOK_LIMIT {
        return Err(io::Error::other(too_large()));
    }
    // A file that grew since its size was taken is cut, so broken.
    let mut bytes = Vec::new();
    file.take(WORKBOOK_LIMIT).read_to_end(&mut bytes)?;
    let mut command = Command::new(&worker.program);
    command.args(&worker.args);
    command.args(["sheets", format.name(), &budget.to_string()]);
    let out = {
        let _turn = READING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        run(command, bytes, worker)?
    };
    let answer: Result<Vec<Sheet>, String> =
        serde_json::from_slice(&out).map_err(|_| io::Error::other(UNREADABLE))?;
    answer.map_err(io::Error::other)
}

const UNREADABLE: &str = "the workbook could not be read";

fn too_large() -> String {
    format!("the workbook is larger than {} MiB", WORKBOOK_LIMIT >> 20)
}

/// What became of the child.
enum Outcome {
    Exited(ExitStatus),
    OverMemory,
    OverTime,
}

/// The output of `command` given `input` on stdin, at most [`OUTPUT_LIMIT`] of it, if it exits well
/// within the worker's limits.
fn run(mut command: Command, input: Vec<u8>, worker: &Worker) -> io::Result<Vec<u8>> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    // A child that stops reading makes the write fail once it is gone.
    let writer = std::thread::spawn(move || stdin.map(|mut pipe| pipe.write_all(&input)));
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let read = stdout.map(|pipe| pipe.take(OUTPUT_LIMIT).read_to_end(&mut out));
        read.map(|_| out)
    });
    let outcome = watch(&mut child, worker);
    if !matches!(outcome, Ok(Outcome::Exited(_))) {
        // Already gone if it exited meanwhile; reaped either way.
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = writer.join();
    let out = reader.join().ok().flatten();
    match outcome? {
        Outcome::Exited(status) if status.success() => {
            out.ok_or_else(|| io::Error::other(UNREADABLE))
        }
        Outcome::Exited(_) => Err(io::Error::other(UNREADABLE)),
        Outcome::OverMemory => Err(io::Error::other(format!(
            "{UNREADABLE}: it needs more than {} MiB of memory",
            worker.memory_limit >> 20
        ))),
        Outcome::OverTime => Err(io::Error::other(format!(
            "{UNREADABLE}: it takes longer than {:?}",
            worker.time_limit
        ))),
    }
}

/// Waits for `child` within the worker's limits, checking it every [`POLL`].
fn watch(child: &mut Child, worker: &Worker) -> io::Result<Outcome> {
    let polls = worker.time_limit.as_millis() / POLL.as_millis();
    for _ in 0..=polls {
        if let Some(status) = child.try_wait()? {
            return Ok(Outcome::Exited(status));
        }
        let used = worker.memory.and_then(|memory| memory(child.id()));
        if used.is_some_and(|used| used > worker.memory_limit) {
            return Ok(Outcome::OverMemory);
        }
        std::thread::sleep(POLL);
    }
    Ok(Outcome::OverTime)
}

/// `hive sheets <format> <budget>`: [`worker`] on stdin and stdout, within [`MEMORY_LIMIT`] on
/// Linux.
pub fn child(format: &str, budget: usize) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    limit_memory()?;
    worker(format, budget, io::stdin(), io::stdout())
}

/// The workbook on `input` read as
/// [`sheets`] does, the answer (its sheets, or why not) written to `output` as JSON.
pub fn worker(format: &str, budget: usize, input: impl Read, output: impl Write) -> io::Result<()> {
    let format =
        Format::named(format).ok_or_else(|| io::Error::other("unknown workbook format"))?;
    let mut bytes = Vec::new();
    input.take(WORKBOOK_LIMIT + 1).read_to_end(&mut bytes)?;
    let answer = if bytes.len() as u64 > WORKBOOK_LIMIT {
        Err(too_large())
    } else {
        sheets(bytes, format, budget)
    };
    serde_json::to_writer(output, &answer).map_err(io::Error::other)
}

/// The child's address space capped at [`MEMORY_LIMIT`] (or the lower limit it already has):
/// an allocation past it fails, and the child ends, leaving no core dump of the workbook.
#[cfg(target_os = "linux")]
fn limit_memory() -> io::Result<()> {
    use nix::sys::resource::{Resource, getrlimit, setrlimit};
    setrlimit(Resource::RLIMIT_CORE, 0, 0)?;
    let (_, hard) = getrlimit(Resource::RLIMIT_AS)?;
    let most = MEMORY_LIMIT.min(hard);
    Ok(setrlimit(Resource::RLIMIT_AS, most, most)?)
}

/// The worksheets of the workbook `bytes` in `format`, in their order (chart, dialog and macro
/// sheets left out, hidden ones kept), their rows from A1 taking at most `budget` bytes of
/// JSON in all, sheet records included; a sheet that does not fit, and those after it, are
/// left out.
pub fn sheets(bytes: Vec<u8>, format: Format, budget: usize) -> Result<Vec<Sheet>, String> {
    let reader = Cursor::new(bytes);
    let mut left = budget;
    let mut sheets = Vec::new();
    match format {
        Format::Xlsx => {
            let mut book = Xlsx::new(reader).map_err(broken)?;
            for name in worksheets(book.sheets_metadata()) {
                let mut cells = book.worksheet_cells_reader(&name).map_err(broken)?;
                let mut grid = Grid::new(left);
                while let Some(cell) = cells.next_cell().map_err(broken)? {
                    let (row, col) = cell.get_position();
                    grid.add(row, col, text_ref(cell.get_value()));
                }
                if !grid.into_sheet(name, &mut left, &mut sheets) {
                    break;
                }
            }
        }
        Format::Xls => {
            let mut book = Xls::new(reader).map_err(broken)?;
            for name in worksheets(book.sheets_metadata()) {
                let range = book.worksheet_range(&name).map_err(broken)?;
                if !from_range(&range, left).into_sheet(name, &mut left, &mut sheets) {
                    break;
                }
            }
        }
        Format::Ods => {
            let mut book = Ods::new(reader).map_err(broken)?;
            for name in worksheets(book.sheets_metadata()) {
                let range = book.worksheet_range(&name).map_err(broken)?;
                if !from_range(&range, left).into_sheet(name, &mut left, &mut sheets) {
                    break;
                }
            }
        }
    }
    Ok(sheets)
}

fn worksheets(sheets: &[calamine::Sheet]) -> Vec<String> {
    sheets
        .iter()
        .filter(|sheet| sheet.typ == SheetType::WorkSheet)
        .map(|sheet| sheet.name.clone())
        .collect()
}

/// A broken workbook's error, short.
fn broken(err: impl std::fmt::Display) -> String {
    let text: String = format!("not a readable workbook: {err}");
    text.chars().take(ERROR_CHARS).collect()
}

fn from_range(range: &Range<Data>, budget: usize) -> Grid {
    let mut grid = Grid::new(budget);
    let (top, left) = range.start().unwrap_or_default();
    for (row, col, value) in range.used_cells() {
        let at = |start: u32, offset: usize| start.saturating_add(offset as u32);
        grid.add(at(top, row), at(left, col), text(value));
    }
    grid
}

/// A sheet's cells as they are read: how many rows it has from A1, and the text of those
/// among the first [`TABLE_ROWS`] while their least JSON fits the budget. Once a cell does not
/// fit, no other is kept, and the rows shown end before the first row that lost one: in any
/// order the cells come, a row shown has all of its own.
struct Grid {
    rows: BTreeMap<u32, BTreeMap<u32, String>>,
    total: u64,
    /// The bytes the kept cells may still take: their text, two quotes and a comma each.
    room: usize,
    /// The first row that lost a cell, once one did.
    cut: Option<u32>,
}

impl Grid {
    fn new(budget: usize) -> Grid {
        Grid {
            rows: BTreeMap::new(),
            total: 0,
            room: budget,
            cut: None,
        }
    }

    /// The cell at `row` and `col` (from 0), unless empty.
    fn add(&mut self, row: u32, col: u32, text: String) {
        if text.is_empty() {
            return;
        }
        self.total = self.total.max(u64::from(row) + 1);
        if row as usize >= TABLE_ROWS {
            return;
        }
        let cost = text.len().saturating_add(3);
        if self.cut.is_some() || cost > self.room {
            self.cut = Some(self.cut.map_or(row, |cut| cut.min(row)));
            return;
        }
        self.room -= cost;
        self.rows.entry(row).or_default().insert(col, text);
    }

    /// Adds this sheet named `name` to `sheets` if its record fits in `left`, with the rows
    /// that fit after it; whether it did.
    fn into_sheet(self, name: String, left: &mut usize, sheets: &mut Vec<Sheet>) -> bool {
        let mut sheet = Sheet {
            name,
            rows: Vec::new(),
            total: self.total,
            partial: false,
        };
        // With the comma before it in the list.
        let shell =
            serde_json::to_vec(&sheet).map_or(usize::MAX, |json| json.len().saturating_add(1));
        if shell > *left {
            return false;
        }
        *left -= shell;
        let shown = self.total.min(TABLE_ROWS as u64) as u32;
        let shown = self.cut.map_or(shown, |cut| cut.min(shown));
        let mut kept = self.rows;
        for at in 0..shown {
            let cells = kept.remove(&at).unwrap_or_default();
            let (width, least) = least(&cells);
            if least > *left {
                break;
            }
            let mut row = vec![String::new(); width];
            for (col, text) in cells {
                row[col as usize] = text;
            }
            let size =
                serde_json::to_vec(&row).map_or(usize::MAX, |json| json.len().saturating_add(1));
            if size > *left {
                break;
            }
            *left -= size;
            sheet.rows.push(row);
        }
        sheets.push(sheet);
        true
    }
}

/// A row's width (its last cell's column, plus one) and the least bytes its JSON takes, its
/// comma included: its text, two quotes and a comma a cell, its brackets. Checked before the
/// row is made: a cell far to the right makes a wide row.
fn least(cells: &BTreeMap<u32, String>) -> (usize, usize) {
    let width = cells
        .last_key_value()
        .map_or(0, |(&col, _)| col as usize + 1);
    let text = cells.values().map(String::len).sum::<usize>();
    (width, text + width * 3 + 2)
}

fn text_ref(value: &DataRef<'_>) -> String {
    match value {
        DataRef::SharedString(shared) => (*shared).to_owned(),
        other => text(&Data::from(other.clone())),
    }
}

/// A cell's value as the viewer shows it: numbers to Excel's 15 significant digits, dates as
/// `YYYY-MM-DD`, times as `HH:MM:SS`, both as `YYYY-MM-DD HH:MM:SS`, durations as `H:MM:SS`,
/// booleans as `TRUE` / `FALSE`, errors as Excel writes them (`#DIV/0!`).
pub fn text(value: &Data) -> String {
    match value {
        Data::Int(int) => int.to_string(),
        Data::Float(float) => number(*float),
        Data::String(string) => string.clone(),
        Data::Bool(true) => "TRUE".to_owned(),
        Data::Bool(false) => "FALSE".to_owned(),
        Data::DateTime(date) => date_time(date),
        Data::DateTimeIso(iso) => iso.replacen('T', " ", 1),
        Data::DurationIso(iso) => iso_duration(iso).unwrap_or_else(|| iso.clone()),
        Data::Error(error) => error.to_string(),
        Data::Empty => String::new(),
    }
}

/// `value` to 15 significant digits, as Excel shows it, in its shortest form.
fn number(value: f64) -> String {
    let rounded: f64 = format!("{value:.14e}").parse().unwrap_or(value);
    // Never `-0`.
    (rounded + 0.0).to_string()
}

fn date_time(date: &ExcelDateTime) -> String {
    if date.is_duration() {
        return duration(date.as_f64() * 86_400.0);
    }
    let (year, month, day, hour, minute, second, milli) = date.to_ymd_hms_milli();
    let mut time = format!("{hour:02}:{minute:02}:{second:02}");
    if milli > 0 {
        time.push_str(&format!(".{milli:03}"));
    }
    let day = format!("{year:04}-{month:02}-{day:02}");
    if (0.0..1.0).contains(&date.as_f64()) {
        time
    } else if time == "00:00:00" {
        day
    } else {
        format!("{day} {time}")
    }
}

/// `seconds` as `H:MM:SS`, hours past 24 kept.
fn duration(seconds: f64) -> String {
    let sign = if seconds < 0.0 { "-" } else { "" };
    let total = seconds.abs().round() as u64;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    format!("{sign}{hours}:{minutes:02}:{seconds:02}")
}

/// An ISO 8601 duration of OpenDocument (`PT12H30M00S`, `P1DT2H`, `-PT1M`) as [`duration`]
/// writes it, or `None` if it has years or months, or is not one.
fn iso_duration(iso: &str) -> Option<String> {
    let (sign, rest) = match iso.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, iso),
    };
    let rest = rest.strip_prefix('P')?;
    let (days, time) = rest.split_once('T').unwrap_or((rest, ""));
    let mut seconds = match days {
        "" => 0.0,
        days => days.strip_suffix('D')?.parse::<f64>().ok()? * 86_400.0,
    };
    let mut number = String::new();
    for c in time.chars() {
        let unit = match c {
            'H' => 3600.0,
            'M' => 60.0,
            'S' => 1.0,
            _ => {
                number.push(c);
                continue;
            }
        };
        seconds += std::mem::take(&mut number).parse::<f64>().ok()? * unit;
    }
    number.is_empty().then(|| duration(sign * seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use calamine::{CellErrorType, ExcelDateTimeType};
    use hive_protocol::MAX_PAYLOAD;

    const XLSX: &[u8] = include_bytes!("../tests/fixtures/workbooks/book.xlsx");
    const XLS: &[u8] = include_bytes!("../tests/fixtures/workbooks/book.xls");
    const ODS: &[u8] = include_bytes!("../tests/fixtures/workbooks/book.ods");

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|&c| c.to_owned()).collect()
    }

    fn book() -> Vec<Sheet> {
        vec![
            Sheet {
                name: "Orders".into(),
                rows: vec![
                    row(&["Item", "Date", "Amount", "Note"]),
                    row(&["Pen", "2024-03-15", "", "blue"]),
                    row(&["Ink", "2024-03-16 14:30:00", "7", "TRUE"]),
                    row(&["Total", "", "14"]),
                ],
                total: 4,
                partial: false,
            },
            Sheet {
                name: "Notes".into(),
                rows: vec![row(&["Hello"]), row(&["12:00:00"])],
                total: 2,
                partial: false,
            },
        ]
    }

    #[test]
    fn workbooks_are_known_by_their_extension() {
        assert_eq!(Format::of("a.xlsx"), Some(Format::Xlsx));
        assert_eq!(Format::of("d/A.XLS"), Some(Format::Xls));
        assert_eq!(Format::of("b.Ods"), Some(Format::Ods));
        for path in ["a.csv", "xlsx", ".xlsx", "a.xlsx.bak", "a", "a.xlsm"] {
            assert_eq!(Format::of(path), None, "{path}");
        }
        for format in [Format::Xlsx, Format::Xls, Format::Ods] {
            assert_eq!(Format::named(format.name()), Some(format));
        }
    }

    #[test]
    fn each_format_reads_two_sheets_with_dates_and_an_empty_cell() {
        for (bytes, format) in [(XLSX, Format::Xlsx), (XLS, Format::Xls), (ODS, Format::Ods)] {
            assert_eq!(
                sheets(bytes.to_vec(), format, MAX_PAYLOAD),
                Ok(book()),
                "{format:?}"
            );
        }
    }

    #[test]
    fn a_broken_workbook_says_so() {
        for format in [Format::Xlsx, Format::Xls, Format::Ods] {
            let err = sheets(b"not a workbook".to_vec(), format, MAX_PAYLOAD).unwrap_err();
            assert!(err.starts_with("not a readable workbook: "), "{err}");
            assert!(err.chars().count() <= ERROR_CHARS, "{err}");
        }
        // An xlsx read as an ods: its parts are not there.
        let err = sheets(XLSX.to_vec(), Format::Ods, MAX_PAYLOAD).unwrap_err();
        assert!(err.starts_with("not a readable workbook: "), "{err}");
        let long = broken("x".repeat(1000));
        assert_eq!(long.chars().count(), ERROR_CHARS);
    }

    fn size(value: &impl serde::Serialize) -> usize {
        serde_json::to_vec(value).unwrap().len()
    }

    #[test]
    fn sheets_share_the_budget_and_stop_where_it_ends() {
        let all = book();
        // Each sheet's record and row with its comma.
        let shell = |sheet: &Sheet| {
            let empty = Sheet {
                rows: Vec::new(),
                ..sheet.clone()
            };
            size(&empty) + 1
        };
        let rows = |sheet: &Sheet| sheet.rows.iter().map(|r| size(r) + 1).sum::<usize>();
        let exact = shell(&all[0]) + rows(&all[0]) + shell(&all[1]) + rows(&all[1]);
        let read = |budget| sheets(XLSX.to_vec(), Format::Xlsx, budget).unwrap();
        assert_eq!(read(exact), all);
        // A byte short: the last row of the last sheet is left out.
        let short = read(exact - 1);
        assert_eq!(
            (&short[0], &short[1].rows),
            (&all[0], &vec![row(&["Hello"])])
        );
        assert_eq!(short[1].total, 2);
        // Room for the first sheet and only the second's record.
        let first = shell(&all[0]) + rows(&all[0]);
        let bare = read(first + shell(&all[1]));
        assert_eq!((bare.len(), bare[1].rows.len()), (2, 0));
        // Not even that: the second sheet is left out.
        assert_eq!(read(first + shell(&all[1]) - 1).len(), 1);
        assert!(read(0).is_empty());
        for (bytes, format) in [(XLS, Format::Xls), (ODS, Format::Ods)] {
            let one = sheets(bytes.to_vec(), format, first + shell(&all[1]) - 1).unwrap();
            assert_eq!(one, all[..1], "{format:?}");
        }
    }

    /// The rows of `grid` as one sheet, given `budget` bytes past its record.
    fn shown(grid: Grid, budget: usize) -> Vec<Vec<String>> {
        let record = Sheet {
            name: String::new(),
            rows: Vec::new(),
            total: grid.total,
            partial: false,
        };
        let (mut left, mut sheets) = (size(&record) + 1 + budget, Vec::new());
        assert!(grid.into_sheet(String::new(), &mut left, &mut sheets));
        sheets.remove(0).rows
    }

    #[test]
    fn rows_are_kept_from_a1_up_to_the_row_cap() {
        let mut grid = Grid::new(MAX_PAYLOAD);
        grid.add(2, 1, "b3".into());
        grid.add(0, 0, String::new());
        grid.add(TABLE_ROWS as u32 + 5, 0, "far".into());
        assert_eq!(grid.total, TABLE_ROWS as u64 + 6);
        let rows = shown(grid, MAX_PAYLOAD);
        assert_eq!(rows.len(), TABLE_ROWS);
        assert_eq!(rows[..3], [row(&[]), row(&[]), row(&["", "b3"])]);
        let mut exact = Grid::new(MAX_PAYLOAD);
        exact.add(TABLE_ROWS as u32 - 1, 0, "z".into());
        assert_eq!(shown(exact, MAX_PAYLOAD)[TABLE_ROWS - 1], row(&["z"]));
    }

    #[test]
    fn cells_past_the_budget_are_counted_not_kept() {
        // "abc" and its quotes and comma: 6 bytes.
        let mut grid = Grid::new(11);
        grid.add(0, 0, "abc".into());
        grid.add(1, 1, "abc".into());
        grid.add(1, 0, "a".into());
        grid.add(2, 0, "ab".into());
        assert_eq!((grid.total, grid.room, grid.cut), (3, 5, Some(1)));
        assert_eq!(grid.rows.len(), 1);
        // A row that lost a cell is not shown, nor those after it.
        assert_eq!(shown(grid, MAX_PAYLOAD), [row(&["abc"])]);
        // The cut is the first row that lost one, in whatever order they come.
        let mut grid = Grid::new(6);
        grid.add(5, 0, "abc".into());
        grid.add(4, 0, "abc".into());
        grid.add(2, 0, "abc".into());
        grid.add(3, 0, "abc".into());
        assert_eq!((grid.room, grid.cut), (0, Some(2)));
        assert_eq!(shown(grid, MAX_PAYLOAD), [row(&[]), row(&[])]);
        let mut grid = Grid::new(12);
        grid.add(0, 0, "abc".into());
        grid.add(1, 0, "abc".into());
        assert_eq!((grid.rows.len(), grid.room, grid.cut), (2, 0, None));
    }

    #[test]
    fn a_rows_least_json_is_counted_from_its_cells() {
        let cells = |list: &[(u32, &str)]| -> BTreeMap<u32, String> {
            list.iter()
                .map(|&(col, text)| (col, text.to_owned()))
                .collect()
        };
        assert_eq!(least(&cells(&[])), (0, 2));
        // `["ab","","c"]` and its comma: 14 bytes.
        assert_eq!(least(&cells(&[(0, "ab"), (2, "c")])), (3, 14));
        assert_eq!(least(&cells(&[(9, "")])), (10, 32));
    }

    #[test]
    fn a_wide_row_that_cannot_fit_is_never_made() {
        // `["","x"]` and its comma: 9 bytes, its least too; `["y"]`, 6.
        let grid = || {
            let mut grid = Grid::new(MAX_PAYLOAD);
            grid.add(0, 1, "x".into());
            grid.add(1, 0, "y".into());
            grid
        };
        assert_eq!(shown(grid(), 9 + 6), [row(&["", "x"]), row(&["y"])]);
        assert_eq!(shown(grid(), 9 + 5), [row(&["", "x"])]);
        assert_eq!(shown(grid(), 9), [row(&["", "x"])]);
        assert!(shown(grid(), 8).is_empty());
        // Escapes make its JSON longer than its least (6): made, then left out.
        let quote = || {
            let mut grid = Grid::new(MAX_PAYLOAD);
            grid.add(0, 0, "\"".into());
            grid
        };
        assert_eq!(shown(quote(), 7).len(), 1);
        assert!(shown(quote(), 6).is_empty());
        assert!(shown(quote(), 5).is_empty());
    }

    #[test]
    fn values_show_as_computed() {
        assert_eq!(text(&Data::Int(-3)), "-3");
        assert_eq!(text(&Data::Float(7.0)), "7");
        assert_eq!(text(&Data::Float(0.1 + 0.2)), "0.3");
        assert_eq!(text(&Data::Float(-0.0)), "0");
        assert_eq!(text(&Data::Float(1.5e-7)), "0.00000015");
        assert_eq!(
            text(&Data::Float(123_456_789.123_456_78)),
            "123456789.123457"
        );
        assert_eq!(text(&Data::String("a".into())), "a");
        assert_eq!(text(&Data::Bool(true)), "TRUE");
        assert_eq!(text(&Data::Bool(false)), "FALSE");
        assert_eq!(text(&Data::Error(CellErrorType::Div0)), "#DIV/0!");
        assert_eq!(text(&Data::Empty), "");
        assert_eq!(
            text(&Data::DateTimeIso("2024-03-16T14:30:00".into())),
            "2024-03-16 14:30:00"
        );
        assert_eq!(text(&Data::DateTimeIso("2024-03-16".into())), "2024-03-16");
        assert_eq!(text_ref(&DataRef::SharedString("s")), "s");
        assert_eq!(text_ref(&DataRef::Int(2)), "2");
    }

    #[test]
    fn dates_and_times_show_as_iso_dates_and_clock_times() {
        let at =
            |value, kind, is_1904| text(&Data::DateTime(ExcelDateTime::new(value, kind, is_1904)));
        let date = |value| at(value, ExcelDateTimeType::DateTime, false);
        assert_eq!(date(45366.0), "2024-03-15");
        assert_eq!(date(45366.75), "2024-03-15 18:00:00");
        assert_eq!(date(45_366.000_005_787), "2024-03-15 00:00:00.500");
        assert_eq!(date(0.5), "12:00:00");
        assert_eq!(date(0.0), "00:00:00");
        assert_eq!(date(1.0), "1900-01-01");
        assert_eq!(date(60.0), "1900-02-29");
        assert_eq!(
            at(0.0 + 1.0, ExcelDateTimeType::DateTime, true),
            "1904-01-02"
        );
        let span = |value| at(value, ExcelDateTimeType::TimeDelta, false);
        assert_eq!(span(1.5), "36:00:00");
        assert_eq!(span(-0.25), "-6:00:00");
        assert_eq!(span(0.000_011_574), "0:00:01");
    }

    #[test]
    fn opendocument_durations_show_as_clock_times() {
        let shown = |iso: &str| text(&Data::DurationIso(iso.into()));
        assert_eq!(shown("PT12H30M00S"), "12:30:00");
        assert_eq!(shown("PT1.6S"), "0:00:02");
        assert_eq!(shown("P1DT2H"), "26:00:00");
        assert_eq!(shown("P2D"), "48:00:00");
        assert_eq!(shown("-PT1M"), "-0:01:00");
        assert_eq!(shown("PT"), "0:00:00");
        for odd in ["P1Y", "PT1X", "PT5", "12:00", "PTxH", "PxD", "P1"] {
            assert_eq!(shown(odd), odd, "{odd}");
        }
    }

    fn worker_answer(format: &str, input: &[u8]) -> Result<Vec<Sheet>, String> {
        let mut out = Vec::new();
        worker(format, MAX_PAYLOAD, input, &mut out).unwrap();
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn the_worker_answers_its_sheets_or_why_not() {
        assert_eq!(worker_answer("xlsx", XLSX), Ok(book()));
        assert_eq!(worker_answer("ODS", ODS), Ok(book()));
        assert!(
            worker_answer("xls", b"junk")
                .unwrap_err()
                .starts_with("not a readable")
        );
        let err = worker("csv", MAX_PAYLOAD, &b""[..], Vec::new()).unwrap_err();
        assert_eq!(err.to_string(), "unknown workbook format");
    }

    #[test]
    fn the_worker_reads_no_more_than_the_limit() {
        struct Endless(u64);
        impl Read for Endless {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.0 += buf.len() as u64;
                buf.fill(0);
                Ok(buf.len())
            }
        }
        let mut input = Endless(0);
        let mut out = Vec::new();
        worker("xlsx", MAX_PAYLOAD, &mut input, &mut out).unwrap();
        let answer: Result<Vec<Sheet>, String> = serde_json::from_slice(&out).unwrap();
        assert_eq!(answer, Err("the workbook is larger than 32 MiB".into()));
        assert!(input.0 <= WORKBOOK_LIMIT + 1 + 65_536, "{}", input.0);
        let exact = vec![0; WORKBOOK_LIMIT as usize];
        assert!(
            worker_answer("xlsx", &exact)
                .unwrap_err()
                .starts_with("not a readable")
        );
    }

    #[cfg(unix)]
    mod child {
        use super::*;

        /// A worker running `script` with `sh -c`, `sheets <format> <budget>` as `$1 $2 $3`
        /// (the system's own `sh`: no new executable for macOS to scan first).
        fn shell(script: &str) -> Worker {
            Worker {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into(), "sh".into()],
                memory_limit: MEMORY_LIMIT,
                time_limit: Duration::from_secs(10),
                memory: None,
            }
        }

        fn read_with(worker: &Worker, input: &[u8]) -> io::Result<Vec<Sheet>> {
            read(input, input.len() as u64, Format::Ods, 77, worker)
        }

        #[test]
        fn the_childs_answer_is_the_reading() {
            let sheet = r#"{"Ok":[{"name":"S","rows":[["a"]],"total":1,"partial":false}]}"#;
            let script = format!(
                "[ \"$(cat)\" = bytes ] && [ \"$1 $2 $3\" = 'sheets ods 77' ] && echo '{sheet}'"
            );
            let echo = shell(&script);
            let sheets = read_with(&echo, b"bytes").unwrap();
            assert_eq!(sheets[0].rows, [row(&["a"])]);
            let refused = shell(r#"cat > /dev/null; echo '{"Err":"no good"}'"#);
            assert_eq!(read_with(&refused, b"").unwrap_err().to_string(), "no good");
        }

        #[test]
        fn a_child_that_fails_or_says_nonsense_could_not_read_it() {
            for script in [
                "exit 3",
                r#"echo '{"Ok":[]}'; exit 3"#,
                "kill -9 $$",
                "echo nonsense",
                "head -c 9000000 /dev/zero",
            ] {
                let worker = shell(script);
                let err = read_with(&worker, b"x").unwrap_err();
                assert_eq!(err.to_string(), UNREADABLE, "{script}");
            }
            // It never reads its input: the write fails, the answer stands.
            let deaf = shell(r#"echo '{"Ok":[]}'"#);
            assert_eq!(read_with(&deaf, &vec![0; 4 << 20]).unwrap(), []);
        }

        #[test]
        fn a_child_past_its_limits_is_stopped() {
            let mut slow = shell("exec sleep 30");
            slow.time_limit = Duration::from_millis(100);
            let started = std::time::Instant::now();
            let err = read_with(&slow, b"").unwrap_err().to_string();
            assert_eq!(err, format!("{UNREADABLE}: it takes longer than 100ms"));
            assert!(started.elapsed() < Duration::from_secs(10));
            let mut hungry = shell("exec sleep 30");
            hungry.memory = Some(|_| Some(MEMORY_LIMIT + 1));
            let err = read_with(&hungry, b"").unwrap_err().to_string();
            assert_eq!(
                err,
                format!("{UNREADABLE}: it needs more than 1024 MiB of memory")
            );
            // At the limit is within it.
            let mut full = shell(r#"echo '{"Ok":[]}'"#);
            full.memory = Some(|_| Some(MEMORY_LIMIT));
            assert_eq!(read_with(&full, b"").unwrap(), []);
        }

        #[test]
        fn a_workbook_over_the_limit_is_not_read() {
            let never = shell("exit 1");
            let err = read(&b""[..], WORKBOOK_LIMIT + 1, Format::Xlsx, 0, &never).unwrap_err();
            assert_eq!(err.to_string(), "the workbook is larger than 32 MiB");
            // At the limit is read.
            let empty = shell(r#"echo '{"Ok":[]}'"#);
            assert_eq!(
                read(&b""[..], WORKBOOK_LIMIT, Format::Xlsx, 0, &empty).unwrap(),
                []
            );
            let missing = Worker::new("/nonexistent/hive".into());
            let err = read(&b""[..], 0, Format::Xlsx, 0, &missing).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotFound);
            // At most the limit is given, even when the file grew since its size was taken.
            let echo = shell(r#"[ "$(wc -c)" -eq 33554432 ] && echo '{"Ok":[]}'"#);
            assert_eq!(
                read(io::repeat(b'x'), 0, Format::Xlsx, 0, &echo).unwrap(),
                []
            );
        }

        #[test]
        fn an_answer_is_read_up_to_two_frames() {
            // A valid answer of `size` bytes: a sheet named with that many `a`s, less its JSON.
            let empty = r#"{"Ok":[{"name":"","rows":[],"total":0,"partial":false}]}"#.len();
            let answer = |size: usize| {
                let name = size - empty;
                shell(&format!(
                    r#"cat > /dev/null; printf '{{"Ok":[{{"name":"'; head -c {name} /dev/zero | tr '\0' a; printf '","rows":[],"total":0,"partial":false}}]}}'"#
                ))
            };
            let big = answer(OUTPUT_LIMIT as usize);
            assert_eq!(
                read_with(&big, b"").unwrap()[0].name.len(),
                OUTPUT_LIMIT as usize - empty
            );
            let over = answer(OUTPUT_LIMIT as usize + 1);
            assert_eq!(read_with(&over, b"").unwrap_err().to_string(), UNREADABLE);
        }
    }
}
