//! Line provenance (27.5, `docs/spike/line-provenance.md`): from a line to the session that
//! wrote it.
//!
//! For each `PostToolUse` of a file edit, `hive hook` sends the edit's new lines as hashes
//! (`hive_edit`, [`crate::slim`]); the service keeps an *edit record* (session, subagent,
//! tool, time, file, the runs of new lines) and, per file an agent or Hive's editor touched, a
//! *line map*: the file's line hashes when Hive last looked and each line's origin (an edit, the
//! user in Hive's editor, unknown, or before Hive followed the file). When the file changed
//! since, the map follows it through a diff of hashes ([`crate::anchor`]). A line Hive cannot
//! answer for live goes through `git blame`: the commit, then the repository's edit records made
//! before it that hold that line, as a *probable* answer.
//!
//! What a payload claims is checked against the file itself: a claimed run whose hashes are not
//! in the file labels nothing. The service never sees the file's text from a payload, and it
//! keeps no text of its own: what it reads is hashed and dropped. Everything is advisory.
//!
//! Storage: one JSON file per repository in `<Hive folder>/provenance/` (0700, files 0600,
//! written atomically at most every 2 s and at shutdown), never inside `.git` (#56); size-capped.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Read};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use hive_protocol::{
    AgentEvent, Control, EventKind, LineCommit, LineEdit, LineOrigin, LineSide, Project,
    SessionFile,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::anchor::{self, Anchor};
use crate::paths::canonical;
use crate::{adapter, file, git, sessions};

/// Largest file followed line by line; a larger one gets file-level records only.
pub const FILE_BYTES: u64 = 2 << 20;
/// Most lines of a file followed line by line.
pub const FILE_LINES: usize = 50_000;
/// Most edit records kept per repository; the oldest go first.
pub const MAX_RECORDS: usize = 20_000;
/// Age past which an edit record goes.
pub const MAX_AGE_MS: u64 = 90 * 24 * 3600 * 1000;
/// Largest repository file: past it the line maps of the files untouched longest go, then the
/// oldest records.
pub const REPO_BYTES: usize = 8 << 20;
/// Largest total of the repository files: past it the least recently written goes.
pub const TOTAL_BYTES: u64 = 64 << 20;
/// Most runs of an edit believed.
const MAX_RUNS: usize = 4096;
/// Most hashed lines of one edit believed (what `hive hook` sends at most).
const MAX_HASHED: usize = 40_000;
/// Most runs of the file's earlier records tried when lines come back, and most runs of one
/// edit searched for away from their line.
const RECLAIM_RUNS: usize = 256;
/// Longest file path believed.
const PATH_LIMIT: usize = 4096;
/// Longest subagent id kept, in bytes (as `states` keeps them).
const AGENT_LIMIT: usize = 256;
/// Longest subagent type kept, in characters.
const AGENT_TYPE_LIMIT: usize = 64;
/// The file's own records tried when lines change back to an earlier text, newest first.
const RECLAIM_RECORDS: usize = 50;
/// How long after a commit an edit may be recorded and still be in it (clocks, hook delay).
const COMMIT_SLACK_MS: u64 = 5 * 60 * 1000;
/// Context lines around a blamed line, matched against the records.
const CONTEXT: usize = 3;
/// Most files `session_files` lists.
const FILES_LIMIT: usize = 500;
/// The longest `git blame` and `git cat-file` may take.
const GIT_TIME: Duration = Duration::from_secs(10);
/// A worktree's followed files are looked at again at most this often.
const TOUCH_EVERY_MS: u64 = 1000;
/// The store's format.
const VERSION: u32 = 1;

/// Where a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The edit record with this id.
    Edit(u64),
    /// The user, in Hive's editor, at this time (ms).
    Editor(u64),
    /// Changed outside an agent's `Edit`/`Write` (a shell command, another editor, git).
    Unknown,
    /// There before Hive followed the file.
    Before,
}

/// One recorded edit of a file by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    pub id: u64,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    /// The session's working folder, when it is not the worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub ms: u64,
    pub worktree: String,
    /// Relative to the worktree, as git names it in the repository.
    pub path: String,
    #[serde(default)]
    pub user_modified: bool,
    /// The runs of new lines: 1-based start, count, and their [packed](anchor::pack) hashes
    /// (empty: too many to keep, followed through the diff only).
    #[serde(default)]
    pub added: Vec<(usize, usize, String)>,
}

impl Edit {
    /// Each run's 0-based range and hashes (empty when not kept).
    /// A run outside the lines a file may have (a stored record is untrusted too) is left out.
    fn runs(&self) -> impl Iterator<Item = (Range<usize>, Vec<u64>)> + '_ {
        self.added.iter().filter_map(|(start, count, packed)| {
            let at = start.checked_sub(1)?;
            let end = at
                .checked_add(*count)
                .filter(|&e| e <= FILE_LINES && *count > 0)?;
            let hashes = anchor::unpack(packed).filter(|h| h.len() == *count);
            Some((at..end, hashes.unwrap_or_default()))
        })
    }
}

/// A followed file: its line hashes when Hive last looked, and each line's origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMap {
    pub size: u64,
    pub mtime_ms: u64,
    /// When it was last changed or asked about, for the size cap.
    pub used_ms: u64,
    pub hashes: Vec<u64>,
    pub origins: Vec<Origin>,
}

/// One repository's records and its worktrees' line maps.
#[derive(Debug, Default)]
struct Repo {
    project: String,
    next_id: u64,
    edits: Vec<Edit>,
    maps: BTreeMap<String, BTreeMap<String, FileMap>>,
    /// When each worktree's files were last looked at ([`TOUCH_EVERY_MS`]).
    touched: HashMap<String, u64>,
}

