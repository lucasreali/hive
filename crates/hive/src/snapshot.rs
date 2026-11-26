//! Snapshots of a worktree at the start of each agent turn (27.2, `docs/spike/diff-bases.md`),
//! the base of "Since the agent's last turn".
//!
//! A snapshot is a git tree written through a **snapshot index** of Hive's (`GIT_INDEX_FILE`,
//! seeded from a copy of the user's index, so only changed files are hashed) into **Hive's own
//! object store**: a bare repository in Hive's folder (`snapshots/<repo>/store.git`) whose
//! `objects/info/alternates` names the repository's objects (`GIT_OBJECT_DIRECTORY`). Nothing
//! is created, moved or changed inside `.git` (#56, the human's reading of 2026-11-26: git
//! refreshing the time of an object it meets again is accepted). Refs in the store
//! (`refs/turns/<session>/<n>`) keep the kept snapshots alive across its prune, and
//! `turns.json` next to it lists them across a restart.
//!
//! Every git call of a snapshot runs with the repository's hooks (`core.hooksPath` = an empty
//! folder of Hive's), its clean filters and the split index turned off, as well as fsmonitor
//! ([`git::command`]): no program of the repository runs, nothing goes into `.git`.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::git::{self, Stdout, read_limited};

/// The turns kept per session (the human, 2026-11-26).
pub const KEEP: usize = 10;
/// The longest a turn is kept (the human, 2026-11-26): 7 days.
pub const AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// The most bytes of one repository's store (the human, 2026-11-26): past it, its oldest
/// snapshots are dropped.
pub const SIZE_CAP: u64 = 1 << 30;
/// The longest one git call of a snapshot may take: a first one after a big checkout hashes
/// much.
const TIME_LIMIT: Duration = Duration::from_secs(60);
/// Most bytes of a git call's output.
const OUTPUT_LIMIT: u64 = 16 << 20;
/// Most bytes of a store's `turns.json`.
const RECORD_LIMIT: u64 = 1 << 20;
/// Most bytes of an index's listing (`git ls-files --stage`), read to find its gitlinks.
const LISTING_LIMIT: u64 = 256 << 20;
/// The most nested repositories (submodules, embedded repositories) a snapshot looks into.
const NESTED: usize = 256;
/// A store is pruned at most this often, unless snapshots were dropped.
const PRUNE_EVERY: Duration = Duration::from_secs(60);
/// The tools whose `PostToolUse` is an edit: one before the snapshot is done makes it late.
pub const EDITS: [&str; 4] = ["Edit", "MultiEdit", "Write", "NotebookEdit"];

/// One kept snapshot: the worktree `worktree` at the start of turn `n` of `session`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub session: String,
    pub n: u64,
    /// The tree id, from git's own output.
    pub tree: String,
    /// The worktree's id (its path).
    pub worktree: String,
    pub started_ms: u64,
    /// It finished after the agent's first edit of the turn: that edit may be in it.
    pub late: bool,
}

/// A store's `turns.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    /// The repository's common folder, whose objects are the store's alternate.
    repo: PathBuf,
    turns: Vec<Turn>,
}

/// The snapshots of every repository, under Hive's `snapshots` folder.
pub struct Snapshots {
    root: PathBuf,
    /// One lock per store (by folder), holding when it was last pruned: a snapshot, a listing
    /// through its index and a prune never run at once.
    stores: Mutex<HashMap<PathBuf, Arc<Mutex<Instant>>>>,
    /// The sessions whose snapshot is being taken, and whether they edited a file meanwhile.
    pending: Mutex<HashMap<String, bool>>,
    size_cap: u64,
    prune_every: Duration,
}

/// The repository of a worktree, as git names it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Repo {
    /// Its common folder (`.git` of the main worktree).
    common: PathBuf,
    /// The worktree's own index (a linked worktree's is in `.git/worktrees/<name>`).
    index: PathBuf,
    /// `sha1` or `sha256`.
    format: String,
}

