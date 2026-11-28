//! The service's shared socket (29.12, `docs/spike/multiplayer.md` §3, §8): opened, changed and
//! closed to match the Sharing settings, and the teammates' connections on it.
//!
//! A connection there is a teammate's only when the kernel's peer uid names a login on the
//! Sharing list ([`crate::sharing::Store::guest`]); anyone else is refused, and so is
//! `role: hook` (hook events come only through the owner's own `0600` socket). A teammate's app
//! is welcomed with who it is (`welcome.you`) and then held: serving it (sessions, the role
//! table, the shared spaces) is 29.13's, so until then every frame it sends is read and dropped.
//! Removing a person from the list, or turning sharing off, closes their connections at once.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Frame, FrameCodec, PROTOCOL_VERSION, Role, Sharing};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::codec::{FramedRead, FramedWrite};

use super::State;
use crate::VERSION;
use crate::sharing::{self, Id, Store, Users};

/// Why a service cannot be shared: only a remote Linux one can (spike §3, Stage 26 Q5).
pub const UNAVAILABLE: &str = "Only the Hive of a remote Linux host can be shared";
/// The most teammates' connections at once (Q10).
pub const GUESTS: usize = 8;
/// The most connections of one person at once (two laptops, Q10).
pub const PER_PERSON: usize = 2;
/// How long a connection on the shared socket has to send its `hello`.
const HELLO_WAIT: Duration = Duration::from_secs(10);

/// The service's sharing: its settings, who is who, the open shared socket and the teammates
/// connected through it.
pub(super) struct Share {
    store: Store,
    users: Arc<dyn Users>,
    /// Why this service cannot be shared; `None` when it can.
    unavailable: Option<&'static str>,
    /// The shared folders' parent ([`sharing::root`]).
    root: PathBuf,
    open: Mutex<Option<Open>>,
    /// Why sharing is on but its socket is not open.
    error: Mutex<Option<String>>,
    guests: Mutex<Vec<Seat>>,
    next: AtomicU64,
}

/// The open shared socket: the group it was opened for, where, and its accepting task.
struct Open {
    group: String,
    dir: PathBuf,
    id: Id,
    task: JoinHandle<()>,
}

/// A teammate's connection, closed when woken.
struct Seat {
    id: u64,
    login: String,
    close: Arc<Notify>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Share {
    /// `can`: whether this service may be shared (a remote Linux one).
    pub fn new(store: Store, users: Arc<dyn Users>, root: PathBuf, can: bool) -> Self {
        Self {
            store,
            users,
            unavailable: (!can).then_some(UNAVAILABLE),
            root,
            open: Mutex::default(),
            error: Mutex::default(),
            guests: Mutex::default(),
            next: AtomicU64::new(0),
        }
    }

    /// The owner's login name: this process's user's.
    fn owner(&self) -> String {
        let me = nix::unistd::getuid().as_raw();
        self.users.name(me).unwrap_or_else(|| format!("uid {me}"))
    }

    fn group_names(&self) -> Vec<String> {
        self.users
            .groups()
            .into_iter()
            .map(|(_, name)| name)
            .collect()
    }
}

impl State {
    /// Settings → Sharing as it is now.
    pub(super) fn sharing_message(&self) -> Control {
        let share = &self.share;
        Control::Sharing {
            sharing: share.store.get(),
            owner: share.owner(),
            groups: share.group_names(),
            spaces: self.projects.space_choices(),
            unavailable: share.unavailable.map(str::to_owned),
            error: lock(&share.error).clone(),
        }
    }

    /// Checks and saves `sharing`, then opens, changes or closes the shared socket to match:
    /// the reply for the app.
    pub(super) fn set_sharing(self: &Arc<Self>, sharing: Sharing) -> Control {
        let failed = |message: String| Control::SharingFailed { message };
        let share = &self.share;
        if let Some(why) = share.unavailable {
            return failed(why.to_owned());
        }
        let (owner, groups, spaces) = (
            share.owner(),
            share.group_names(),
            self.projects.space_choices(),
        );
        let context = sharing::Context {
            owner: &owner,
            groups: &groups,
            spaces: &spaces,
            users: share.users.as_ref(),
        };
        if let Err(message) = sharing::check(&sharing, &context) {
            return failed(message);
        }
        if let Err(err) = share.store.set(sharing) {
            return failed(format!("Cannot save the Sharing settings: {err}"));
        }
        self.apply_sharing();
        self.sharing_message()
    }

