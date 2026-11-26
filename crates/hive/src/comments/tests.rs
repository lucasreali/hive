use std::cell::RefCell;
use std::collections::HashMap;

use super::*;

/// A worktree's files in memory, for the comments to read; a file set to `Err` cannot be read.
#[derive(Default)]
struct Files(RefCell<HashMap<String, io::Result<Vec<u8>>>>);

impl Files {
    fn set(&self, file: &str, text: &str) {
        self.0
            .borrow_mut()
            .insert(file.to_owned(), Ok(text.as_bytes().to_vec()));
    }

    fn unreadable(&self, file: &str) {
        let err = io::Error::other("not a regular file");
        self.0.borrow_mut().insert(file.to_owned(), Err(err));
    }

    fn remove(&self, file: &str) {
        self.0.borrow_mut().remove(file);
    }

    fn read(&self, file: &str) -> io::Result<Option<Vec<u8>>> {
        match self.0.borrow().get(file) {
            None => Ok(None),
            Some(Ok(bytes)) => Ok(Some(bytes.clone())),
            Some(Err(err)) => Err(io::Error::other(err.to_string())),
        }
    }
}

struct Fixture {
    tmp: tempfile::TempDir,
    worktree: String,
    files: Files,
    comments: Comments,
}

impl Fixture {
    fn new() -> Self {
        Self::with_limits(FILE_LIMIT, 3)
    }

    fn with_limits(limit: u64, most: usize) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("wt");
        std::fs::create_dir(&dir).unwrap();
        let comments = Comments::with_limits(tmp.path().join("hive/comments.json"), limit, most);
        Self {
            worktree: dir.display().to_string(),
            tmp,
            files: Files::default(),
            comments,
        }
    }

    fn store(&self) -> PathBuf {
        self.tmp.path().join("hive/comments.json")
    }

    fn handle(&self, request: Request) -> io::Result<Vec<ReviewComment>> {
        let read = |file: &str| self.files.read(file);
        self.comments.handle(&self.worktree, &read, request)
    }

    fn list(&self) -> Vec<ReviewComment> {
        self.handle(Request::List).unwrap()
    }

    fn add(&self, file: &str, from: u32, to: u32, body: &str) -> io::Result<Vec<ReviewComment>> {
        self.handle(Request::Add {
            file: file.to_owned(),
            lines: (from, to),
            body: body.to_owned(),
            author: "ana".to_owned(),
            ms: 42,
        })
    }

    /// Each comment's place and whether it is outdated.
    fn places(&self) -> Vec<(u32, u32, bool)> {
        let comments = self.list();
        comments
            .iter()
            .map(|c| (c.from, c.to, c.outdated))
            .collect()
    }
}

/// `n` lines `line 1` … `line n`, each with enough text to be found on its own.
fn numbered(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("let line_{i} = {i};")).collect()
}

fn text(lines: &[String]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn strings(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|&l| l.to_owned()).collect()
}