/// The repository file.
#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    project: String,
    next_id: u64,
    edits: Vec<Edit>,
    worktrees: BTreeMap<String, BTreeMap<String, StoredMap>>,
}

#[derive(Serialize, Deserialize)]
struct StoredMap {
    size: u64,
    mtime_ms: u64,
    used_ms: u64,
    hashes: String,
    /// Runs of `[0-based start, length, origin]`.
    origins: Vec<(usize, usize, StoredOrigin)>,
}

/// An origin in the file: an edit id, or `"before"`, `"unknown"`, `"editor@<ms>"`.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
#[serde(untagged)]
enum StoredOrigin {
    Edit(u64),
    Named(String),
}

impl From<Origin> for StoredOrigin {
    fn from(origin: Origin) -> Self {
        match origin {
            Origin::Edit(id) => StoredOrigin::Edit(id),
            Origin::Editor(ms) => StoredOrigin::Named(format!("editor@{ms}")),
            Origin::Unknown => StoredOrigin::Named("unknown".to_owned()),
            Origin::Before => StoredOrigin::Named("before".to_owned()),
        }
    }
}

impl StoredOrigin {
    fn origin(&self) -> Option<Origin> {
        match self {
            StoredOrigin::Edit(id) => Some(Origin::Edit(*id)),
            StoredOrigin::Named(name) => match name.as_str() {
                "unknown" => Some(Origin::Unknown),
                "before" => Some(Origin::Before),
                other => other
                    .strip_prefix("editor@")?
                    .parse()
                    .ok()
                    .map(Origin::Editor),
            },
        }
    }
}

impl FileMap {
    fn stored(&self) -> StoredMap {
        let mut runs: Vec<(usize, usize, StoredOrigin)> = Vec::new();
        let mut start = 0;
        for (i, origin) in self.origins.iter().enumerate() {
            if self.origins.get(i + 1) != Some(origin) {
                runs.push((start, i + 1 - start, (*origin).into()));
                start = i + 1;
            }
        }
        StoredMap {
            size: self.size,
            mtime_ms: self.mtime_ms,
            used_ms: self.used_ms,
            hashes: anchor::pack(&self.hashes),
            origins: runs,
        }
    }

    /// The map a stored one holds; `None` when its runs do not cover its lines exactly.
    fn from_stored(stored: &StoredMap) -> Option<FileMap> {
        let hashes = anchor::unpack(&stored.hashes)?;
        let mut origins = Vec::with_capacity(hashes.len());
        for (start, len, origin) in &stored.origins {
            let origin = origin.origin()?;
            let end = origins.len().checked_add(*len)?;
            if *start != origins.len() || end > hashes.len() {
                return None;
            }
            origins.extend(std::iter::repeat_n(origin, *len));
        }
        (origins.len() == hashes.len()).then_some(FileMap {
            size: stored.size,
            mtime_ms: stored.mtime_ms,
            used_ms: stored.used_ms,
            hashes,
            origins,
        })
    }

    /// The run of lines around `line` (0-based) with its origin.
    fn run(&self, line: usize) -> Range<usize> {
        let origin = self.origins[line];
        let start = self.origins[..line]
            .iter()
            .rposition(|o| *o != origin)
            .map_or(0, |i| i + 1);
        let end = self.origins[line..]
            .iter()
            .position(|o| *o != origin)
            .map_or(self.origins.len(), |i| line + i);
        start..end
    }
}

/// A file's line hashes as read now, with its size and modification time.
struct Lines {
    hashes: Vec<u64>,
    size: u64,
    mtime_ms: u64,
}

/// What reading a followed file gave.
enum Look {
    Lines(Lines),
    /// Over [`FILE_BYTES`] or [`FILE_LINES`]: followed by file-level records only.
    TooLarge,
}

/// The new origins of `file`'s lines, after the change from `old` (the map, if any), optionally
/// by `edit` (whose `whole` run is the whole file, Q5), with the file's earlier records
/// `history` (newest first) to re-claim text that came back:
///
/// 1. Lines the diff maps from `old` keep their origin; the others are `fresh` (no map yet:
///    [`Origin::Before`]).
/// 2. Each of the edit's runs is claimed where its hashes are, at its line, else nearest to it
///    ([`anchor::relocate`]); one not found leaves its lines unknown. A `whole` run claims, through
///    a diff, only the lines that changed since the map. A run without hashes claims its range.
/// 3. Unknown lines that hold one of the file's earlier runs again are that edit's.
pub fn follow(
    old: Option<&FileMap>,
    file: &[u64],
    fresh: Origin,
    edit: Option<(&Edit, bool)>,
    history: &[&Edit],
) -> Vec<Origin> {
    let (mut origins, changed) = match old {
        Some(old) => {
            let map = anchor::diff(&old.hashes, file);
            let mut origins = vec![fresh; file.len()];
            let mut changed = vec![true; file.len()];
            for (line, origin) in old.origins.iter().enumerate() {
                if let Some(new) = map.line(line) {
                    origins[new] = *origin;
                    changed[new] = false;
                }
            }
            (origins, changed)
        }
        None => (vec![Origin::Before; file.len()], vec![true; file.len()]),
    };
    if let Some((edit, whole)) = edit {
        claim(&mut origins, &changed, file, edit, whole);
    }
    reclaim(&mut origins, file, history);
    origins
}

