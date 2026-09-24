//! Claude Code's session logs of the followed projects, for the sidebar's Sessions: every
//! `<claude dir>/projects/<encoded cwd>/<id>.jsonl` whose `cwd` lies in a followed worktree.
//! Logs are only read, except when the user deletes one.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use hive_protocol::{Project, Session, SessionRole};
use serde_json::Value;

use crate::projects;

/// Most bytes read from one log; a longer one is summarized from its start.
const LOG_LIMIT: u64 = 67_108_864; // 64 MiB
/// Most characters kept of a title or a message.
const TEXT_LIMIT: usize = 300;
/// Longest session id accepted (Claude's are 36-character UUIDs).
const ID_LIMIT: usize = 64;

/// Where Claude Code keeps its session logs: `$CLAUDE_CONFIG_DIR/projects`, else
/// `$HOME/.claude/projects`; `None` without either.
pub fn root(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |key| var(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("CLAUDE_CONFIG_DIR")
        .or_else(|| var("HOME").map(|home| home.join(".claude")))
        .map(|dir| dir.join("projects"))
}

/// What a log says about its session, read once per version of the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub cwd: Option<String>,
    /// A title set by the user, else Claude's.
    pub title: Option<String>,
    pub first_prompt: Option<String>,
    pub last: Option<(SessionRole, String)>,
    /// User and assistant messages with text.
    pub messages: u64,
    pub model: Option<String>,
    pub branch: Option<String>,
}

/// Summarizes a JSONL log, at most [`LOG_LIMIT`] bytes of it. Lines that are not JSON are
/// skipped, and so are subagent (sidechain) and meta messages.
pub fn summarize(log: &mut dyn Read) -> Summary {
    let mut summary = Summary::default();
    let (mut custom, mut ai) = (None, None);
    let reader = BufReader::new(log.take(LOG_LIMIT));
    for line in reader.split(b'\n').map_while(Result::ok) {
        let Ok(record) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let text = |key: &str| record.get(key).and_then(Value::as_str).map(cut);
        match record.get("type").and_then(Value::as_str) {
            Some("custom-title") => custom = text("customTitle"),
            Some("ai-title") => ai = text("aiTitle"),
            Some(kind @ ("user" | "assistant")) => {
                if summary.cwd.is_none() {
                    summary.cwd = text("cwd");
                }
                if let Some(branch) = text("gitBranch") {
                    summary.branch = Some(branch);
                }
                let flag = |key: &str| record.get(key).and_then(Value::as_bool) == Some(true);
                if flag("isSidechain") || flag("isMeta") {
                    continue;
                }
                let message = record.get("message");
                if kind == "assistant"
                    && let Some(model) =
                        message.and_then(|m| m.get("model")).and_then(Value::as_str)
                    && !model.starts_with('<')
                {
                    summary.model = Some(model.to_owned());
                }
                let Some(said) = message.and_then(|m| m.get("content")).and_then(first_text) else {
                    continue;
                };
                let role = if kind == "user" {
                    SessionRole::User
                } else {
                    SessionRole::Assistant
                };
                // Slash commands and their output come as tags, not as something the user wrote.
                if role == SessionRole::User
                    && summary.first_prompt.is_none()
                    && !said.starts_with('<')
                {
                    summary.first_prompt = Some(said.clone());
                }
                summary.messages += 1;
                summary.last = Some((role, said));
            }
            _ => {}
        }
    }
    summary.title = custom.or(ai);
    summary
}

/// A message's text: the content itself, or its first non-empty text block; trimmed and cut.
fn first_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(text) => Some(text.as_str()),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .find(|t| !t.trim().is_empty()),
        _ => None,
    }?;
    Some(cut(text.trim())).filter(|t| !t.is_empty())
}

fn cut(text: &str) -> String {
    text.chars().take(TEXT_LIMIT).collect()
}