#[test]
fn a_comment_keeps_its_lines_text_context_and_author_across_restarts() {
    let fx = Fixture::new();
    fx.files.set("src/a.rs", &text(&numbered(10)));
    let added = fx.add("src/a.rs", 4, 5, "  why two?\n").unwrap();
    let expected = ReviewComment {
        id: 1,
        file: "src/a.rs".into(),
        from: 4,
        to: 5,
        body: "why two?".into(),
        code: strings(&["let line_4 = 4;", "let line_5 = 5;"]),
        before: strings(&["let line_1 = 1;", "let line_2 = 2;", "let line_3 = 3;"]),
        after: strings(&["let line_6 = 6;", "let line_7 = 7;", "let line_8 = 8;"]),
        author: "ana".into(),
        ms: 42,
        resolved: false,
        outdated: false,
    };
    assert_eq!(added, vec![expected.clone()]);
    assert!(fx.comments.has(&fx.worktree));
    assert!(!fx.comments.has("/elsewhere"));
    // Kept in Hive's folder: another service run lists it.
    let again = Comments::new(fx.store());
    assert_eq!(again.most, 500);
    let read = |file: &str| fx.files.read(file);
    assert_eq!(
        again.handle(&fx.worktree, &read, Request::List).unwrap(),
        vec![expected]
    );
    // A second comment gets the next id, and both are listed oldest first.
    let both = fx.add("src/a.rs", 1, 1, "first line").unwrap();
    assert_eq!(both.iter().map(|c| c.id).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(both[1].before, Vec::<String>::new());
}

#[test]
fn a_comment_follows_its_code_through_edits_above_and_below() {
    let fx = Fixture::new();
    let mut lines = numbered(20);
    lines[9] = "}".into();
    fx.files.set("a.rs", &text(&lines));
    // A lone `}` cannot be found by its text: the diff of the kept hashes follows it.
    fx.add("a.rs", 10, 10, "close here?").unwrap();
    fx.add("a.rs", 12, 14, "this block").unwrap();
    lines.insert(0, "use std::io;".into());
    lines.insert(0, "// header".into());
    lines.push("trailing();".into());
    fx.files.set("a.rs", &text(&lines));
    assert_eq!(fx.places(), [(12, 12, false), (14, 16, false)]);
    // Lines removed above.
    lines.drain(0..5);
    fx.files.set("a.rs", &text(&lines));
    assert_eq!(fx.places(), [(7, 7, false), (9, 11, false)]);
    let list = fx.list();
    assert_eq!(
        list[1].code,
        strings(&[
            "let line_12 = 12;",
            "let line_13 = 13;",
            "let line_14 = 14;"
        ])
    );
    assert_eq!(list[0].code, strings(&["}"]));
    // A line added inside the block: it is found nowhere whole, so it is outdated.
    lines.insert(10, "inserted();".into());
    fx.files.set("a.rs", &text(&lines));
    assert_eq!(fx.places(), [(7, 7, false), (9, 11, true)]);
}

#[test]
fn moved_code_is_found_nearest_to_where_it_was() {
    let fx = Fixture::new();
    let mut lines = numbered(30);
    fx.files.set("a.rs", &text(&lines));
    fx.add("a.rs", 5, 6, "moved?").unwrap();
    // Cut and pasted far below: the diff loses it, its text finds it.
    let block: Vec<String> = lines.drain(4..6).collect();
    lines.splice(20..20, block.clone());
    fx.files.set("a.rs", &text(&lines));
    assert_eq!(fx.places(), [(21, 22, false)]);
    let list = fx.list();
    assert_eq!(list[0].before[2], "let line_22 = 22;");
    // Copies of it: the one with its context wins, then the nearest.
    lines.splice(0..0, block.clone());
    lines.splice(10..10, block);
    fx.files.set("a.rs", &text(&lines));
    assert_eq!(fx.places(), [(25, 26, false)]);
}

#[test]
fn a_comment_whose_code_is_gone_is_outdated_and_comes_back_with_it() {
    let fx = Fixture::new();
    let lines = numbered(8);
    fx.files.set("a.rs", &text(&lines));
    fx.add("a.rs", 3, 3, "keep?").unwrap();
    let mut changed = lines.clone();
    changed[2] = "let line_3 = 33;".into();
    fx.files.set("a.rs", &text(&changed));
    let list = fx.list();
    // Listed still, at its last place, with its last text.
    assert_eq!((list[0].from, list[0].to, list[0].outdated), (3, 3, true));
    assert_eq!(list[0].code, strings(&["let line_3 = 3;"]));
    // Still outdated while the file changes elsewhere.
    changed.insert(0, "// new".into());
    fx.files.set("a.rs", &text(&changed));
    assert_eq!(fx.places(), [(3, 3, true)]);
    // The text back (a revert), one line lower: placed again.
    changed[3] = "let line_3 = 3;".into();
    fx.files.set("a.rs", &text(&changed));
    assert_eq!(fx.places(), [(4, 4, false)]);
    // The file deleted: outdated; back as it was: placed again.
    fx.files.remove("a.rs");
    assert_eq!(fx.places(), [(4, 4, true)]);
    fx.files.set("a.rs", &text(&changed));
    assert_eq!(fx.places(), [(4, 4, false)]);
    // A trivial line whose place is gone cannot be found by its text: outdated.
    let mut braces = changed.clone();
    braces[5] = "}".into();
    fx.files.set("a.rs", &text(&braces));
    fx.add("a.rs", 6, 6, "brace").unwrap();
    braces.remove(5);
    fx.files.set("a.rs", &text(&braces));
    assert_eq!(fx.places(), [(4, 4, false), (6, 6, true)]);
}

#[test]
fn a_file_that_cannot_be_read_keeps_its_comments_where_they_were() {
    let fx = Fixture::new();
    fx.files.set("a.rs", &text(&numbered(5)));
    fx.add("a.rs", 2, 2, "x").unwrap();
    let saved = std::fs::read(fx.store()).unwrap();
    fx.files.unreadable("a.rs");
    assert_eq!(fx.places(), [(2, 2, false)]);
    // Grown past the lines followed: the same.
    fx.files.set("a.rs", &text(&numbered(FILE_LINES + 1)));
    assert_eq!(fx.places(), [(2, 2, false)]);
    assert_eq!(std::fs::read(fx.store()).unwrap(), saved);
}

#[test]
fn a_long_file_is_followed_by_text_alone() {
    let fx = Fixture::new();
    let mut lines = numbered(SNAPSHOT_LINES + 1);
    lines[99] = "}".into();
    fx.files.set("big.rs", &text(&lines));
    fx.add("big.rs", 50, 51, "a").unwrap();
    fx.add("big.rs", 100, 100, "b").unwrap();
    // No hashes kept for it.
    assert!(
        !std::fs::read_to_string(fx.store())
            .unwrap()
            .contains("big.rs\":\"")
    );
    // Unchanged: both stay.
    assert_eq!(fx.places(), [(50, 51, false), (100, 100, false)]);
    lines.insert(0, "// top".into());
    fx.files.set("big.rs", &text(&lines));
    assert_eq!(fx.places(), [(51, 52, false), (100, 100, true)]);
}

#[test]
fn resolved_comments_stay_listed_and_deleted_ones_go() {
    let fx = Fixture::new();
    fx.files.set("a.rs", &text(&numbered(5)));
    fx.files.set("b.rs", &text(&numbered(5)));
    fx.add("a.rs", 1, 1, "one").unwrap();
    fx.add("b.rs", 2, 2, "two").unwrap();
    let resolved = fx
        .handle(Request::Resolve {
            id: 1,
            resolved: true,
        })
        .unwrap();
    assert_eq!(
        resolved.iter().map(|c| c.resolved).collect::<Vec<_>>(),
        [true, false]
    );
    let open = fx
        .handle(Request::Resolve {
            id: 1,
            resolved: false,
        })
        .unwrap();
    assert!(!open[0].resolved);
    // An unknown id changes nothing.
    assert_eq!(
        fx.handle(Request::Resolve {
            id: 9,
            resolved: true
        })
        .unwrap(),
        open
    );
    assert_eq!(fx.handle(Request::Delete { id: 9 }).unwrap(), open);
    let left = fx.handle(Request::Delete { id: 2 }).unwrap();
    assert_eq!(left.iter().map(|c| c.id).collect::<Vec<_>>(), [1]);
    // The deleted comment's file is no longer followed.
    assert!(
        !std::fs::read_to_string(fx.store())
            .unwrap()
            .contains("b.rs")
    );
    assert!(fx.handle(Request::Delete { id: 1 }).unwrap().is_empty());
    assert!(!fx.comments.has(&fx.worktree));
    // Only the last id is left: a late request about a deleted comment never hits a new one.
    let empty = serde_json::json!({ &fx.worktree: { "next": 2, "comments": [], "files": {}, "digests": {} } });
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(fx.store()).unwrap())
            .unwrap(),
        empty
    );
    assert_eq!(fx.add("a.rs", 1, 1, "again").unwrap()[0].id, 3);
    fx.comments
        .forget(std::slice::from_ref(&fx.worktree))
        .unwrap();
    assert!(fx.list().is_empty());
}