    /// Opens the shared socket when sharing is on (again, for another group), closes it when it
    /// is off, and closes the connections of every teammate no longer on the list.
    pub(super) fn apply_sharing(self: &Arc<Self>) {
        let share = &self.share;
        let sharing = share.store.get();
        let wanted = (share.unavailable.is_none() && sharing.on).then_some(&sharing.group);
        {
            let mut open = lock(&share.open);
            if open.as_ref().map(|open| &open.group) != wanted {
                if let Some(old) = open.take() {
                    old.task.abort();
                    sharing::close(&old.dir, old.id);
                }
                *lock(&share.error) = None;
                if let Some(group) = wanted {
                    match self.open_shared(group) {
                        Ok(opened) => *open = Some(opened),
                        Err(err) => {
                            eprintln!("hive: cannot share this Hive: {err}");
                            *lock(&share.error) = Some(err);
                        }
                    }
                }
            }
        }
        let listed =
            |login: &str| wanted.is_some() && sharing.people.iter().any(|p| p.login == login);
        for seat in lock(&share.guests)
            .iter()
            .filter(|seat| !listed(&seat.login))
        {
            seat.close.notify_one();
        }
    }

    /// The shared socket for `group`, accepting teammates.
    fn open_shared(self: &Arc<Self>, group: &str) -> Result<Open, String> {
        let share = &self.share;
        let found = share
            .users
            .groups()
            .into_iter()
            .find(|(_, name)| name == group);
        let (gid, _) = found.ok_or_else(|| format!("you are no longer in the group {group:?}"))?;
        let uid = nix::unistd::getuid().as_raw();
        let dir = sharing::dir(&share.root, uid);
        let (listener, id) = sharing::open(&dir, uid, gid).map_err(|err| err.to_string())?;
        let task = tokio::spawn(accept_guests(listener, self.clone()));
        Ok(Open {
            group: group.to_owned(),
            dir,
            id,
            task,
        })
    }

    /// The service ends: the shared socket and folder go, and every teammate's connection.
    pub(super) fn end_sharing(&self) {
        if let Some(open) = lock(&self.share.open).take() {
            open.task.abort();
            sharing::close(&open.dir, open.id);
        }
        for seat in lock(&self.share.guests).iter() {
            seat.close.notify_one();
        }
    }
}

/// Serves each connection on the shared socket, until the socket closes or fails.
async fn accept_guests(listener: UnixListener, state: Arc<State>) {
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(guest(stream, state.clone()));
    }
}