/// One repository's store: its folder under [`Snapshots`]'s root.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Store {
    dir: PathBuf,
}

/// What "since this turn" compares with: a kept snapshot of a worktree, with what listing
/// against it needs.
#[derive(Debug)]
pub struct Since {
    pub turn: Turn,
    worktree: PathBuf,
    store: Store,
    repo: Repo,
    lock: Arc<Mutex<Instant>>,
    /// The store's size cap ([`SIZE_CAP`]).
    size_cap: u64,
}

impl PartialEq for Since {
    fn eq(&self, other: &Self) -> bool {
        (&self.turn, &self.worktree, &self.store) == (&other.turn, &other.worktree, &other.store)
    }
}

impl Eq for Since {}

impl Snapshots {
    /// The snapshots kept under `root` (Hive's `snapshots` folder).
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            stores: Mutex::default(),
            pending: Mutex::default(),
            size_cap: SIZE_CAP,
            prune_every: PRUNE_EVERY,
        }
    }

    /// With another size cap and prune interval (tests).
    pub fn with_limits(self, size_cap: u64, prune_every: Duration) -> Self {
        Self {
            size_cap,
            prune_every,
            ..self
        }
    }

    /// A snapshot of `session` is about to be taken.
    pub fn started(&self, session: &str) {
        lock(&self.pending).insert(session.to_owned(), false);
    }

    /// `session` edited a file: its snapshot under way, if any, is late.
    pub fn edited(&self, session: &str) {
        if let Some(late) = lock(&self.pending).get_mut(session) {
            *late = true;
        }
    }

    /// Whether `session` edited a file since its snapshot started; it is no longer pending.
    fn finished(&self, session: &str) -> bool {
        lock(&self.pending).remove(session).unwrap_or(false)
    }

    /// Takes the snapshot of the worktree at `worktree` (id `id`) for a new turn of `session`
    /// started at `now_ms`, keeps it, and drops what the caps leave out. Runs git: call it off
    /// the hook's path, on a blocking thread.
    pub fn take(&self, worktree: &Path, id: &str, session: &str, now_ms: u64) -> io::Result<Turn> {
        let taken = self.taken(worktree, id, session, now_ms);
        // Never left pending, whatever happened.
        self.finished(session);
        taken
    }

    fn taken(&self, worktree: &Path, id: &str, session: &str, now_ms: u64) -> io::Result<Turn> {
        valid(session)?;
        let repo = Repo::of(worktree)?;
        let (store, held) = self.store(&repo);
        let mut pruned = lock(&held);
        // A record that cannot be read would leave refs no turn names, keeping their objects
        // forever: the store starts again.
        if store.read_whole().is_none() {
            std::fs::remove_dir_all(&store.dir)?;
        }
        store.create(&repo)?;
        let tree = store.snapshot(worktree, &repo, &store.index(id))?;
        // An edit after this point is not in the tree.
        let late = self.finished(session);
        let mut record = store.read();
        record.repo.clone_from(&repo.common);
        let n = record
            .turns
            .iter()
            .filter(|t| t.session == session)
            .map(|t| t.n)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let turn = Turn {
            session: session.to_owned(),
            n,
            tree,
            worktree: id.to_owned(),
            started_ms: now_ms,
            late,
        };
        store.set_ref(&turn)?;
        record.turns.push(turn.clone());
        let dropped = trim(&mut record.turns, now_ms, |w| Path::new(w).exists());
        store.drop_refs(&dropped)?;
        store.write(&record)?;
        if !dropped.is_empty() || pruned.elapsed() >= self.prune_every {
            store.prune()?;
            *pruned = Instant::now();
        }
        // Past the cap, the oldest go first; at most every turn kept.
        while store.size() > self.size_cap {
            let Some(oldest) = record.turns.pop() else {
                break;
            };
            store.drop_refs(std::slice::from_ref(&oldest))?;
            store.write(&record)?;
            store.prune()?;
        }
        if !record.turns.contains(&turn) {
            return Err(io::Error::other(
                "this turn's snapshot was too large to keep",
            ));
        }
        Ok(turn)
    }

    /// The store of `repo`, and its lock.
    fn store(&self, repo: &Repo) -> (Store, Arc<Mutex<Instant>>) {
        let key = format!("{:016x}", fnv(repo.common.as_os_str().as_encoded_bytes()));
        let store = Store {
            dir: self.root.join(key),
        };
        let held = self.held(&store);
        (store, held)
    }

    /// The lock of `store`.
    fn held(&self, store: &Store) -> Arc<Mutex<Instant>> {
        let mut stores = lock(&self.stores);
        let held = stores.entry(store.dir.clone());
        held.or_insert_with(|| Arc::new(Mutex::new(Instant::now())))
            .clone()
    }

    /// The last kept turn of each session whose last snapshot is of the worktree at
    /// `worktree` (id `id`), the latest first; none when git cannot name its repository.
    pub fn turns(&self, worktree: &Path, id: &str) -> Vec<Turn> {
        let Ok(repo) = Repo::of(worktree) else {
            return Vec::new();
        };
        let (store, _) = self.store(&repo);
        let mut all = store.read().turns;
        // Each session's highest first: the one kept.
        all.sort_by_key(|t| Reverse(t.n));
        let mut last: HashMap<String, Turn> = HashMap::new();
        for turn in all {
            last.entry(turn.session.clone()).or_insert(turn);
        }
        let mut turns: Vec<Turn> = last.into_values().filter(|t| t.worktree == id).collect();
        turns.sort_by_key(|t| Reverse((t.started_ms, t.session.clone())));
        turns
    }

    /// The last kept turn of `session`, when it is of the worktree at `worktree` (id `id`).
    pub fn since(&self, worktree: &Path, id: &str, session: &str) -> io::Result<Since> {
        valid(session)?;
        let turn = self
            .turns(worktree, id)
            .into_iter()
            .find(|t| t.session == session);
        let turn = turn.ok_or_else(|| {
            io::Error::other("No snapshot of this agent's last turn in this worktree")
        })?;
        let repo = Repo::of(worktree)?;
        let (store, lock) = self.store(&repo);
        Ok(Since {
            turn,
            worktree: worktree.to_owned(),
            store,
            repo,
            lock,
            size_cap: self.size_cap,
        })
    }

    /// Drops every snapshot of `session` (its `SessionEnd`); a store left with none goes.
    pub fn end(&self, session: &str) {
        self.each(|turns| {
            let (gone, kept) = turns.drain(..).partition(|t| t.session == session);
            *turns = kept;
            gone
        });
    }

    /// Drops what is older than the age cap at `now_ms` or of a worktree that is gone, and
    /// every store whose repository is gone or that holds no snapshot (at start).
    pub fn sweep(&self, now_ms: u64) {
        self.each(|turns| trim(turns, now_ms, |w| Path::new(w).exists()));
    }

    /// Runs `drop` on the turns of every store, which returns the turns it removed; those are
    /// dropped, and a store left with none, or whose repository is gone, is deleted.
    fn each(&self, mut drop: impl FnMut(&mut Vec<Turn>) -> Vec<Turn>) {
        let Ok(dirs) = std::fs::read_dir(&self.root) else {
            return;
        };
        for dir in dirs.filter_map(Result::ok) {
            let store = Store { dir: dir.path() };
            let held = self.held(&store);
            let _held = lock(&held);
            let mut record = store.read();
            let dropped = drop(&mut record.turns);
            let done = match () {
                () if record.turns.is_empty() || !record.repo.exists() => {
                    std::fs::remove_dir_all(&store.dir)
                }
                () if dropped.is_empty() => Ok(()),
                () => store
                    .drop_refs(&dropped)
                    .and_then(|()| store.write(&record))
                    .and_then(|()| store.prune()),
            };
            if let Err(err) = done {
                eprintln!("hive: warning: cannot drop snapshots: {err}");
            }
        }
    }
}