#[test]
fn comments_have_limits() {
    let fx = Fixture::new();
    fx.files.set("a.rs", &text(&numbered(LINES_MOST + 1)));
    let refused = |from, to, body: &str| fx.add("a.rs", from, to, body).unwrap_err().to_string();
    assert_eq!(refused(1, 1, " \n"), "a comment needs some text");
    let long = "é".repeat(BODY_MOST + 1);
    assert_eq!(refused(1, 1, &long), "a comment is at most 4000 characters");
    assert_eq!(refused(0, 1, "x"), "a comment is on 1 to 1000 lines");
    assert_eq!(refused(2, 1, "x"), "a comment is on 1 to 1000 lines");
    assert_eq!(refused(1, 1001, "x"), "a comment is on 1 to 1000 lines");
    assert_eq!(
        refused(1003, 1003, "x"),
        "a.rs has no line 1003 (it has 1001)"
    );
    assert_eq!(
        fx.add("gone.rs", 1, 1, "x").unwrap_err().to_string(),
        "gone.rs does not exist"
    );
    fx.files.unreadable("bad");
    assert_eq!(
        fx.add("bad", 1, 1, "x").unwrap_err().to_string(),
        "not a regular file"
    );
    fx.files.set("huge.rs", &text(&numbered(FILE_LINES + 1)));
    assert_eq!(
        fx.add("huge.rs", 1, 1, "x").unwrap_err().to_string(),
        "huge.rs has more than 50000 lines"
    );
    // At the edges: accepted.
    let body = "é".repeat(BODY_MOST);
    let added = fx.add("a.rs", 2, LINES_MOST as u32 + 1, &body).unwrap();
    assert_eq!(added[0].code.len(), QUOTE_LINES);
    assert_eq!(fx.list().len(), 1);
    // The count.
    for _ in 1..3 {
        fx.add("a.rs", 1, 1, "x").unwrap();
    }
    assert_eq!(
        refused(1, 1, "x"),
        "a worktree has at most 3 comments: delete some first"
    );
}