fn claim(origins: &mut [Origin], changed: &[bool], file: &[u64], edit: &Edit, whole: bool) {
    let mine = Origin::Edit(edit.id);
    let mut relocations = RECLAIM_RUNS;
    for (range, hashes) in edit.runs() {
        let clamped = range.start.min(file.len())..range.end.min(file.len());
        if hashes.is_empty() {
            for line in clamped.filter(|&l| changed[l]) {
                origins[line] = mine;
            }
        } else if whole {
            let map = anchor::diff(&hashes, file);
            let lines = (0..hashes.len()).filter_map(|i| map.line(i));
            for line in lines.filter(|&l| changed[l]) {
                origins[line] = mine;
            }
        } else if file.get(range.clone()) == Some(&hashes[..]) {
            origins[range].fill(mine);
        } else {
            let anchor = Anchor {
                start: range.start,
                lines: hashes,
                before: Vec::new(),
                after: Vec::new(),
            };
            let found = relocations.checked_sub(1).and_then(|left| {
                relocations = left;
                anchor::relocate(&anchor, file, range.start)
            });
            match found {
                Some(found) => origins[found].fill(mine),
                None => {
                    for line in clamped.filter(|&l| changed[l]) {
                        origins[line] = Origin::Unknown;
                    }
                }
            }
        }
    }
}

fn reclaim(origins: &mut [Origin], file: &[u64], history: &[&Edit]) {
    if !origins.contains(&Origin::Unknown) {
        return;
    }
    let runs = history.iter().take(RECLAIM_RECORDS).flat_map(|edit| {
        edit.runs()
            .filter(|(_, h)| !h.is_empty())
            .map(move |run| (edit.id, run))
    });
    for (id, (range, hashes)) in runs.take(RECLAIM_RUNS) {
        {
            let anchor = Anchor {
                start: range.start,
                lines: hashes,
                before: Vec::new(),
                after: Vec::new(),
            };
            let Some(found) = anchor::relocate(&anchor, file, range.start) else {
                continue;
            };
            for origin in &mut origins[found] {
                if *origin == Origin::Unknown {
                    *origin = Origin::Edit(id);
                }
            }
        }
    }
}

/// `path` from a payload or the app: relative, normal parts only, never in `.git` (#56).
fn relative(path: &str) -> Option<&str> {
    let rel = Path::new(path);
    let normal = rel.components().all(|c| match c {
        Component::Normal(name) => !name.as_encoded_bytes().eq_ignore_ascii_case(b".git"),
        _ => false,
    });
    (!path.is_empty() && path.len() <= PATH_LIMIT && normal).then_some(path)
}

/// Reads and hashes `rel` of the worktree at `dir` (a regular file inside it, as the viewer
/// reads files); its text is dropped at once.
fn look(dir: &Path, rel: &str) -> io::Result<Look> {
    let mut opened = file::open_inside(dir, rel)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the file is gone"))?;
    let meta = opened.metadata()?;
    let mut bytes = Vec::new();
    opened
        .by_ref()
        .take(FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_BYTES {
        return Ok(Look::TooLarge);
    }
    let Some(hashes) = anchor::hashes(&bytes, FILE_LINES) else {
        return Ok(Look::TooLarge);
    };
    Ok(Look::Lines(Lines {
        hashes,
        size: meta.len(),
        mtime_ms: mtime_ms(&meta),
    }))
}

fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// The store: every repository's records and line maps, loaded the first time each is needed.
pub struct Provenance {
    /// `<Hive folder>/provenance`; `None` keeps everything in memory (tests of the service).
    dir: Option<PathBuf>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    repos: HashMap<String, Repo>,
    dirty: HashSet<String>,
}

impl Default for Provenance {
    fn default() -> Self {
        Self::new(None)
    }
}

/// An edit as the hook sent it, checked.
struct Claimed {
    file: String,
    whole: bool,
    file_level: bool,
    user_modified: bool,
    added: Vec<(usize, usize, String)>,
}

/// `hive_edit`, checked: an absolute `file`, a known `kind`, at most [`MAX_RUNS`] runs, each
/// starting at line 1 or later, within [`FILE_LINES`], with hashes that unpack to its count
/// (else kept as a range only).
fn claimed(edit: &Value) -> Option<Claimed> {
    let file = edit.get("file")?.as_str()?;
    if file.len() > PATH_LIMIT || !Path::new(file).is_absolute() {
        return None;
    }
    let kind = edit.get("kind")?.as_str()?;
    if !matches!(kind, "edit" | "create" | "update" | "notebook") {
        return None;
    }
    let runs = edit.get("added")?.as_array()?;
    if runs.len() > MAX_RUNS {
        return None;
    }
    let mut added = Vec::new();
    let (mut next, mut hashed) = (1usize, 0usize);
    for run in runs {
        let start = usize::try_from(run.get(0)?.as_u64()?).ok()?;
        let count = usize::try_from(run.get(1)?.as_u64()?).ok()?;
        let packed = run.get(2)?.as_str()?;
        let end = start.checked_add(count)?;
        // In order, never overlapping, within the lines a followed file may have.
        if start < next || count == 0 || end > FILE_LINES + 1 {
            return None;
        }
        next = end;
        let kept = anchor::unpack(packed).is_some_and(|h| h.len() == count);
        hashed += if kept { count } else { 0 };
        if hashed > MAX_HASHED {
            return None;
        }
        added.push((
            start,
            count,
            if kept {
                packed.to_owned()
            } else {
                String::new()
            },
        ));
    }
    let whole = edit.get("whole").and_then(Value::as_bool) == Some(true);
    // A whole file is one run.
    if whole && added.len() > 1 {
        return None;
    }
    Some(Claimed {
        file: file.to_owned(),
        whole,
        file_level: kind == "notebook",
        user_modified: edit.get("user_modified").and_then(Value::as_bool) == Some(true),
        added,
    })
}

/// The repository file's name for `project`.
fn key(project: &str) -> String {
    format!(
        "{:016x}",
        anchor::digest(&[anchor::line_hash(project.as_bytes())])
    )
}

/// The followed project of `worktree` (not a group's folder).
fn project_of<'a>(projects: &'a [Project], worktree: &str) -> Option<&'a Project> {
    projects
        .iter()
        .filter(|p| !p.group)
        .find(|p| p.worktrees.iter().any(|w| w.path == worktree))
}