impl Since {
    /// What changed in the worktree since the turn, through the worktree's snapshot index
    /// refreshed now: `git diff-index --cached --raw` and `--numstat` output, in that order.
    pub fn list(&self) -> io::Result<(Vec<u8>, Vec<u8>)> {
        self.diff(true)
    }

    /// [`Since::list`]'s `--raw` output only (where a file renamed since the turn came from).
    pub fn raw(&self) -> io::Result<Vec<u8>> {
        self.diff(false).map(|(raw, _)| raw)
    }

    /// Refreshes the worktree's snapshot index, then `git diff-index --cached` against the
    /// turn's tree: `--raw`, and `--numstat` when asked (else empty).
    fn diff(&self, numstat: bool) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let _held = lock(&self.lock);
        let index = self.store.index(&self.turn.worktree);
        self.store.refresh(&self.worktree, &self.repo, &index)?;
        let run = |options: &[&str]| {
            let mut diff = vec!["diff-index", "--cached", "-z", "--find-renames"];
            diff.extend_from_slice(options);
            diff.extend(["--end-of-options", &self.turn.tree]);
            self.store.work(&self.worktree, &index, &args(&diff))
        };
        let raw = run(&["--raw"])?;
        let counts = ["--numstat", "--no-ext-diff", "--no-textconv"];
        let numstat = if numstat { run(&counts)? } else { Vec::new() };
        // A listing hashes what changed into the store: past its cap, what no snapshot keeps
        // goes now rather than at the next snapshot.
        if self.store.size() > self.size_cap {
            self.store.prune()?;
        }
        Ok((raw, numstat))
    }

    /// `git ls-tree -l -z` of `path` in the turn's tree. `path` is taken byte for byte.
    pub fn ls_tree(&self, path: &str) -> io::Result<Vec<u8>> {
        let tree = self.turn.tree.as_str();
        let ls = [
            "-c",
            "core.precomposeUnicode=false",
            "--literal-pathspecs",
            "ls-tree",
            "-l",
            "-z",
            tree,
            "--",
            path,
        ];
        let index = self.store.index(&self.turn.worktree);
        self.store.work(&self.worktree, &index, &args(&ls))
    }

    /// The blob `oid` (from [`Since::ls_tree`]'s output).
    pub fn blob(&self, oid: &str) -> io::Result<Vec<u8>> {
        let index = self.store.index(&self.turn.worktree);
        let cat = args(&["cat-file", "blob", "--end-of-options", oid]);
        self.store.work(&self.worktree, &index, &cat)
    }
}