#[test]
fn shown_lines_are_cut_and_crlf_files_match_lf_ones() {
    let fx = Fixture::new();
    // Control characters are left out of the shown text, a tab is kept.
    let ctl = Fixture::new();
    ctl.files.set("ctl.txt", "a\u{1b}[31m\tb\u{7}\n");
    assert_eq!(ctl.add("ctl.txt", 1, 1, "c").unwrap()[0].code, ["a[31m\tb"]);
    let wide = "x".repeat(QUOTE_CHARS + 50);
    fx.files
        .set("a.txt", &format!("{wide}\r\nsecond line here\r\n"));
    let added = fx.add("a.txt", 1, 1, "wide").unwrap();
    assert_eq!(added[0].code, ["x".repeat(QUOTE_CHARS)]);
    assert_eq!(added[0].after, ["second line here"]);
    // The same text with LF endings, a line above: the same code.
    fx.files
        .set("a.txt", &format!("top\n{wide}\nsecond line here\n"));
    assert_eq!(fx.places(), [(2, 2, false)]);
    // An empty file has no line to comment on.
    fx.files.set("empty", "");
    assert_eq!(
        fx.add("empty", 1, 1, "x").unwrap_err().to_string(),
        "empty has no line 1 (it has 0)"
    );
}

#[test]
fn a_full_store_refuses_a_change_but_still_lists() {
    let fx = Fixture::with_limits(1500, 3);
    fx.files.set("a.rs", &text(&numbered(10)));
    fx.add("a.rs", 2, 2, "fits").unwrap();
    let err = fx.add("a.rs", 3, 3, &"y".repeat(1000)).unwrap_err();
    assert_eq!(err.to_string(), "comments.json would be over 1500 bytes");
    // Moves that cannot be saved are still answered.
    let mut lines = numbered(10);
    lines.splice(0..0, (0..40).map(|i| format!("let more_{i} = {i};")));
    fx.files.set("a.rs", &text(&lines));
    assert_eq!(fx.places(), [(42, 42, false)]);
}