impl Provenance {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records an agent's file edit from its `PostToolUse` (`hive_edit`), and looks again at
    /// the files of the worktree a tool or a turn of the agent ended in.
    pub fn saw(&self, projects: &[Project], event: &AgentEvent, now_ms: u64) {
        if !matches!(
            event.kind,
            EventKind::ToolFinished { .. } | EventKind::TurnFinished
        ) {
            return;
        }
        if let Some(edit) = event.raw.get("hive_edit")
            && let Err(err) = self.record(projects, event, edit, now_ms)
        {
            eprintln!("hive: cannot record an edit: {err}");
        }
        if let Some(cwd) = &event.cwd
            && let Some((project, worktree)) = crate::projects::place(projects, cwd)
        {
            self.touch(&project, &worktree, now_ms);
        }
    }

    /// Records one edit; an edit outside the followed worktrees, or of a shape that is not an
    /// edit, is left out quietly. Errors only for a followed file that could not be read.
    fn record(
        &self,
        projects: &[Project],
        event: &AgentEvent,
        edit: &Value,
        now_ms: u64,
    ) -> io::Result<()> {
        let tool = match &event.kind {
            EventKind::ToolFinished { tool: Some(tool) }
                if matches!(
                    tool.as_str(),
                    "Edit" | "Write" | "MultiEdit" | "NotebookEdit"
                ) =>
            {
                tool.clone()
            }
            _ => return Ok(()),
        };
        let Some(session) = event.session_id.clone().filter(|s| sessions::valid_id(s)) else {
            return Ok(());
        };
        let Some(claim) = claimed(edit) else {
            return Ok(());
        };
        let followed: Vec<Project> = projects.iter().filter(|p| !p.group).cloned().collect();
        let Some((project, worktree)) = crate::projects::place(&followed, &claim.file) else {
            return Ok(());
        };
        let real = canonical(Path::new(&claim.file))?;
        let root = canonical(Path::new(&worktree))?;
        let Some(rel) = real
            .strip_prefix(&root)
            .ok()
            .and_then(Path::to_str)
            .and_then(relative)
            .map(str::to_owned)
        else {
            return Ok(());
        };
        let shown = || adapter::clip(&rel, 200);
        let look = look(Path::new(&worktree), &rel)
            .map_err(|err| io::Error::other(format!("{}: {err}", shown())))?;
        let agent = event.subagent.as_ref();
        let tool_use_id = event.raw.get("tool_use_id").and_then(Value::as_str);
        let tool_use_id = tool_use_id.filter(|id| {
            (1..=64).contains(&id.len())
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        });
        let cwd = event
            .cwd
            .clone()
            .filter(|c| *c != worktree && c.len() <= PATH_LIMIT);
        let mut inner = self.inner();
        let repo = self.repo(&mut inner, &project, now_ms);
        let id = repo.next_id;
        repo.next_id = repo.next_id.saturating_add(1);
        let file_level = claim.file_level || matches!(look, Look::TooLarge);
        let record = Edit {
            id,
            session,
            agent: agent
                .map(|a| a.id.clone())
                .filter(|a| a.len() <= AGENT_LIMIT),
            agent_type: agent
                .and_then(|a| a.agent_type.as_deref())
                .map(|t| adapter::clip(t, AGENT_TYPE_LIMIT)),
            tool,
            tool_use_id: tool_use_id.map(str::to_owned),
            cwd,
            ms: now_ms,
            worktree: worktree.clone(),
            path: rel.clone(),
            user_modified: claim.user_modified,
            added: if file_level { Vec::new() } else { claim.added },
        };
        let files = repo.maps.entry(worktree.clone()).or_default();
        match look {
            Look::TooLarge => {
                files.remove(&rel);
            }
            Look::Lines(read) => {
                let history: Vec<&Edit> = repo
                    .edits
                    .iter()
                    .rev()
                    .filter(|e| e.path == rel && e.worktree == worktree)
                    .collect();
                let old = files.get(&rel);
                let edit = (!file_level).then_some((&record, claim.whole));
                let origins = follow(old, &read.hashes, Origin::Unknown, edit, &history);
                files.insert(
                    rel.clone(),
                    FileMap {
                        size: read.size,
                        mtime_ms: read.mtime_ms,
                        used_ms: now_ms,
                        hashes: read.hashes,
                        origins,
                    },
                );
            }
        }
        repo.edits.push(record);
        prune(repo, now_ms);
        inner.dirty.insert(project);
        Ok(())
    }

    /// The user saved `path` of `worktree` in Hive's editor: its changed lines are the user's.
    pub fn saved(&self, projects: &[Project], worktree: &str, path: &str, now_ms: u64) {
        if let (Some(project), Some(rel)) = (project_of(projects, worktree), relative(path)) {
            let fresh = Origin::Editor(now_ms);
            self.resync(&project.id, worktree, rel, fresh, now_ms, true);
        }
    }