/// `path` with every character other than an ASCII letter or digit as `-`, which is how Claude
/// names a project's log folder (it turns at least `/` and `.` into `-`); comparing two paths
/// this way tells whether a folder may hold a project's logs.
fn normalized(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A session id as Claude writes them: letters, digits and `-`, not too long.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_LIMIT
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The logs of one Claude directory, with each log's summary kept until the file changes.
pub struct Sessions {
    root: Option<PathBuf>,
    cache: Mutex<HashMap<PathBuf, (SystemTime, u64, Summary)>>,
}

impl Sessions {
    pub fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Every session whose `cwd` lies in a followed worktree, the most recent first.
    pub fn list(&self, projects: &[Project]) -> io::Result<Vec<Session>> {
        let Some(root) = &self.root else {
            return Ok(Vec::new());
        };
        let prefixes: Vec<String> = projects.iter().map(|p| normalized(&p.path)).collect();
        let dirs = match std::fs::read_dir(root) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            dirs => dirs?,
        };
        let mut sessions = Vec::new();
        for dir in dirs.flatten() {
            let name = normalized(&dir.file_name().to_string_lossy());
            if !prefixes.iter().any(|p| name.starts_with(p.as_str())) {
                continue;
            }
            let Ok(logs) = std::fs::read_dir(dir.path()) else {
                continue;
            };
            for log in logs.flatten() {
                if let Some(session) = self.session(projects, &log.path()) {
                    sessions.push(session);
                }
            }
        }
        sessions.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then(a.id.cmp(&b.id)));
        Ok(sessions)
    }

    /// The listed session `id`.
    pub fn find(&self, projects: &[Project], id: &str) -> io::Result<Session> {
        if !valid_id(id) {
            return Err(io::Error::other(format!("invalid session id {id:?}")));
        }
        self.list(projects)?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| io::Error::other(format!("no session {id} in the followed projects")))
    }

    /// Deletes the listed session `id`: its log and the folder of the same name beside it
    /// (its subagents' logs), if any.
    pub fn delete(&self, projects: &[Project], id: &str) -> io::Result<()> {
        let log = PathBuf::from(self.find(projects, id)?.log);
        std::fs::remove_file(&log)?;
        let folder = log.with_extension("");
        if folder.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            std::fs::remove_dir_all(&folder)?;
        }
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&log);
        Ok(())
    }

    /// The session logged at `path`, when it is a regular `<id>.jsonl` file whose `cwd` lies
    /// in a followed worktree.
    fn session(&self, projects: &[Project], path: &Path) -> Option<Session> {
        let id = path
            .file_name()?
            .to_str()?
            .strip_suffix(".jsonl")
            .filter(|id| valid_id(id))?;
        let meta = path.symlink_metadata().ok().filter(|m| m.is_file())?;
        let modified = meta.modified().ok()?;
        let summary = self.summary(path, modified, meta.len())?;
        let cwd = summary.cwd.clone()?;
        let (project, worktree) = projects::place(projects, &cwd)?;
        let updated_ms = modified
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let (last_role, last_text) = summary.last.clone().unzip();
        Some(Session {
            id: id.to_owned(),
            project,
            worktree,
            cwd,
            title: summary.title.or(summary.first_prompt),
            last_role,
            last_text,
            messages: summary.messages,
            model: summary.model,
            branch: summary.branch,
            updated_ms,
            log: path.to_string_lossy().into_owned(),
        })
    }

    fn summary(&self, path: &Path, modified: SystemTime, len: u64) -> Option<Summary> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((m, l, summary)) = cache.get(path)
            && (*m, *l) == (modified, len)
        {
            return Some(summary.clone());
        }
        let summary = summarize(&mut File::open(path).ok()?);
        cache.insert(path.to_owned(), (modified, len, summary.clone()));
        Some(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_protocol::Worktree;

    fn var<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.into())
    }

    #[test]
    fn the_root_follows_claude_config_dir_then_home() {
        let root = |vars: &[(&str, &str)]| super::root(var(vars));
        assert_eq!(
            root(&[("CLAUDE_CONFIG_DIR", "/c"), ("HOME", "/h")]),
            Some("/c/projects".into())
        );
        assert_eq!(
            root(&[("CLAUDE_CONFIG_DIR", ""), ("HOME", "/h")]),
            Some("/h/.claude/projects".into())
        );
        assert_eq!(root(&[]), None);
    }

    const LOG: &str = r#"{"type":"mode","mode":"x"}