impl Repo {
    /// The repository of the worktree at `dir`.
    fn of(dir: &Path) -> io::Result<Self> {
        let asked = [
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
            "--git-path",
            "index",
            "--show-object-format",
        ];
        let out = git::output(dir, &asked, &[0])?;
        let mut lines = out.split(|&b| b == b'\n').filter(|line| !line.is_empty());
        let mut next = || {
            let line = lines.next();
            line.ok_or("git did not name the repository")
                .map_err(io::Error::other)
        };
        let common = PathBuf::from(git::os_string(next()?));
        let index = dir.join(git::os_string(next()?));
        // Git's own output, passed back to git as one argument.
        let format = String::from_utf8_lossy(next()?).into_owned();
        Ok(Self {
            common,
            index,
            format,
        })
    }
}

impl Store {
    fn bare(&self) -> PathBuf {
        self.dir.join("store.git")
    }

    fn objects(&self) -> PathBuf {
        self.bare().join("objects")
    }

    /// The empty folder git is told holds the hooks: none runs.
    fn hooks(&self) -> PathBuf {
        self.dir.join("no-hooks")
    }

    /// The snapshot index of the worktree `id`.
    fn index(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{:016x}.index", fnv(id.as_bytes())))
    }

    fn record(&self) -> PathBuf {
        self.dir.join("turns.json")
    }

