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
            let drop = offered.len().saturating_sub(KEPT);
            offered.drain(..drop);
            self.write(&offered)?;
        }
        Ok(raised)
    }

    fn write(&self, offered: &[String]) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(offered)?;
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
    fn an_unwritable_file_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        // Its folder is a file.
        std::fs::write(tmp.path().join("data"), "").unwrap();
        let offers = Offers::new(tmp.path().join("data/offered-pulls.json"));
        let list = pulls(vec![pull(1, PullState::Merged, Some("/a"))]);
        assert!(offers.raised(&list, &worktrees()).is_err());
    }
}
