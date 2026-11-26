//! Review comments that stay on their code (27.3). A worktree's comments are kept by the service
//! with the lines they are on, the text of those lines and a few lines around them, their author,
//! time and state (open or resolved). When the file changes, each one moves with its code
//! ([`crate::anchor`]): the file's lines are diffed against the line hashes kept when the
//! comments were last placed, and a comment whose lines did not all stay together is looked for
//! by its text and context, nearest to where it was. One whose code is gone is outdated, and
//! stays listed at its last place; it comes back if its text does. Comments live in
//! `<data>/comments.json` ([`WorktreeStore`]), never inside `.git` (#56), and go with the
//! worktree.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use hive_protocol::{Control, ReviewComment};
use serde::{Deserialize, Serialize};

use crate::anchor::{self, Anchor, Mapped};
use crate::projects::Projects;
use crate::worktree_store::WorktreeStore;

/// Largest comments file.
pub const FILE_LIMIT: u64 = 4 << 20; // 4 MiB
/// Most comments in one worktree.
pub const MOST: usize = 500;
/// Most characters of a comment's text.
pub const BODY_MOST: usize = 4000;
/// Most lines one comment is on.
pub const LINES_MOST: usize = 1000;
/// Largest file a comment is placed on and followed in.
pub const READ_LIMIT: u64 = 2 << 20; // 2 MiB
/// Most lines of a file a comment is placed on and followed in.
pub const FILE_LINES: usize = 50_000;
/// Most lines of a file whose hashes are kept to diff it at its next change; a longer file's
/// comments are only looked for by their text.
const SNAPSHOT_LINES: usize = 10_000;
/// Lines of context kept on each side of a comment's lines.
const CONTEXT: usize = 3;
/// Most lines of a comment's own text kept to show.
const QUOTE_LINES: usize = 20;
/// Most characters kept of each line shown.
const QUOTE_CHARS: usize = 200;
/// Most characters of an author's name.
const AUTHOR_CHARS: usize = 64;

/// A worktree's comments.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Kept {
    /// The last id given.
    next: u64,
    comments: Vec<Stored>,
    /// By file: its line hashes ([`anchor::pack`]) when its comments were last placed, kept
    /// while it has a comment that is not outdated and at most [`SNAPSHOT_LINES`] lines.
    files: BTreeMap<String, String>,
}

/// A comment as kept: what the app gets, and its anchor's line hashes (packed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Stored {
    #[serde(flatten)]
    comment: ReviewComment,
    /// Its lines' hashes, and those of the context above and below.
    hashes: String,
    above: String,
    below: String,
}

/// A file as read now: its lines (without `\n`) and their hashes.
#[derive(Default)]
struct Lines<'a> {
    lines: Vec<&'a [u8]>,
    hashes: Vec<u64>,
}

impl<'a> Lines<'a> {
    /// `text`'s lines (a final `\n` ends the last line), or `None` past [`FILE_LINES`].
    fn of(text: &'a [u8]) -> Option<Self> {
        let mut lines = Vec::new();
        if !text.is_empty() {
            let body = text.strip_suffix(b"\n").unwrap_or(text);
            for line in body.split(|&b| b == b'\n') {
                if lines.len() == FILE_LINES {
                    return None;
                }
                lines.push(line);
            }
        }
        let hashes = lines.iter().map(|line| anchor::line_hash(line)).collect();
        Some(Self { lines, hashes })
    }

    /// The text of `range`'s lines, the first [`QUOTE_LINES`], each cut at [`QUOTE_CHARS`].
    fn quote(&self, range: Range<usize>) -> Vec<String> {
        let lines = self.lines[range].iter().take(QUOTE_LINES);
        lines.map(|line| shown(line)).collect()
    }
}

/// A line to show: lossy UTF-8, without a trailing `\r`, cut at [`QUOTE_CHARS`] characters.
fn shown(line: &[u8]) -> String {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    String::from_utf8_lossy(line)
        .chars()
        .take(QUOTE_CHARS)
        .collect()
}

/// Reads a worktree's file: its bytes, `None` when it does not exist.
pub type Read<'a> = &'a dyn Fn(&str) -> io::Result<Option<Vec<u8>>>;

/// What the app asks of a worktree's comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    List,
    Add {
        file: String,
        lines: (u32, u32),
        body: String,
        author: String,
        ms: u64,
    },
    Resolve {
        id: u64,
        resolved: bool,
    },
    Delete {
        id: u64,
    },
}

/// The review comments of every worktree.
pub struct Comments {
    store: WorktreeStore<Kept>,
    /// Held from reading a worktree's comments to saving them.
    lock: Mutex<()>,
    /// Most comments in one worktree: [`MOST`].
    most: usize,
}

impl Comments {
    pub fn new(file: PathBuf) -> Self {
        Self::with_limits(file, FILE_LIMIT, MOST)
    }