    /// Creates the store of `repo` (once) and names the repository's objects as its alternate.
    fn create(&self, repo: &Repo) -> io::Result<()> {
        for dir in [&self.dir, &self.hooks()] {
            crate::mode::folder(std::fs::DirBuilder::new().recursive(true), 0o700).create(dir)?;
        }
        if !self.bare().join("HEAD").exists() {
            let format = format!("--object-format={}", repo.format);
            let init = ["init", "--bare", "--quiet", "--template=", &format];
            let mut init: Vec<&OsStr> = args(&init);
            let bare = self.bare();
            init.push(bare.as_os_str());
            self.run(self.command(&self.dir)?, &init, &[])?;
        }
        let info = self.objects().join("info");
        let alternates = info.join("alternates");
        let mut line = repo.common.join("objects").into_os_string();
        line.push("\n");
        let line = line.as_encoded_bytes();
        if std::fs::read(&alternates).ok().as_deref() != Some(line) {
            std::fs::create_dir_all(&info)?;
            crate::wrapper::write_atomic(&alternates, line, 0o600)?;
        }
        Ok(())
    }

    /// `git -C <dir>` with the repository's hooks and the split index turned off.
    fn command(&self, dir: &Path) -> io::Result<Command> {
        let mut command = git::command(dir)?;
        let mut hooks = OsString::from("core.hooksPath=");
        hooks.push(self.hooks());
        command
            .arg("-c")
            .arg(hooks)
            .args(["-c", "core.splitIndex=false"])
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES");
        Ok(command)
    }

    /// Runs `command` with `args` and `input`, within [`TIME_LIMIT`].
    fn run(&self, mut command: Command, args: &[&OsStr], input: &[u8]) -> io::Result<Vec<u8>> {
        command.args(args);
        let keep = Stdout::Max(OUTPUT_LIMIT);
        git::limited(command, "git", args, input, &[0], keep, Some(TIME_LIMIT))
    }

    /// `git <args>` in the store.
    fn git(&self, args: &[&OsStr], input: &[u8]) -> io::Result<Vec<u8>> {
        let mut command = self.command(&self.dir)?;
        command.arg("--git-dir").arg(self.bare());
        self.run(command, args, input)
    }

    /// `git <args>` in the worktree at `worktree` with `index` as its index and the store as
    /// its object folder, its filters turned off.
    fn work(&self, worktree: &Path, index: &Path, args: &[&OsStr]) -> io::Result<Vec<u8>> {
        self.work_with(worktree, index, &filters(worktree)?, args)
    }

    /// [`Store::work`] with the `-c` options `filters`.
    fn work_with(
        &self,
        worktree: &Path,
        index: &Path,
        filters: &[String],
        args: &[&OsStr],
    ) -> io::Result<Vec<u8>> {
        let mut command = self.command(worktree)?;
        command.args(filters);
        command
            .env("GIT_INDEX_FILE", index)
            .env("GIT_OBJECT_DIRECTORY", self.objects());
        self.run(command, args, &[])
    }

    /// The `-c` options turning off the filter drivers of the worktree at `worktree` and of
    /// every repository nested in it that git looks into: `git add -A` runs `git status` in each
    /// checked-out gitlink of `index` (a submodule, an embedded repository), which runs that
    /// repository's own filters, and so on down. The options reach those through
    /// `GIT_CONFIG_PARAMETERS`. At most `most` nested repositories.
    fn nested_filters(
        &self,
        worktree: &Path,
        index: &Path,
        most: usize,
    ) -> io::Result<Vec<String>> {
        let mut options = filters(worktree)?;
        let mut queue = self.gitlinks(worktree, Some(index))?;
        let mut seen = 0;
        while let Some(dir) = queue.pop() {
            seen += 1;
            if seen > most {
                return Err(io::Error::other("too many nested repositories to snapshot"));
            }
            // Not checked out: git looks into nothing there.
            if dir.join(".git").exists() {
                options.extend(filters(&dir)?);
                queue.extend(self.gitlinks(&dir, None)?);
            }
        }
        Ok(options)
    }