    /// Looks again at `path` of `worktree` when it changed since: the changed lines get
    /// `fresh`. With `start`, a file not followed yet starts being followed (its lines
    /// `before`).
    fn resync(
        &self,
        project: &str,
        worktree: &str,
        rel: &str,
        fresh: Origin,
        now_ms: u64,
        start: bool,
    ) {
        let mut inner = self.inner();
        let repo = self.repo(&mut inner, project, now_ms);
        if !start && !repo.maps.get(worktree).is_some_and(|f| f.contains_key(rel)) {
            return;
        }
        let looked = look(Path::new(worktree), rel);
        let history: Vec<Edit> = repo
            .edits
            .iter()
            .rev()
            .filter(|e| e.path == rel && e.worktree == worktree)
            .take(RECLAIM_RECORDS)
            .cloned()
            .collect();
        let history: Vec<&Edit> = history.iter().collect();
        let files = repo.maps.entry(worktree.to_owned()).or_default();
        let read = match looked {
            Ok(Look::Lines(read)) => read,
            _ => {
                if files.remove(rel).is_some() {
                    inner.dirty.insert(project.to_owned());
                }
                return;
            }
        };
        let old = files.get(rel);
        if old.is_some_and(|m| m.hashes == read.hashes) {
            return;
        }
        let origins = follow(old, &read.hashes, fresh, None, &history);
        files.insert(
            rel.to_owned(),
            FileMap {
                size: read.size,
                mtime_ms: read.mtime_ms,
                used_ms: now_ms,
                hashes: read.hashes,
                origins,
            },
        );
        inner.dirty.insert(project.to_owned());
    }

    /// Looks again, at most once a second, at the followed files of `worktree` whose size or
    /// modification time changed (a `stat` each).
    fn touch(&self, project: &str, worktree: &str, now_ms: u64) {
        let changed: Vec<String> = {
            let mut inner = self.inner();
            let repo = self.repo(&mut inner, project, now_ms);
            let last = repo.touched.entry(worktree.to_owned()).or_insert(0);
            if now_ms < last.saturating_add(TOUCH_EVERY_MS) {
                return;
            }
            *last = now_ms;
            let Some(files) = repo.maps.get(worktree) else {
                return;
            };
            let stat = |rel: &str| std::fs::metadata(Path::new(worktree).join(rel));
            files
                .iter()
                .filter(|(rel, map)| {
                    stat(rel).is_ok_and(|m| m.len() != map.size || mtime_ms(&m) != map.mtime_ms)
                })
                .map(|(rel, _)| rel.clone())
                .collect()
        };
        for rel in changed {
            self.resync(project, worktree, &rel, Origin::Unknown, now_ms, false);
        }
    }