#[test]
fn a_comment_found_nowhere_in_its_kept_hashes_is_outdated() {
    let fx = Fixture::new();
    fx.files.set("a.rs", &text(&numbered(4)));
    fx.add("a.rs", 2, 2, "x").unwrap();
    // A store whose anchor cannot be read (edited by hand).
    let saved = std::fs::read_to_string(fx.store()).unwrap();
    let packed = anchor::pack(&[anchor::line_hash(b"let line_2 = 2;")]);
    std::fs::write(
        fx.store(),
        saved.replace(&format!("\"hashes\":\"{packed}\""), "\"hashes\":\"!\""),
    )
    .unwrap();
    fx.files.set("a.rs", &text(&numbered(5)));
    assert_eq!(fx.places(), [(2, 2, true)]);
}

#[test]
fn the_author_is_the_user_without_control_characters() {
    assert_eq!(author_from(Some("ana".into())), "ana");
    assert_eq!(author_from(Some(" b\u{1b}[31mob\n ".into())), "b[31mob");
    assert_eq!(author_from(Some("\n".into())), "you");
    assert_eq!(author_from(None), "you");
    assert_eq!(author_from(Some("z".repeat(100))).len(), AUTHOR_CHARS);
    assert!(!author().is_empty());
}

#[test]
fn files_are_read_inside_the_worktree_only() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "a\n").unwrap();
    assert_eq!(
        read_file(tmp.path(), "a.rs").unwrap(),
        Some(b"a\n".to_vec())
    );
    assert_eq!(read_file(tmp.path(), "none").unwrap(), None);
    assert!(read_file(tmp.path(), "../x").is_err());
    std::fs::write(tmp.path().join("big"), vec![b'x'; READ_LIMIT as usize + 1]).unwrap();
    assert!(read_file(tmp.path(), "big").is_err());
    // A link out of the worktree, and a FIFO, are refused (without blocking).
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), "s\n").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path().join("secret"), tmp.path().join("link")).unwrap();
        assert!(read_file(tmp.path(), "link").is_err());
        let fifo = tmp.path().join("fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        assert!(read_file(tmp.path(), "fifo").is_err());
    }
}

#[test]
fn an_unchanged_file_is_not_searched_again() {
    let fx = Fixture::new();
    fx.files.set("a.rs", &text(&numbered(5)));
    fx.add("a.rs", 2, 2, "x").unwrap();
    // Marked outdated by hand: the file did not change, so nothing looks for it again.
    let saved = std::fs::read_to_string(fx.store()).unwrap();
    let edited = saved.replace("\"outdated\":false", "\"outdated\":true");
    std::fs::write(fx.store(), edited).unwrap();
    assert_eq!(fx.places(), [(2, 2, true)]);
    // Once it changes, it is.
    fx.files
        .set("a.rs", &format!("top\n{}", text(&numbered(5))));
    assert_eq!(fx.places(), [(3, 3, false)]);
}

#[test]
fn one_request_reads_so_many_bytes_of_files() {
    let mut fx = Fixture::new();
    let lines = numbered(5);
    fx.files.set("a.rs", &text(&lines));
    fx.files.set("b.rs", &text(&lines));
    fx.add("a.rs", 2, 2, "a").unwrap();
    fx.add("b.rs", 2, 2, "b").unwrap();
    let moved = format!("top\n{}", text(&lines));
    fx.files.set("a.rs", &moved);
    fx.files.set("b.rs", &moved);
    // `a.rs` is read first, and uses the whole budget: `b.rs` waits for the next request.
    fx.comments.budget = moved.len() as u64;
    assert_eq!(fx.places(), [(3, 3, false), (2, 2, false)]);
    fx.comments.budget = READ_BUDGET;
    assert_eq!(fx.places(), [(3, 3, false), (3, 3, false)]);
}

#[test]
fn comments_too_long_to_list_are_refused() {
    let mut fx = Fixture::new();
    fx.files.set("a.rs", &text(&numbered(5)));
    let listed = fx.add("a.rs", 2, 2, "x").unwrap();
    let size = serde_json::to_vec(&listed).unwrap().len();
    fx.comments.list_limit = size;
    assert_eq!(fx.list(), listed);
    fx.comments.list_limit = size - 1;
    let err = fx.handle(Request::List).unwrap_err();
    assert_eq!(
        err.to_string(),
        "the comments are too long to list: delete some"
    );
    assert_eq!((READ_BUDGET, LIST_LIMIT), (64 << 20, 3 << 20));
}