    /// The gitlinks of the repository at `dir`, through `index` (Hive's) or its own index.
    fn gitlinks(&self, dir: &Path, index: Option<&Path>) -> io::Result<Vec<PathBuf>> {
        let mut command = self.command(dir)?;
        if let Some(index) = index {
            command.env("GIT_INDEX_FILE", index);
        }
        command.args(["ls-files", "--stage", "-z"]);
        let keep = Stdout::Max(LISTING_LIMIT);
        let ls = args(&["ls-files", "--stage", "-z"]);
        let out = git::limited(command, "git", &ls, &[], &[0], keep, Some(TIME_LIMIT))?;
        let links = out.split(|&b| b == 0).filter_map(|entry| {
            let (meta, path) = entry.split_at(entry.iter().position(|&b| b == b'\t')?);
            let path = path.get(1..).filter(|p| !p.is_empty())?;
            meta.starts_with(b"160000 ")
                .then(|| dir.join(git::os_string(path)))
        });
        Ok(links.collect())
    }

    /// Brings `index` up to the worktree at `worktree` (`git add -A`): seeded from a copy of
    /// the user's index when it is missing (a read only). A failure removes it, so the next one
    /// starts afresh.
    fn refresh(&self, worktree: &Path, repo: &Repo, index: &Path) -> io::Result<()> {
        let refreshed = (|| {
            // No index yet (nothing ever staged): the snapshot index starts empty.
            if !index.exists() && repo.index.exists() {
                std::fs::copy(&repo.index, index)?;
            }
            let filters = self.nested_filters(worktree, index, NESTED)?;
            self.work_with(worktree, index, &filters, &args(&["add", "-A"]))
                .map(drop)
        })();
        if refreshed.is_err() {
            let mut lock = index.as_os_str().to_owned();
            lock.push(".lock");
            let _ = std::fs::remove_file(index);
            let _ = std::fs::remove_file(lock);
        }
        refreshed
    }

    /// The tree of the worktree at `worktree` now, through `index`.
    fn snapshot(&self, worktree: &Path, repo: &Repo, index: &Path) -> io::Result<String> {
        self.refresh(worktree, repo, index)?;
        let out = self.work(worktree, index, &args(&["write-tree"]))?;
        let tree = String::from_utf8_lossy(out.trim_ascii()).into_owned();
        // Git's own output, checked as a stored one is.
        let written = object_id(&tree).then_some(tree);
        written.ok_or("git wrote no tree").map_err(io::Error::other)
    }

    /// The ref keeping `turn`'s tree.
    fn set_ref(&self, turn: &Turn) -> io::Result<()> {
        let name = refname(turn);
        self.git(&args(&["update-ref", &name, &turn.tree]), &[])
            .map(drop)
    }

    /// Deletes the refs of `turns`.
    fn drop_refs(&self, turns: &[Turn]) -> io::Result<()> {
        if turns.is_empty() {
            return Ok(());
        }
        let input: String = turns
            .iter()
            .map(|t| format!("delete {}\n", refname(t)))
            .collect();
        self.git(&args(&["update-ref", "--stdin"]), input.as_bytes())
            .map(drop)
    }