    /// The repository of `project`, loaded from its file the first time.
    fn repo<'a>(&self, inner: &'a mut Inner, project: &str, now_ms: u64) -> &'a mut Repo {
        inner.repos.entry(project.to_owned()).or_insert_with(|| {
            let mut repo = self.load(project);
            prune(&mut repo, now_ms);
            repo
        })
    }

    /// The repository file of `project`; a malformed one is set aside (`.json.bad`) and Hive
    /// starts empty. Maps of worktrees that are gone are dropped.
    fn load(&self, project: &str) -> Repo {
        let empty = Repo {
            project: project.to_owned(),
            ..Repo::default()
        };
        let Some(dir) = &self.dir else { return empty };
        let path = dir.join(format!("{}.json", key(project)));
        let bytes = match std::fs::File::open(&path)
            .and_then(|mut f| git::read_limited(&mut f, 2 * REPO_BYTES as u64))
        {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return empty,
            Err(_) => Vec::new(),
        };
        let stored = serde_json::from_slice::<Stored>(&bytes)
            .ok()
            .filter(|s| s.version == VERSION && s.project == project);
        let Some(stored) = stored else {
            let _ = std::fs::rename(&path, path.with_extension("json.bad"));
            eprintln!("hive: warning: set aside an unreadable provenance file");
            return empty;
        };
        let maps = stored
            .worktrees
            .into_iter()
            .filter(|(worktree, _)| Path::new(worktree).is_dir())
            .map(|(worktree, files)| {
                let files = files
                    .iter()
                    .filter(|(rel, _)| relative(rel).is_some())
                    .filter_map(|(rel, m)| Some((rel.clone(), FileMap::from_stored(m)?)))
                    .collect();
                (worktree, files)
            })
            .collect();
        let mut edits = stored.edits;
        edits.retain(|e| relative(&e.path).is_some() && sessions::valid_id(&e.session));
        // Never an id already given, whatever the file says.
        let next_id = edits
            .iter()
            .map(|e| e.id.saturating_add(1))
            .max()
            .unwrap_or(0);
        Repo {
            project: project.to_owned(),
            next_id: next_id.max(stored.next_id),
            edits,
            maps,
            touched: HashMap::new(),
        }
    }

    /// Writes the repositories changed since the last call (atomically, 0600), then keeps the
    /// files within [`TOTAL_BYTES`].
    pub fn flush(&self) {
        self.flush_within(TOTAL_BYTES);
    }

    fn flush_within(&self, total: u64) {
        let Some(dir) = &self.dir else { return };
        let mut inner = self.inner();
        if inner.dirty.is_empty() {
            return;
        }
        if let Err(err) =
            crate::mode::folder(std::fs::DirBuilder::new().recursive(true), 0o700).create(dir)
        {
            eprintln!("hive: warning: cannot keep line provenance: {err}");
            return;
        }
        let Inner { repos, dirty } = &mut *inner;
        let mut written = HashSet::new();
        for (project, repo) in repos.iter_mut().filter(|(p, _)| dirty.contains(*p)) {
            let name = format!("{}.json", key(project));
            let path = dir.join(&name);
            let written_ok = if repo.edits.is_empty() && repo.maps.values().all(BTreeMap::is_empty)
            {
                // Already gone is as good.
                let _ = std::fs::remove_file(&path);
                Ok(())
            } else {
                crate::wrapper::write_atomic(&path, &fit(repo), 0o600)
            };
            match written_ok {
                Ok(()) => {
                    written.insert(name);
                }
                Err(err) => eprintln!("hive: warning: cannot keep line provenance: {err}"),
            }
        }
        dirty.clear();
        for gone in evict_files(dir, &written, total) {
            inner.repos.retain(|project, _| key(project) != gone);
        }
    }

    /// A worktree was removed: its line maps go, its edit records stay for lookups through a
    /// commit.
    pub fn worktree_removed(&self, project: &str, worktree: &str, now_ms: u64) {
        let mut inner = self.inner();
        let repo = self.repo(&mut inner, project, now_ms);
        if repo.maps.remove(worktree).is_some() {
            inner.dirty.insert(project.to_owned());
        }
    }

    /// A project was removed from Hive: its repository's file goes with it.
    pub fn project_removed(&self, project: &str) {
        let mut inner = self.inner();
        inner.repos.remove(project);
        inner.dirty.remove(project);
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_file(dir.join(format!("{}.json", key(project))));
        }
    }

    /// `who_wrote`'s answer: the live origin of the line in the worktree, else through `git
    /// blame` and the records made before the commit. `title` names a session (and whether its
    /// log is gone) from its id and working folder.
    pub fn who_wrote(
        &self,
        projects: &[Project],
        title: &Title<'_>,
        asked: Asked,
        now_ms: u64,
    ) -> Control {
        let origin = self.origin(projects, title, &asked, now_ms);
        Control::LineAuthor {
            worktree: asked.worktree,
            path: asked.path,
            line: asked.line,
            side: asked.side,
            origin,
        }
    }

    fn origin(
        &self,
        projects: &[Project],
        title: &Title<'_>,
        asked: &Asked,
        now_ms: u64,
    ) -> LineOrigin {
        let Asked {
            worktree,
            path,
            line,
            side,
            base,
        } = asked;
        let (worktree, line, side) = (worktree.as_str(), *line, *side);
        let unknown = LineOrigin::Unknown { commit: None };
        let (Some(project), Some(rel)) = (project_of(projects, worktree), relative(path)) else {
            return unknown;
        };
        let at = (line as usize).saturating_sub(1);
        if line == 0 {
            return unknown;
        }
        if side == LineSide::Worktree {
            self.resync(&project.id, worktree, rel, Origin::Unknown, now_ms, false);
            let mut inner = self.inner();
            let repo = self.repo(&mut inner, &project.id, now_ms);
            let map = repo.maps.get_mut(worktree).and_then(|f| f.get_mut(rel));
            if let Some(map) = map.filter(|m| at < m.origins.len()) {
                map.used_ms = now_ms;
                match map.origins[at] {
                    Origin::Edit(id) => {
                        let run = map.run(at);
                        if let Some(edit) = repo.edits.iter().find(|e| e.id == id) {
                            return LineOrigin::Session {
                                edit: Box::new(line_edit(edit, &project.id, run, title)),
                                commit: None,
                            };
                        }
                    }
                    Origin::Editor(ms) => return LineOrigin::Editor { ms },
                    Origin::Unknown | Origin::Before => {}
                }
            }
        }
        let rev = match side {
            LineSide::Worktree => None,
            // The worktree is followed (checked above): its base is known.
            LineSide::Base => Some(
                crate::changes::against(projects, worktree, *base)
                    .ok()
                    .and_then(|against| against.commit)
                    .unwrap_or_else(|| "HEAD".to_owned()),
            ),
        };
        let dir = Path::new(worktree);
        let Some(blamed) = blame(dir, rel, line, rev.as_deref()) else {
            return unknown;
        };
        let Some(commit) = blamed.commit.clone() else {
            return unknown;
        };
        let anchor =
            at_commit(dir, &commit.id, &blamed.path, blamed.line).unwrap_or_else(|| Anchor {
                start: 0,
                lines: vec![blamed.hash],
                before: Vec::new(),
                after: Vec::new(),
            });
        let renamed = renamed_in(dir, &commit.id, &blamed.path);
        let mut inner = self.inner();
        let repo = self.repo(&mut inner, &project.id, now_ms);
        let until = blamed.committed_ms.saturating_add(COMMIT_SLACK_MS);
        let named = |p: &String| *p == blamed.path || p == rel || renamed.as_ref() == Some(p);
        let best = repo
            .edits
            .iter()
            .filter(|e| e.ms <= until && named(&e.path))
            .filter_map(|e| {
                e.runs()
                    .filter(|(_, hashes)| hashes.contains(&blamed.hash))
                    .filter_map(|(_, hashes)| anchor::relocate_scored(&anchor, &hashes, 0))
                    .map(|(_, score)| score)
                    .max()
                    .map(|score| (score, e.ms, e))
            })
            .max_by_key(|(score, ms, e)| (*score, *ms, e.id));
        match best {
            Some((_, _, edit)) => {
                let line = line as usize - 1;
                LineOrigin::Session {
                    edit: Box::new(line_edit(edit, &project.id, line..line + 1, title)),
                    commit: Some(commit),
                }
            }
            None => LineOrigin::Unknown {
                commit: Some(commit),
            },
        }
    }

    /// `session_files`' answer: the files the session `id` changed, from the records of every
    /// followed repository.
    pub fn session_files(&self, projects: &[Project], id: String, now_ms: u64) -> Control {
        let mut inner = self.inner();
        let mut files: BTreeMap<(String, String), SessionFile> = BTreeMap::new();
        for project in projects.iter().filter(|p| !p.group) {
            let repo = self.repo(&mut inner, &project.id, now_ms);
            let mine: HashSet<u64> = repo
                .edits
                .iter()
                .filter(|e| e.session == id)
                .map(|e| e.id)
                .collect();
            for edit in repo.edits.iter().filter(|e| e.session == id) {
                let entry = files
                    .entry((edit.worktree.clone(), edit.path.clone()))
                    .or_insert_with(|| {
                        let map = repo
                            .maps
                            .get(&edit.worktree)
                            .and_then(|f| f.get(&edit.path));
                        let lines = map.map_or_else(Vec::new, |m| {
                            let of = |o: &Origin| matches!(o, Origin::Edit(e) if mine.contains(e));
                            m.origins
                                .iter()
                                .enumerate()
                                .filter(|(_, o)| of(o))
                                .map(|(i, _)| i)
                                .collect()
                        });
                        SessionFile {
                            worktree: edit.worktree.clone(),
                            path: edit.path.clone(),
                            edits: 0,
                            lines_now: lines.len() as u32,
                            first_line: lines.first().map(|l| *l as u32 + 1),
                            last_ms: 0,
                        }
                    });
                entry.edits += 1;
                entry.last_ms = entry.last_ms.max(edit.ms);
            }
        }
        let mut files: Vec<SessionFile> = files.into_values().collect();
        files.sort_by(|a, b| b.last_ms.cmp(&a.last_ms).then(a.path.cmp(&b.path)));
        let truncated = files.len() > FILES_LIMIT;
        files.truncate(FILES_LIMIT);
        Control::SessionFiles {
            id,
            files,
            truncated,
        }
    }

    /// How many files each session changed, from the records of every followed repository.
    pub fn files_changed(&self, projects: &[Project], now_ms: u64) -> HashMap<String, u64> {
        let mut inner = self.inner();
        let mut files: HashMap<String, HashSet<(String, String)>> = HashMap::new();
        for project in projects.iter().filter(|p| !p.group) {
            let repo = self.repo(&mut inner, &project.id, now_ms);
            for edit in &repo.edits {
                let file = (edit.worktree.clone(), edit.path.clone());
                files.entry(edit.session.clone()).or_default().insert(file);
            }
        }
        files
            .into_iter()
            .map(|(session, files)| (session, files.len() as u64))
            .collect()
    }
}

