//! Offers to remove a worktree once its pull request merged or closed (17.27), as the Claude
//! Code desktop app archives a session then. The pull requests view's list says so; the app
//! shows an inbox item that opens the usual remove flow (6.6's health, 6.8's archive script):
//! nothing is ever removed here. Each pull request is offered once, kept in
//! `<data>/hive/offered-pulls.json` so a new service does not offer it again; one reopened is
//! forgotten, so it is offered again when it ends again.

use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use hive_protocol::{Control, PullState, PullSummary, Worktree};

use crate::git::read_limited;
use crate::wrapper::write_atomic;

/// Largest file read.
const FILE_LIMIT: u64 = 256 * 1024;
/// Most pull requests kept, the latest offered; older ones are dropped.
const KEPT: usize = 1000;

/// The pull requests already offered.
pub struct Offers {
    file: PathBuf,
    lock: Mutex<()>,
}

impl Offers {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            lock: Mutex::new(()),
        }
    }

    /// The offers a `pulls` reply raises, given the project's `worktrees` as they are now: for
    /// each worktree but the main one, with no open pull request of its own, its most recently
    /// updated merged or closed pull request not offered yet (all of them count as offered).
    /// A reply with an error, or for another message, raises none.
    pub fn raised(&self, reply: &Control, worktrees: &[Worktree]) -> io::Result<Vec<Control>> {
        let Control::Pulls {
            project,
            repo: Some(repo),
            mine,
            error: None,
            ..
        } = reply
        else {
            return Ok(Vec::new());
        };
        let key = |pull: &PullSummary| format!("{project}\n{}#{}", repo.name, pull.number);
        let ended =
            |pull: &PullSummary| matches!(pull.state, PullState::Merged | PullState::Closed);
        let _lock = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut offered = self.read();
        let before = offered.clone();
        // Reopened: its next end is offered again.
        let open: Vec<String> = mine.iter().filter(|p| !ended(p)).map(key).collect();
        offered.retain(|k| !open.contains(k));
        let mut raised = Vec::new();
        for w in worktrees.iter().filter(|w| !w.main) {
            let own: Vec<&PullSummary> = mine
                .iter()
                .filter(|p| p.worktree.as_ref() == Some(&w.id))
                .collect();
            if own.iter().any(|p| !ended(p)) {
                continue;
            }
            let new: Vec<&PullSummary> = own
                .into_iter()
                .filter(|p| !offered.contains(&key(p)))
                .collect();
            let Some(first) = new.first() else {
                continue;
            };
            raised.push(Control::PullEnded {
                project: project.clone(),
                worktree: w.id.clone(),
                number: first.number,
                title: first.title.clone(),
                state: first.state,
            });
            offered.extend(new.iter().map(|p| key(p)));
        }
        if offered != before {
            self.write(&fitting(offered))?;
        }
        Ok(raised)
    }

    fn write(&self, offered: &[String]) -> io::Result<()> {
        let json = serde_json::to_vec(offered)?;
        self.file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
        write_atomic(&self.file, &json, 0o600)
    }

    /// The pull requests in the file, the oldest offered first; an unreadable file has none.
    fn read(&self) -> Vec<String> {
        let read = std::fs::File::open(&self.file)
            .and_then(|mut file| read_limited(&mut file, FILE_LIMIT))
            .ok();
        read.and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
}

/// The latest `offered`, at most [`KEPT`] and as many as the file holds within [`FILE_LIMIT`]
/// once written (`[`, the entries as JSON strings with a comma between them, `]`): what is
/// written is always read back.
fn fitting(mut offered: Vec<String>) -> Vec<String> {
    let drop = offered.len().saturating_sub(KEPT);
    offered.drain(..drop);
    let size = |key: &String| serde_json::to_string(key).unwrap_or_default().len() + 1;
    let mut total = 1 + offered.iter().map(size).sum::<usize>();
    let mut drop = 0;
    // Bound by the list, whatever the sizes say.
    while let Some(oldest) = offered.get(drop).filter(|_| total > FILE_LIMIT as usize) {
        total -= size(oldest);
        drop += 1;
    }
    offered.drain(..drop);
    offered
}

#[cfg(test)]
mod tests {
    use hive_protocol::PullRepo;

    use super::*;

    fn worktree(id: &str, main: bool) -> Worktree {
        Worktree {
            id: id.into(),
            name: id.into(),
            path: id.into(),
            branch: Some(id.into()),
            main,
            claude: !main,
            status: None,
        }
    }