    /// Comments in a `file` of at most `limit` bytes, at most `most` in a worktree.
    pub fn with_limits(file: PathBuf, limit: u64, most: usize) -> Self {
        Self {
            store: WorktreeStore::new(file, limit),
            lock: Mutex::new(()),
            most,
        }
    }

    /// Drops the comments of `worktrees` (removed ones).
    pub fn forget(&self, worktrees: &[String]) -> io::Result<()> {
        let _lock = self.lock();
        self.store.forget(worktrees)
    }

    /// Whether `worktree` has comments.
    pub fn has(&self, worktree: &str) -> bool {
        let _lock = self.lock();
        !self.store.get(worktree).comments.is_empty()
    }

    /// Does `request` on `worktree`'s comments, each first moved to where its code is in the
    /// files `read` gives, and saves them; their list. A list is answered even when the moves
    /// cannot be saved (the next one finds them again).
    pub fn handle(
        &self,
        worktree: &str,
        read: Read<'_>,
        request: Request,
    ) -> io::Result<Vec<ReviewComment>> {
        let _lock = self.lock();
        let before = self.store.get(worktree);
        let mut kept = before.clone();
        let files: BTreeSet<String> = kept
            .comments
            .iter()
            .map(|c| c.comment.file.clone())
            .collect();
        for file in &files {
            // A file that cannot be read now (too large, not a regular file) keeps its comments
            // where they were: whether its code is gone cannot be told.
            match read(file) {
                Ok(Some(bytes)) => {
                    if let Some(now) = Lines::of(&bytes) {
                        follow(&mut kept, file, &now);
                    }
                }
                Ok(None) => follow(&mut kept, file, &Lines::default()),
                Err(_) => {}
            }
        }
        let list = matches!(request, Request::List);
        apply(&mut kept, read, request, self.most)?;
        if kept.comments.is_empty() {
            kept = Kept::default();
        }
        if kept != before {
            let saved = self.store.update(worktree, |data| *data = kept.clone());
            if !list {
                saved?;
            } else if let Err(err) = saved {
                eprintln!("hive: warning: cannot save where the review comments moved: {err}");
            }
        }
        Ok(kept.comments.into_iter().map(|c| c.comment).collect())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Does `request` on `kept`, whose comments are already where their code is.
fn apply(kept: &mut Kept, read: Read<'_>, request: Request, most: usize) -> io::Result<()> {
    match request {
        Request::List => {}
        Request::Add {
            file,
            lines,
            body,
            author,
            ms,
        } => add(kept, read, (file, lines), body, (author, ms, most))?,
        Request::Resolve { id, resolved } => {
            let found = kept.comments.iter_mut().find(|c| c.comment.id == id);
            if let Some(stored) = found {
                stored.comment.resolved = resolved;
            }
        }
        Request::Delete { id } => {
            kept.comments.retain(|c| c.comment.id != id);
            let comments = &kept.comments;
            kept.files
                .retain(|file, _| comments.iter().any(|c| &c.comment.file == file));
        }
    }
    Ok(())
}

/// Adds a comment `body` on the lines `from` to `to` (1-based) of `file` as it is now.
fn add(
    kept: &mut Kept,
    read: Read<'_>,
    (file, (from, to)): (String, (u32, u32)),
    body: String,
    (author, ms, most): (String, u64, usize),
) -> io::Result<()> {
    let body = body.trim();
    if body.is_empty() {
        return Err(io::Error::other("a comment needs some text"));
    }
    if body.chars().count() > BODY_MOST {
        return Err(io::Error::other(format!(
            "a comment is at most {BODY_MOST} characters"
        )));
    }
    if kept.comments.len() >= most {
        return Err(io::Error::other(format!(
            "a worktree has at most {most} comments: delete some first"
        )));
    }
    let (start, end) = (from as usize, to as usize);
    if start == 0 || end < start || end - start >= LINES_MOST {
        return Err(io::Error::other(format!(
            "a comment is on 1 to {LINES_MOST} lines"
        )));
    }
    let bytes = read(&file)?.ok_or_else(|| io::Error::other(format!("{file} does not exist")))?;
    let now = Lines::of(&bytes)
        .ok_or_else(|| io::Error::other(format!("{file} has more than {FILE_LINES} lines")))?;
    if end > now.hashes.len() {
        return Err(io::Error::other(format!(
            "{file} has no line {end} (it has {})",
            now.hashes.len()
        )));
    }
    // Read again: the file may have changed since its comments were placed.
    follow(kept, &file, &now);
    kept.next += 1;
    let comment = ReviewComment {
        id: kept.next,
        file: file.clone(),
        from,
        to,
        body: body.to_owned(),
        code: Vec::new(),
        before: Vec::new(),
        after: Vec::new(),
        author,
        ms,
        resolved: false,
        outdated: false,
    };
    let mut stored = Stored {
        comment,
        hashes: String::new(),
        above: String::new(),
        below: String::new(),
    };
    place(&mut stored, &now, start - 1..end);
    kept.comments.push(stored);
    snapshot(kept, &file, &now);
    Ok(())
}

/// Moves the comments on `file` to where their code is in `now`, or marks them outdated.
fn follow(kept: &mut Kept, file: &str, now: &Lines) {
    let old = kept
        .files
        .get(file)
        .and_then(|packed| anchor::unpack(packed));
    if old.as_deref() == Some(&now.hashes[..]) {
        return;
    }
    let map = old.map(|old| anchor::diff(&old, &now.hashes));
    for stored in kept.comments.iter_mut().filter(|c| c.comment.file == file) {
        match found(stored, map.as_ref(), &now.hashes) {
            Some(range) => place(stored, now, range),
            None => stored.comment.outdated = true,
        }
    }
    snapshot(kept, file, now);
}

/// Where `stored`'s code is in the file whose line hashes are `now`, `map` taking its last place
/// there when the file's previous hashes were kept; `None` when it is gone.
fn found(stored: &Stored, map: Option<&anchor::LineMap>, now: &[u64]) -> Option<Range<usize>> {
    let unpack = |packed: &str| anchor::unpack(packed).unwrap_or_default();
    let anchor = Anchor {
        start: stored.comment.from.saturating_sub(1) as usize,
        lines: unpack(&stored.hashes),
        before: unpack(&stored.above),
        after: unpack(&stored.below),
    };
    let range = anchor.start..anchor.start + anchor.lines.len();
    let at = match map {
        // An outdated comment was not placed in the kept hashes: only its text finds it.
        Some(map) if !stored.comment.outdated => match map.range(range) {
            Mapped::Kept(range) => return Some(range),
            Mapped::Partly(range) => range.start,
            Mapped::Gone => anchor.start,
        },
        _ if now.get(range.clone()) == Some(&anchor.lines[..]) && !anchor.lines.is_empty() => {
            return Some(range);
        }
        _ => anchor.start,
    };
    anchor::relocate(&anchor, now, at)
}

/// Sets `stored` on the lines `range` of `now`: its place, text, context and their hashes.
fn place(stored: &mut Stored, now: &Lines, range: Range<usize>) {
    let anchor = Anchor::at(&now.hashes, range.clone(), CONTEXT);
    stored.hashes = anchor::pack(&anchor.lines);
    stored.above = anchor::pack(&anchor.before);
    stored.below = anchor::pack(&anchor.after);
    let comment = &mut stored.comment;
    comment.from = range.start as u32 + 1;
    comment.to = range.end as u32;
    comment.outdated = false;
    comment.code = now.quote(range.clone());
    comment.before = now.quote(range.start - anchor.before.len()..range.start);
    comment.after = now.quote(range.end..range.end + anchor.after.len());
}

/// Keeps `now`'s hashes for `file` while a comment there is placed on them and the file is
/// short enough; drops them otherwise.
fn snapshot(kept: &mut Kept, file: &str, now: &Lines) {
    let placed = kept
        .comments
        .iter()
        .any(|c| c.comment.file == file && !c.comment.outdated);
    if placed && now.hashes.len() <= SNAPSHOT_LINES {
        kept.files
            .insert(file.to_owned(), anchor::pack(&now.hashes));
    } else {
        kept.files.remove(file);
    }
}

/// Who writes comments here: the service's user (`USER`, `USERNAME` on Windows), "you"
/// without one. Stage 28 shares comments between people.
pub fn author() -> String {
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME"));
    author_from(user.ok())
}

/// `name` as an author: without control characters, at most [`AUTHOR_CHARS`] characters;
/// "you" when nothing is left.
fn author_from(name: Option<String>) -> String {
    let name: String = name
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(AUTHOR_CHARS)
        .collect();
    match name.trim() {
        "" => "you".to_owned(),
        name => name.to_owned(),
    }
}

/// The file `file` of the worktree at `dir`, checked as `open_file` checks it (relative, inside
/// the worktree through links, a regular file), at most [`READ_LIMIT`] bytes.
fn read_file(dir: &Path, file: &str) -> io::Result<Option<Vec<u8>>> {
    let opened = crate::file::open_inside(dir, file)?;
    opened
        .map(|mut f| crate::git::read_limited(&mut f, READ_LIMIT))
        .transpose()
}

/// The answer to `request` on the comments of the worktree `path` of `projects`: `Comments`, or
/// `Error` saying why not.
pub fn answer(projects: &Projects, comments: &Comments, path: String, request: Request) -> Control {
    let what = match request {
        Request::List => "list the review comments",
        Request::Add { .. } => "add the comment",
        Request::Resolve { .. } => "change the comment",
        Request::Delete { .. } => "delete the comment",
    };
    let result = projects.worktree(&path).and_then(|dir| {
        let read = |file: &str| read_file(&dir, file);
        comments.handle(&path, &read, request)
    });
    match result {
        Ok(comments) => Control::Comments { path, comments },
        Err(err) => Control::Error {
            message: format!("Cannot {what}: {err}"),
        },
    }
}

#[cfg(test)]
mod tests;