/// A connection on the shared socket: refused unless the kernel names a person on the Sharing
/// list and its `hello` is an app's of this version; else welcomed and held (29.13 serves it).
async fn guest(stream: UnixStream, state: Arc<State>) {
    let uid = stream.peer_cred().map(|cred| cred.uid()).ok();
    let (read, write) = stream.into_split();
    let mut reader = FramedRead::new(read, FrameCodec);
    let mut writer = FramedWrite::new(write, FrameCodec);
    let hello = tokio::time::timeout(HELLO_WAIT, reader.next()).await;
    let hello = hello
        .ok()
        .flatten()
        .and_then(Result::ok)
        .map(|f| f.to_control());
    let share = &state.share;
    let name = uid.and_then(|uid| share.users.name(uid));
    let who = name.clone().unwrap_or_else(|| format!("uid {uid:?}"));
    let owner = share.owner();
    // Taken with the seats held, so a person removed meanwhile is either kicked or never seated.
    let seated = {
        let mut guests = lock(&share.guests);
        let guest = name.as_deref().and_then(|name| share.store.guest(name));
        match (hello, guest) {
            (Some(Ok(Control::Hello { .. })), None) => {
                Err(format!("{who} is not on {owner}'s Sharing list"))
            }
            (
                Some(Ok(Control::Hello {
                    role: Role::Hook, ..
                })),
                Some(_),
            ) => Err("hook events come only through the owner's own socket".to_owned()),
            (
                Some(Ok(Control::Hello {
                    protocol, version, ..
                })),
                Some(_),
            ) if protocol != PROTOCOL_VERSION || version != VERSION => {
                // Restarting the service is the owner's (Q5): no terminal count for a teammate.
                let mismatch = Control::VersionMismatch {
                    protocol: PROTOCOL_VERSION,
                    version: VERSION.to_owned(),
                    terminals: None,
                };
                Ok(Err(mismatch))
            }
            (Some(Ok(Control::Hello { .. })), Some(_)) if guests.len() >= GUESTS => Err(format!(
                "{owner}'s Hive has {GUESTS} teammates connected already"
            )),
            (Some(Ok(Control::Hello { .. })), Some(guest))
                if guests.iter().filter(|s| s.login == guest.name).count() >= PER_PERSON =>
            {
                Err(format!(
                    "you are connected to {owner}'s Hive from {PER_PERSON} apps already"
                ))
            }
            (Some(Ok(Control::Hello { .. })), Some(guest)) => {
                let seat = Seat {
                    id: share.next.fetch_add(1, Ordering::Relaxed),
                    login: guest.name.clone(),
                    close: Arc::default(),
                };
                let held = (seat.id, seat.close.clone(), guest);
                guests.push(seat);
                Ok(Ok(held))
            }
            _ => Err("expected a hello message".to_owned()),
        }
    };
    let (id, close, you) = match seated {
        Ok(Ok(held)) => held,
        Ok(Err(mismatch)) => {
            let _ = writer.send(Frame::control(0, &mismatch)).await;
            return;
        }
        Err(message) => {
            eprintln!("hive: refused a connection on the shared socket ({who}): {message}");
            let _ = writer
                .send(Frame::control(0, &Control::Error { message }))
                .await;
            return;
        }
    };
    let welcome = Control::Welcome {
        version: VERSION.to_owned(),
        distro: None,
        warning: None,
        you: Some(you),
    };
    if writer.send(Frame::control(0, &welcome)).await.is_ok() {
        // Held until it closes or is closed: its frames are 29.13's to serve.
        loop {
            tokio::select! {
                frame = reader.next() => if !matches!(frame, Some(Ok(_))) { break },
                () = close.notified() => break,
            }
        }
    }
    lock(&share.guests).retain(|seat| seat.id != id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_protocol::{Guest, Person, ShareRole};
    use std::os::unix::fs::MetadataExt;

    fn me() -> u32 {
        nix::unistd::getuid().as_raw()
    }

    fn gid() -> u32 {
        nix::unistd::getegid().as_raw()
    }

    /// A shareable service in `dir` whose users are `users` (uid, name); its group is this
    /// process's, named `team`.
    fn shared(dir: &std::path::Path, users: &[(u32, &str)]) -> Arc<State> {
        let fake = sharing::Fake {
            users: users.iter().map(|(id, n)| (*id, (*n).to_owned())).collect(),
            groups: vec![(gid(), "team".into()), (4_000_000_000, "gone".into())],
        };
        let share = Share::new(
            Store::load(dir.join("sharing.json")),
            Arc::new(fake),
            dir.join("tmp"),
            true,
        );
        std::fs::create_dir(dir.join("tmp")).unwrap();
        let state = super::super::test_state(dir);
        let mut state = Arc::into_inner(state).unwrap();
        state.share = share;
        Arc::new(state)
    }

    fn person(login: &str, role: ShareRole) -> Person {
        Person {
            login: login.into(),
            display: String::new(),
            git_name: String::new(),
            git_email: String::new(),
            role,
        }
    }

    fn on(people: Vec<Person>) -> Sharing {
        Sharing {
            on: true,
            group: "team".into(),
            spaces: vec![],
            people,
        }
    }

    fn socket(state: &State) -> PathBuf {
        sharing::dir(&state.share.root, me()).join(sharing::SOCKET)
    }

    type Client = (
        FramedRead<tokio::net::unix::OwnedReadHalf, FrameCodec>,
        FramedWrite<tokio::net::unix::OwnedWriteHalf, FrameCodec>,
    );

    /// Connects to the shared socket and says `hello`: the first answer.
    async fn hello(state: &State, hello: Control) -> (Client, Control) {
        let stream = UnixStream::connect(socket(state)).await.unwrap();
        let (read, write) = stream.into_split();
        let (mut reader, mut writer) = (
            FramedRead::new(read, FrameCodec),
            FramedWrite::new(write, FrameCodec),
        );
        writer.send(Frame::control(0, &hello)).await.unwrap();
        let next = tokio::time::timeout(Duration::from_secs(5), reader.next()).await;
        let answer = next.unwrap().unwrap().unwrap().to_control().unwrap();
        ((reader, writer), answer)
    }

    /// Waits until `ready`, at most 5 s.
    async fn wait_until(ready: impl Fn() -> bool) {
        let wait = async {
            while !ready() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .unwrap();
    }

    fn app() -> Control {
        Control::hello(Role::App, VERSION)
    }

    fn refusal(message: &str) -> Control {
        Control::Error {
            message: message.into(),
        }
    }

    /// The connection closes: its next read ends.
    async fn closed(client: &mut Client) {
        let next = tokio::time::timeout(Duration::from_secs(5), client.0.next()).await;
        assert!(next.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_service_that_cannot_be_shared_says_why_and_saves_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let state = super::super::test_state(tmp.path());
        let Control::Sharing {
            unavailable,
            sharing,
            error,
            ..
        } = state.sharing_message()
        else {
            panic!()
        };
        assert_eq!(unavailable.as_deref(), Some(UNAVAILABLE));
        assert_eq!((sharing, error), (Sharing::default(), None));
        let reply = state.set_sharing(on(vec![]));
        assert_eq!(
            reply,
            Control::SharingFailed {
                message: UNAVAILABLE.into()
            }
        );
        assert!(!tmp.path().join("sharing.json").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sharing_opens_the_group_socket_and_a_teammate_on_the_list_is_welcomed() {
        let tmp = tempfile::tempdir().unwrap();
        // This process's user stands for bob, a teammate; the owner is too (same uid).
        let state = shared(tmp.path(), &[(me(), "bob"), (3000, "carol")]);
        // A check that fails saves nothing.
        let bad = Sharing {
            group: "wheel".into(),
            ..on(vec![])
        };
        assert_eq!(
            state.set_sharing(bad),
            Control::SharingFailed {
                message: "You are not in the group \"wheel\"".into()
            }
        );
        // The list is saved without the check's owner rule: bob is the test's own user.
        let mut bob = person("bob", ShareRole::Collaborator);
        bob.display = "Bob".into();
        state.share.store.set(on(vec![bob])).unwrap();
        state.apply_sharing();
        let Control::Sharing {
            owner,
            groups,
            unavailable,
            error,
            ..
        } = state.sharing_message()
        else {
            panic!()
        };
        assert_eq!((owner.as_str(), unavailable, error), ("bob", None, None));
        assert_eq!(groups, ["team", "gone"]);
        let dir = sharing::dir(&state.share.root, me());
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o710);
        // Welcomed as who the kernel says, never as anything the app says.
        let (mut client, answer) = hello(&state, app()).await;
        let you = Guest {
            name: "bob".into(),
            display: "Bob".into(),
            role: ShareRole::Collaborator,
        };
        let welcome = Control::Welcome {
            version: VERSION.into(),
            distro: None,
            warning: None,
            you: Some(you),
        };
        assert_eq!(answer, welcome);
        // Its requests are not served yet (29.13): read and dropped, the connection held.
        client
            .1
            .send(Frame::control(0, &Control::ListProjects))
            .await
            .unwrap();
        client
            .1
            .send(Frame::control(0, &Control::GetSharing))
            .await
            .unwrap();
        let quiet = tokio::time::timeout(Duration::from_millis(200), client.0.next()).await;
        assert!(quiet.is_err(), "nothing is answered: {quiet:?}");
        // A second app of bob's is seated too; a third is refused.
        let (second, answer) = hello(&state, app()).await;
        assert!(matches!(answer, Control::Welcome { .. }));
        let (_third, answer) = hello(&state, app()).await;
        assert_eq!(
            answer,
            refusal("you are connected to bob's Hive from 2 apps already")
        );
        // A seat is given back when its app goes.
        drop(second);
        wait_until(|| lock(&state.share.guests).len() == 1).await;
        let (_again, answer) = hello(&state, app()).await;
        assert!(matches!(answer, Control::Welcome { .. }));
        // Removing bob from the list closes his connections at once.
        state.share.store.set(on(vec![])).unwrap();
        state.apply_sharing();
        closed(&mut client).await;
        wait_until(|| lock(&state.share.guests).is_empty()).await;
        // Turning sharing off removes the socket and its folder.
        state.share.store.set(Sharing::default()).unwrap();
        state.apply_sharing();
        assert!(!dir.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn anyone_not_on_the_list_a_hook_or_another_version_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let state = shared(tmp.path(), &[(me(), "alice")]);
        let reply = state.set_sharing(on(vec![]));
        assert!(matches!(reply, Control::Sharing { .. }), "{reply:?}");
        // The owner's own user is not on the list (it cannot be): refused like a stranger.
        let (_c, answer) = hello(&state, app()).await;
        assert_eq!(answer, refusal("alice is not on alice's Sharing list"));
        // A teammate's hook, or an app of another version.
        state
            .share
            .store
            .set(on(vec![person("alice", ShareRole::Viewer)]))
            .unwrap();
        let (_c, answer) = hello(&state, Control::hello(Role::Hook, VERSION)).await;
        assert_eq!(
            answer,
            refusal("hook events come only through the owner's own socket")
        );
        let (_c, answer) = hello(&state, Control::hello(Role::App, "0.0.1")).await;
        let mismatch = Control::VersionMismatch {
            protocol: PROTOCOL_VERSION,
            version: VERSION.into(),
            terminals: None,
        };
        assert_eq!(answer, mismatch);
        // Something else than a hello.
        let (_c, answer) = hello(&state, Control::ListProjects).await;
        assert_eq!(answer, refusal("expected a hello message"));
        // No name for the uid.
        let other = tmp.path().join("n");
        std::fs::create_dir(&other).unwrap();
        let nameless = shared(&other, &[]);
        nameless.share.store.set(on(vec![])).unwrap();
        nameless.apply_sharing();
        let (_c, answer) = hello(&nameless, app()).await;
        let who = format!("uid {:?} is not on uid {}'s Sharing list", Some(me()), me());
        assert_eq!(answer, refusal(&who));
        nameless.end_sharing();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn at_most_eight_teammates_are_connected_and_the_end_closes_them_all() {
        let tmp = tempfile::tempdir().unwrap();
        let state = shared(tmp.path(), &[(me(), "bob")]);
        state
            .share
            .store
            .set(on(vec![person("bob", ShareRole::Viewer)]))
            .unwrap();
        state.apply_sharing();
        // Seats of others, as if connected.
        for n in 0..GUESTS as u64 {
            lock(&state.share.guests).push(Seat {
                id: 100 + n,
                login: format!("p{n}"),
                close: Arc::default(),
            });
        }
        let (_c, answer) = hello(&state, app()).await;
        assert_eq!(
            answer,
            refusal("bob's Hive has 8 teammates connected already")
        );
        lock(&state.share.guests).clear();
        let (mut client, answer) = hello(&state, app()).await;
        assert!(matches!(answer, Control::Welcome { .. }));
        state.end_sharing();
        closed(&mut client).await;
        assert!(!socket(&state).exists());
        // Ending twice does nothing more.
        state.end_sharing();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_planted_folder_or_a_group_left_is_sharings_error_and_it_is_tried_again() {
        let tmp = tempfile::tempdir().unwrap();
        let state = shared(tmp.path(), &[(me(), "alice")]);
        // Planted: a link in the folder's place.
        let dir = sharing::dir(&state.share.root, me());
        std::os::unix::fs::symlink(tmp.path(), &dir).unwrap();
        state.share.store.set(on(vec![])).unwrap();
        state.apply_sharing();
        let Control::Sharing { error, .. } = state.sharing_message() else {
            panic!()
        };
        assert!(error.unwrap().contains("someone else made it first"));
        // Once it is gone, the next change opens the socket.
        std::fs::remove_file(&dir).unwrap();
        state.apply_sharing();
        let Control::Sharing { error, .. } = state.sharing_message() else {
            panic!()
        };
        assert_eq!(error, None);
        assert!(socket(&state).exists());
        // Another group: the socket is opened again for it; a group the owner left fails.
        let gone = Sharing {
            group: "gone".into(),
            ..on(vec![])
        };
        state.share.store.set(gone).unwrap();
        state.apply_sharing();
        let Control::Sharing { error, .. } = state.sharing_message() else {
            panic!()
        };
        // `gone`, a group this user is not in, cannot be given to the folder.
        assert!(error.is_some());
        let left = Sharing {
            group: "left".into(),
            ..on(vec![])
        };
        state.share.store.set(left).unwrap();
        state.apply_sharing();
        let Control::Sharing { error, .. } = state.sharing_message() else {
            panic!()
        };
        assert_eq!(
            error.as_deref(),
            Some("you are no longer in the group \"left\"")
        );
        // A failed save says why.
        std::fs::remove_file(tmp.path().join("sharing.json")).unwrap();
        std::fs::create_dir(tmp.path().join("sharing.json")).unwrap();
        let reply = state.set_sharing(on(vec![]));
        let Control::SharingFailed { message } = reply else {
            panic!("{reply:?}")
        };
        assert!(
            message.starts_with("Cannot save the Sharing settings: "),
            "{message}"
        );
        state.end_sharing();
    }

    #[tokio::test]
    async fn the_spaces_are_offered_by_id_and_name() {
        let tmp = tempfile::tempdir().unwrap();
        let state = shared(tmp.path(), &[(me(), "alice")]);
        let choices = state.projects.space_choices();
        let Control::Spaces { spaces, .. } = state.projects.spaces_message() else {
            panic!()
        };
        let expected: Vec<(String, String)> = spaces.into_iter().map(|s| (s.id, s.name)).collect();
        let got: Vec<(String, String)> = choices.into_iter().map(|s| (s.id, s.name)).collect();
        assert_eq!(got, expected);
        assert!(!got.is_empty());
    }
}