    fn pull(number: u64, state: PullState, worktree: Option<&str>) -> PullSummary {
        PullSummary {
            number,
            title: format!("T{number}"),
            url: String::new(),
            state,
            branch: String::new(),
            base: "main".into(),
            author: "me".into(),
            review: None,
            checks: None,
            updated_at: String::new(),
            worktree: worktree.map(Into::into),
        }
    }

    fn pulls(mine: Vec<PullSummary>) -> Control {
        Control::Pulls {
            project: "/r".into(),
            repo: Some(PullRepo {
                name: "o/r".into(),
                url: String::new(),
                merge_methods: Vec::new(),
                default_merge: None,
            }),
            mine,
            review: Vec::new(),
            fetched_ms: 1,
            error: None,
        }
    }

    fn ended(worktree: &str, number: u64, state: PullState) -> Control {
        Control::PullEnded {
            project: "/r".into(),
            worktree: worktree.into(),
            number,
            title: format!("T{number}"),
            state,
        }
    }

    fn worktrees() -> Vec<Worktree> {
        vec![
            worktree("/r", true),
            worktree("/a", false),
            worktree("/b", false),
        ]
    }

    #[test]
    fn a_merged_or_closed_pull_request_is_offered_once() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("hive/offered-pulls.json");
        let offers = Offers::new(file.clone());
        let list = pulls(vec![
            pull(1, PullState::Merged, Some("/a")),
            pull(2, PullState::Closed, Some("/b")),
            // Not a worktree's, or the main worktree's: nothing to remove.
            pull(3, PullState::Merged, None),
            pull(4, PullState::Merged, Some("/r")),
            // A worktree that is gone.
            pull(5, PullState::Closed, Some("/gone")),
        ]);
        let raised = offers.raised(&list, &worktrees()).unwrap();
        assert_eq!(
            raised,
            [
                ended("/a", 1, PullState::Merged),
                ended("/b", 2, PullState::Closed)
            ]
        );
        let kept: Vec<String> = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(kept, ["/r\no/r#1", "/r\no/r#2"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // The next list, and a new service reading the file: nothing again.
        assert_eq!(offers.raised(&list, &worktrees()).unwrap(), []);
        let restarted = Offers::new(file);
        assert_eq!(restarted.raised(&list, &worktrees()).unwrap(), []);
    }

    #[test]
    fn a_worktree_with_an_open_pull_request_is_not_offered() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("offered-pulls.json");
        let offers = Offers::new(file.clone());
        let list = pulls(vec![
            pull(7, PullState::Draft, Some("/a")),
            pull(6, PullState::Merged, Some("/a")),
            pull(8, PullState::Open, None),
        ]);
        assert_eq!(offers.raised(&list, &worktrees()).unwrap(), []);
        // Nothing changed, nothing written.
        assert!(!file.exists());
        // Its open one closes: one offer, the newest, and both count as offered.
        let list = pulls(vec![
            pull(7, PullState::Closed, Some("/a")),
            pull(6, PullState::Merged, Some("/a")),
        ]);
        let raised = offers.raised(&list, &worktrees()).unwrap();
        assert_eq!(raised, [ended("/a", 7, PullState::Closed)]);
        assert_eq!(offers.raised(&list, &worktrees()).unwrap(), []);
    }

    #[test]
    fn a_reopened_pull_request_is_offered_again_when_it_ends_again() {
        let tmp = tempfile::tempdir().unwrap();
        let offers = Offers::new(tmp.path().join("offered-pulls.json"));
        let closed = pulls(vec![pull(1, PullState::Closed, Some("/a"))]);
        let reopened = pulls(vec![pull(1, PullState::Open, Some("/a"))]);
        let merged = pulls(vec![pull(1, PullState::Merged, Some("/a"))]);
        let raised = offers.raised(&closed, &worktrees()).unwrap();
        assert_eq!(raised, [ended("/a", 1, PullState::Closed)]);
        assert_eq!(offers.raised(&reopened, &worktrees()).unwrap(), []);
        let raised = offers.raised(&merged, &worktrees()).unwrap();
        assert_eq!(raised, [ended("/a", 1, PullState::Merged)]);
        assert_eq!(offers.raised(&merged, &worktrees()).unwrap(), []);
    }