    /// Removes every object no kept snapshot holds, and the snapshot indexes, which may name
    /// some of them (the next snapshot or listing seeds them again).
    fn prune(&self) -> io::Result<()> {
        self.git(&args(&["prune", "--expire=now"]), &[])?;
        for entry in std::fs::read_dir(&self.dir)?.filter_map(Result::ok) {
            if entry.path().extension() == Some(OsStr::new("index")) {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    /// The bytes of the store's objects.
    fn size(&self) -> u64 {
        let mut size = 0;
        let mut folders = vec![self.objects()];
        while let Some(folder) = folders.pop() {
            let entries = std::fs::read_dir(&folder).into_iter().flatten().flatten();
            let entries = entries.filter_map(|e| Some((e.path(), e.metadata().ok()?)));
            for (path, meta) in entries {
                if meta.is_dir() {
                    folders.push(path);
                } else {
                    size += meta.len();
                }
            }
        }
        size
    }

    /// The store's `turns.json`; an unreadable one holds none. A turn whose session or tree
    /// could not have been written by Hive (the file edited by hand) is left out.
    fn read(&self) -> Record {
        let mut record = self.read_whole().unwrap_or_default();
        record
            .turns
            .retain(|t| valid(&t.session).is_ok() && object_id(&t.tree));
        record
    }

    /// The store's `turns.json`: `None` when it is there but cannot be read (too large,
    /// corrupt), empty when it is not there.
    fn read_whole(&self) -> Option<Record> {
        let file = match std::fs::File::open(self.record()) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Some(Record::default()),
            file => file,
        };
        let bytes = file.and_then(|mut file| read_limited(&mut file, RECORD_LIMIT));
        serde_json::from_slice(&bytes.ok()?).ok()
    }

    fn write(&self, record: &Record) -> io::Result<()> {
        let json = serde_json::to_vec(record)?;
        crate::wrapper::write_atomic(&self.record(), &json, 0o600)
    }
}

/// Keeps the last [`KEEP`] turns of each session that are at most [`AGE_MS`] old at `now_ms`
/// and of a worktree that `exists`, the latest first; returns the others.
fn trim(turns: &mut Vec<Turn>, now_ms: u64, exists: impl Fn(&str) -> bool) -> Vec<Turn> {
    turns.sort_by_key(|t| Reverse((t.started_ms, t.n)));
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut dropped = Vec::new();
    turns.retain(|turn| {
        let count = counts.entry(turn.session.clone()).or_default();
        *count += 1;
        let keep = *count <= KEEP
            && now_ms.saturating_sub(turn.started_ms) <= AGE_MS
            && exists(&turn.worktree);
        if !keep {
            dropped.push(turn.clone());
        }
        keep
    });
    dropped
}

/// The `-c` options that turn off every filter driver the configuration of the worktree at
/// `dir` names: a driver only runs when configured, so these are all.
fn filters(dir: &Path) -> io::Result<Vec<String>> {
    let names = ["config", "--name-only", "--get-regexp", r"^filter\."];
    let out = git::output(dir, &names, &[0, 1])?;
    let mut options = Vec::new();
    let mut seen = Vec::new();
    for key in String::from_utf8_lossy(&out).lines() {
        let Some((name, _)) = key.strip_prefix("filter.").and_then(|k| k.rsplit_once('.')) else {
            continue;
        };
        // `-c` splits at the first `=`: such a driver could not be turned off.
        if name.contains('=') {
            return Err(io::Error::other(format!(
                "cannot turn off the filter {name}"
            )));
        }
        if seen.contains(&name) {
            continue;
        }
        seen.push(name);
        for (field, value) in [("clean", ""), ("process", ""), ("required", "false")] {
            options.extend(["-c".to_owned(), format!("filter.{name}.{field}={value}")]);
        }
    }
    Ok(options)
}

/// Whether `text` is an object id: 40 or 64 hex digits.
fn object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A session id from a hook payload (untrusted) is used in a ref and a file name only when it
/// is `[A-Za-z0-9_-]{1,64}`.
fn valid(session: &str) -> io::Result<()> {
    let fits = (1..=64).contains(&session.len())
        && session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if fits {
        Ok(())
    } else {
        Err(io::Error::other(
            "not a session id Hive keeps snapshots for",
        ))
    }
}

fn refname(turn: &Turn) -> String {
    format!("refs/turns/{}/{}", turn.session, turn.n)
}

fn args<'a>(args: &[&'a str]) -> Vec<&'a OsStr> {
    args.iter().map(|&arg| OsStr::new(arg)).collect()
}

/// FNV-1a: a short name that stays the same across runs and versions.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