/// A session's name from its id and working folder, and whether its log is gone.
pub type Title<'a> = dyn Fn(&str, &str) -> (Option<String>, bool) + 'a;

/// A `who_wrote` request.
pub struct Asked {
    pub worktree: String,
    pub path: String,
    pub line: u32,
    pub side: LineSide,
    pub base: hive_protocol::DiffBase,
}

/// The edit as the app shows it, for its lines `run` (0-based).
fn line_edit(edit: &Edit, project: &str, run: Range<usize>, title: &Title) -> LineEdit {
    let cwd = edit.cwd.as_deref().unwrap_or(&edit.worktree);
    let (title, deleted) = title(&edit.session, cwd);
    LineEdit {
        session: edit.session.clone(),
        title,
        deleted,
        project: Some(project.to_owned()),
        worktree: edit.worktree.clone(),
        agent: edit.agent.clone(),
        agent_type: edit.agent_type.clone(),
        tool: edit.tool.clone(),
        tool_use_id: edit.tool_use_id.clone(),
        ms: edit.ms,
        user_modified: edit.user_modified,
        start: run.start as u32 + 1,
        len: run.len() as u32,
    }
}

/// Drops the records past [`MAX_AGE_MS`] and the oldest past [`MAX_RECORDS`].
fn prune(repo: &mut Repo, now_ms: u64) {
    let oldest = now_ms.saturating_sub(MAX_AGE_MS);
    repo.edits.retain(|e| e.ms >= oldest);
    let over = repo.edits.len().saturating_sub(MAX_RECORDS);
    repo.edits.drain(..over);
}

/// The repository's JSON within [`REPO_BYTES`]: past it, the line maps of the files untouched
/// longest go first, then the oldest records.
fn fit(repo: &mut Repo) -> Vec<u8> {
    fit_within(repo, REPO_BYTES)
}

fn fit_within(repo: &mut Repo, most: usize) -> Vec<u8> {
    // Bounded by what can go: each pass drops a tenth of the maps, else of the records, or stops.
    loop {
        let json = serde_json::to_vec(&stored(repo)).unwrap_or_default();
        if json.len() <= most {
            return json;
        }
        let mut maps: Vec<(u64, String, String)> = repo
            .maps
            .iter()
            .flat_map(|(w, files)| {
                files
                    .iter()
                    .map(move |(rel, m)| (m.used_ms, w.clone(), rel.clone()))
            })
            .collect();
        if !maps.is_empty() {
            // A tenth of them, the untouched longest, per pass.
            maps.sort();
            let drop = (maps.len() / 10).max(1);
            let gone: HashSet<(String, String)> =
                maps.drain(..drop).map(|(_, w, rel)| (w, rel)).collect();
            for (worktree, files) in &mut repo.maps {
                files.retain(|rel, _| !gone.contains(&(worktree.clone(), rel.clone())));
            }
            repo.maps.retain(|_, files| !files.is_empty());
        } else if !repo.edits.is_empty() {
            let drop = (repo.edits.len() / 10).max(1);
            repo.edits.drain(..drop);
        } else {
            return json;
        }
    }
}