    #[test]
    fn a_failed_list_or_another_message_raises_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let offers = Offers::new(tmp.path().join("offered-pulls.json"));
        let list = || pulls(vec![pull(1, PullState::Merged, Some("/a"))]);
        let (mut failed, mut no_repo) = (list(), list());
        if let Control::Pulls { error, .. } = &mut failed {
            *error = Some("rate limit".into());
        }
        if let Control::Pulls { repo, .. } = &mut no_repo {
            *repo = None;
        }
        let other = Control::Notice {
            message: "m".into(),
        };
        for reply in [failed, no_repo, other] {
            assert_eq!(offers.raised(&reply, &worktrees()).unwrap(), []);
        }
    }

    #[test]
    fn only_the_latest_offers_are_kept_and_a_bad_file_counts_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("offered-pulls.json");
        let old: Vec<String> = (0..KEPT as u64).map(|n| format!("/x\no/x#{n}")).collect();
        std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
        let offers = Offers::new(file.clone());
        let list = pulls(vec![pull(1, PullState::Merged, Some("/a"))]);
        assert_eq!(offers.raised(&list, &worktrees()).unwrap().len(), 1);
        let kept: Vec<String> = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(kept.len(), KEPT);
        assert_eq!(kept.first().map(String::as_str), Some("/x\no/x#1"));
        assert_eq!(kept.last().map(String::as_str), Some("/r\no/r#1"));

        std::fs::write(&file, "not json").unwrap();
        assert_eq!(offers.raised(&list, &worktrees()).unwrap().len(), 1);
    }

    #[test]
    fn long_keys_keep_the_file_readable_with_the_latest_offers() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("offered-pulls.json");
        let offers = Offers::new(file.clone());
        // A long project path: 1000 keys would be far over the read limit.
        let project = format!("/{}", "p".repeat(3000));
        let worktrees: Vec<Worktree> = (1..=200)
            .map(|n| worktree(&format!("/w{n}"), false))
            .collect();
        let mine = (1..=200)
            .map(|n| pull(n, PullState::Merged, Some(&format!("/w{n}"))))
            .collect();
        let mut list = pulls(mine);
        if let Control::Pulls { project: p, .. } = &mut list {
            p.clone_from(&project);
        }
        assert_eq!(offers.raised(&list, &worktrees).unwrap().len(), 200);
        let written = std::fs::read(&file).unwrap();
        assert!(written.len() <= FILE_LIMIT as usize, "{}", written.len());
        // Read back: the latest ones, as many as fit, none offered again.
        let kept = offers.read();
        let size = 3000 + "/\no/r#100".len() + 3;
        assert_eq!(kept.len(), (FILE_LIMIT as usize - 1) / size);
        assert_eq!(kept.last(), Some(&format!("{project}\no/r#200")));
        let latest = kept.len() as u64;
        let mine = (201 - latest..=200)
            .map(|n| pull(n, PullState::Merged, Some(&format!("/w{n}"))))
            .collect();
        let mut list = pulls(mine);
        if let Control::Pulls { project: p, .. } = &mut list {
            p.clone_from(&project);
        }
        assert_eq!(offers.raised(&list, &worktrees).unwrap(), []);
    }

    #[test]
    fn the_kept_list_fills_the_file_to_its_last_byte() {
        let short = || "k".repeat(100);
        // `[`, three of 102 bytes and the long one, a comma after each but the last, `]`.
        let long = |extra: usize| "l".repeat(FILE_LIMIT as usize - 1 - 3 * 103 - 3 + extra);
        let full = vec![short(), short(), short(), long(0)];
        let kept = fitting(full.clone());
        assert_eq!(kept, full);
        assert_eq!(
            serde_json::to_vec(&kept).unwrap().len(),
            FILE_LIMIT as usize
        );
        // One byte more: the oldest goes.
        let over = vec![short(), short(), short(), long(1)];
        assert_eq!(fitting(over.clone()), over[1..]);
        // A single key too long for the file: nothing is kept.
        assert_eq!(fitting(vec![long(400)]), Vec::<String>::new());
    }

    #[test]
    fn an_unwritable_file_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        // Its folder is a file.
        std::fs::write(tmp.path().join("data"), "").unwrap();
        let offers = Offers::new(tmp.path().join("data/offered-pulls.json"));
        let list = pulls(vec![pull(1, PullState::Merged, Some("/a"))]);
        assert!(offers.raised(&list, &worktrees()).is_err());
    }
}