not json
{"type":"user","cwd":"/r/src","gitBranch":"main","message":{"role":"user","content":"<command-name>/clear</command-name>"}}
{"type":"user","cwd":"/elsewhere","message":{"role":"user","content":"  Fix the login  "}}
{"type":"assistant","message":{"model":"claude-x","content":[{"type":"tool_use"},{"type":"text","text":" "},{"type":"text","text":"On it."}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}
{"type":"assistant","isSidechain":true,"message":{"model":"claude-sub","content":"sub"}}
{"type":"user","isMeta":true,"message":{"content":"meta"}}
{"type":"user","message":{"role":"user","content":"Thanks, go on"}}
{"type":"assistant","gitBranch":"feat","message":{"model":"<synthetic>","content":"Done."}}
{"type":"ai-title","aiTitle":"Login fix"}
{"type":"user","message":{"content":7}}"#;

    #[test]
    fn a_log_is_summarized() {
        let summary = summarize(&mut LOG.as_bytes());
        assert_eq!(
            summary,
            Summary {
                cwd: Some("/r/src".into()),
                title: Some("Login fix".into()),
                first_prompt: Some("Fix the login".into()),
                last: Some((SessionRole::Assistant, "Done.".into())),
                messages: 5,
                model: Some("claude-x".into()),
                branch: Some("feat".into()),
            }
        );
        // A title the user set wins over Claude's, wherever it is.
        let custom = format!("{{\"type\":\"custom-title\",\"customTitle\":\"Mine\"}}\n{LOG}");
        assert_eq!(summarize(&mut custom.as_bytes()).title, Some("Mine".into()));
        assert_eq!(summarize(&mut &b""[..]), Summary::default());
        let long = format!(
            r#"{{"type":"ai-title","aiTitle":"{}"}}"#,
            "é".repeat(TEXT_LIMIT + 1)
        );
        let title = summarize(&mut long.as_bytes()).title.unwrap();
        assert_eq!(title.chars().count(), TEXT_LIMIT);
    }

    #[test]
    fn ids_and_folder_names() {
        assert!(valid_id("57a46179-c9ac-4bbb-82f1-abc8160adf58"));
        for id in ["", "../x", "a b", "a/b", &"a".repeat(ID_LIMIT + 1)] {
            assert!(!valid_id(id), "{id:?}");
        }
        assert!(valid_id(&"a".repeat(ID_LIMIT)));
        assert_eq!(normalized("/home/me/my.app_x"), "-home-me-my-app-x");
    }

    fn project(path: &str) -> Project {
        let wt = |path: &str, main: bool| Worktree {
            id: path.into(),
            name: path.into(),
            path: path.into(),
            branch: None,
            main,
            claude: !main,
        };
        Project {
            id: path.into(),
            name: path.into(),
            path: path.into(),
            worktrees: vec![
                wt(path, true),
                wt(&format!("{path}/.claude/worktrees/w"), false),
            ],
            error: None,
        }
    }

    fn log(dir: &Path, name: &str, cwd: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        let line =
            format!(r#"{{"type":"user","cwd":"{cwd}","message":{{"content":"hi {name}"}}}}"#);
        std::fs::write(&path, line).unwrap();
        path
    }

    #[test]
    fn sessions_of_followed_projects_are_listed_newest_first_and_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("projects");
        let repo = tmp.path().join("repo");
        let repo = repo.to_str().unwrap();
        let folder = root.join(normalized(repo));
        let older = log(&folder, "a.jsonl", repo);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let wt = format!("{repo}/.claude/worktrees/w");
        log(&root.join(normalized(&wt)), "b.jsonl", &format!("{wt}/src"));
        // Not a session: a bad id, not a log, a symlink, a folder, a cwd elsewhere, a
        // project that is not followed.
        log(&folder, "bad id.jsonl", repo);
        log(&folder, "notes.txt", repo);
        std::os::unix::fs::symlink(&older, folder.join("c.jsonl")).unwrap();
        std::fs::create_dir_all(folder.join("d.jsonl")).unwrap();
        log(&folder, "e.jsonl", "/elsewhere");
        log(&root.join("-other"), "f.jsonl", "/other");
        std::fs::write(root.join("stray-file"), "").unwrap();
        // A file whose name starts like the project's folder is no folder of logs.
        std::fs::write(root.join(format!("{}-x", normalized(repo))), "").unwrap();
        // A session's own folder (subagents' logs).
        std::fs::create_dir_all(folder.join("a/subagents")).unwrap();

        let sessions = Sessions::new(Some(root.clone()));
        let followed = [project(repo), project("/other-not-followed")];
        let list = sessions.list(&followed).unwrap();
        let got: Vec<_> = list
            .iter()
            .map(|s| (s.id.as_str(), s.worktree.as_str(), s.title.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("b", wt.as_str(), Some("hi b.jsonl")),
                ("a", repo, Some("hi a.jsonl")),
            ]
        );
        assert_eq!(list[1].log, older.to_string_lossy());
        assert!(list[1].updated_ms > 0);

        // A summary is kept while the log's time and size stay the same, and read again
        // when they change.
        let modified = older.metadata().unwrap().modified().unwrap();
        let same_size = std::fs::read_to_string(&older)
            .unwrap()
            .replace("hi a", "yo a");
        std::fs::write(&older, same_size).unwrap();
        File::options()
            .write(true)
            .open(&older)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert_eq!(
            sessions.find(&followed, "a").unwrap().title.as_deref(),
            Some("hi a.jsonl")
        );
        std::fs::write(
            &older,
            r#"{"type":"user","cwd":"/elsewhere","message":{"content":"x"}}"#,
        )
        .unwrap();
        assert!(sessions.find(&followed, "a").is_err());

        assert_eq!(
            sessions.find(&followed, "../a").unwrap_err().to_string(),
            "invalid session id \"../a\""
        );
        assert_eq!(
            sessions.find(&followed, "zz").unwrap_err().to_string(),
            "no session zz in the followed projects"
        );
        sessions.delete(&followed, "b").unwrap();
        assert!(sessions.list(&followed).unwrap().is_empty());
        assert!(sessions.delete(&followed, "b").is_err());

        log(&folder, "a.jsonl", repo);
        sessions.delete(&followed, "a").unwrap();
        assert!(!folder.join("a.jsonl").exists());
        assert!(!folder.join("a").exists());
        // A folder that is only a symlink is left alone.
        log(&folder, "g.jsonl", repo);
        std::os::unix::fs::symlink(tmp.path(), folder.join("g")).unwrap();
        sessions.delete(&followed, "g").unwrap();
        assert!(folder.join("g").symlink_metadata().is_ok());

        // No Claude directory, or none yet: no sessions.
        assert!(Sessions::new(None).list(&followed).unwrap().is_empty());
        let missing = Sessions::new(Some(tmp.path().join("none")));
        assert!(missing.list(&followed).unwrap().is_empty());
        // A root that is a file cannot be listed.
        let file = Sessions::new(Some(tmp.path().join("projects/stray-file")));
        assert!(file.list(&followed).is_err());
    }
}