fn stored(repo: &Repo) -> Stored {
    Stored {
        version: VERSION,
        project: repo.project.clone(),
        next_id: repo.next_id,
        edits: repo.edits.clone(),
        worktrees: repo
            .maps
            .iter()
            .map(|(w, files)| {
                let files = files
                    .iter()
                    .map(|(rel, m)| (rel.clone(), m.stored()))
                    .collect();
                (w.clone(), files)
            })
            .collect(),
    }
}

/// Removes the least recently written repository files of `dir` (never one of `kept`) until
/// the rest fit in `most` bytes; their names.
fn evict_files(dir: &Path, kept: &HashSet<String>, most: u64) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(u64, u64, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let meta = e.metadata().ok()?;
            ((name.ends_with(".json") || name.ends_with(".json.bad")) && meta.is_file())
                .then(|| (mtime_ms(&meta), meta.len(), name))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    files.sort();
    let mut gone = Vec::new();
    for (_, size, name) in files {
        if total <= most {
            break;
        }
        if kept.contains(&name) || std::fs::remove_file(dir.join(&name)).is_err() {
            continue;
        }
        total -= size;
        gone.push(name.trim_end_matches(".json").to_owned());
    }
    gone
}

/// What `git blame` says of one line.
#[derive(Debug, PartialEq)]
struct Blamed {
    /// `None`: not committed yet.
    commit: Option<LineCommit>,
    committed_ms: u64,
    /// The file's path and the line (1-based) in the commit.
    path: String,
    line: usize,
    /// The line's hash (its text is dropped).
    hash: u64,
}

/// `git blame --porcelain -L line,line [rev] -- path` in `dir`.
fn blame(dir: &Path, path: &str, line: u32, rev: Option<&str>) -> Option<Blamed> {
    let range = format!("{line},{line}");
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "--literal-pathspecs",
        "blame",
        "--porcelain",
        "-L",
        &range,
    ];
    args.extend(rev);
    args.extend(["--", path]);
    let out = git::output_within(dir, &args, &[0], GIT_TIME).ok()?;
    parse_blame(&out)
}

/// The porcelain of a one-line `git blame`.
fn parse_blame(out: &[u8]) -> Option<Blamed> {
    let mut lines = out.split(|&b| b == b'\n');
    let head = String::from_utf8_lossy(lines.next()?).into_owned();
    let mut words = head.split(' ');
    let id = words.next()?.to_owned();
    let orig: usize = words.next()?.parse().ok()?;
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut text = None;
    for line in lines {
        if let Some(content) = line.strip_prefix(b"\t") {
            text = Some(anchor::line_hash(content));
            break;
        }
        let line = String::from_utf8_lossy(line);
        if let Some((key, value)) = line.split_once(' ') {
            fields.insert(key.to_owned(), value.to_owned());
        }
    }
    let field = |key: &str| fields.get(key).cloned().unwrap_or_default();
    let secs = |key: &str| field(key).parse::<u64>().unwrap_or(0).saturating_mul(1000);
    let uncommitted = !id.is_empty() && id.bytes().all(|b| b == b'0');
    let commit = (!uncommitted).then(|| LineCommit {
        id: id.clone(),
        author: adapter::clip(&field("author"), 200),
        ms: secs("author-time"),
        summary: adapter::clip(&field("summary"), 200),
    });
    Some(Blamed {
        commit,
        committed_ms: secs("committer-time"),
        path: fields.get("filename")?.clone(),
        line: orig,
        hash: text?,
    })
}

/// Where `commit` renamed `path` from, if it did (`git diff-tree -M`).
fn renamed_in(dir: &Path, commit: &str, path: &str) -> Option<String> {
    let args = [
        "-c",
        "core.quotePath=false",
        "diff-tree",
        "--no-commit-id",
        "--root",
        "-r",
        "-M",
        "-z",
        "--name-status",
        commit,
    ];
    let out = git::output_within(dir, &args, &[0], GIT_TIME).ok()?;
    parse_renames(&out, path)
}

/// The source of the rename to `path` in `--name-status -z` output (`R<score>\0from\0to\0`).
fn parse_renames(out: &[u8], path: &str) -> Option<String> {
    let mut fields = out.split(|&b| b == 0);
    while let Some(status) = fields.next() {
        let first = fields.next()?;
        if status.first() == Some(&b'R') || status.first() == Some(&b'C') {
            let to = fields.next()?;
            if to == path.as_bytes() {
                return String::from_utf8(first.to_vec()).ok();
            }
        }
    }
    None
}

/// The anchor of `line` (1-based) of `path` at `commit`, with [`CONTEXT`] lines around it.
fn at_commit(dir: &Path, commit: &str, path: &str, line: usize) -> Option<Anchor> {
    let args = [
        "-c",
        "core.precomposeUnicode=false",
        "--literal-pathspecs",
        "ls-tree",
        "-l",
        "-z",
        commit,
        "--",
        path,
    ];
    let listed = git::output_within(dir, &args, &[0], GIT_TIME).ok()?;
    let (oid, size) = file::parse_ls_tree(&listed)?;
    if size > FILE_BYTES {
        return None;
    }
    let bytes = git::output_within(dir, &["cat-file", "blob", &oid], &[0], GIT_TIME).ok()?;
    let hashes = anchor::hashes(&bytes, FILE_LINES)?;
    let at = line.checked_sub(1)?;
    (at < hashes.len()).then(|| Anchor::at(&hashes, at..at + 1, CONTEXT))
}

#[cfg(test)]
mod tests;
