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
async fn a_sharing_file_ignored_is_sharings_error() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("sharing.json"), "{").unwrap();
    let state = shared(tmp.path(), &[(me(), "alice")]);
    let message = serde_json::to_value(state.sharing_message()).unwrap();
    let error = message["error"].as_str().unwrap();
    assert!(error.ends_with("): sharing is off"), "{error}");
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
    assert!(state.shared());
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
        shared: true,
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
    // A list that still has bob keeps his connection.
    let carol = person("carol", ShareRole::Viewer);
    let both = on(vec![person("bob", ShareRole::Viewer), carol]);
    state.share.store.set(both).unwrap();
    state.apply_sharing();
    let quiet = tokio::time::timeout(Duration::from_millis(200), client.0.next()).await;
    assert!(quiet.is_err(), "still connected: {quiet:?}");
    // The owner's own connections are told the service is shared.
    let (theirs, ours) = UnixStream::pair().unwrap();
    let (gone, _gone_rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(crate::daemon::app::connection(ours, state.clone(), gone));
    let (read, write) = theirs.into_split();
    let mut owner = (
        FramedRead::new(read, FrameCodec),
        FramedWrite::new(write, FrameCodec),
    );
    owner.1.send(Frame::control(0, &app())).await.unwrap();
    let next = tokio::time::timeout(Duration::from_secs(5), owner.0.next()).await;
    let welcome = next.unwrap().unwrap().unwrap().to_control().unwrap();
    assert!(
        matches!(
            welcome,
            Control::Welcome {
                shared: true,
                you: None,
                ..
            }
        ),
        "{welcome:?}"
    );
    drop(owner);
    // Removing bob from the list closes his connections at once.
    state.share.store.set(on(vec![])).unwrap();
    state.apply_sharing();
    closed(&mut client).await;
    wait_until(|| lock(&state.share.guests).is_empty()).await;
    // Turning sharing off removes the socket and its folder, its group kept or not.
    let off = Sharing {
        on: false,
        ..on(vec![])
    };
    state.share.store.set(off).unwrap();
    state.apply_sharing();
    assert!(!dir.exists());
    state.share.store.set(Sharing::default()).unwrap();
    state.apply_sharing();
    assert!(!dir.exists());
    assert!(!state.shared());
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
async fn a_failed_accept_is_tried_again_after_a_pause() {
    let tmp = tempfile::tempdir().unwrap();
    let state = shared(tmp.path(), &[(me(), "alice")]);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let accept = move || {
        let n = counted.fetch_add(1, Ordering::SeqCst);
        async move {
            if n == 0 {
                Err(std::io::Error::other("too many open files"))
            } else {
                std::future::pending().await
            }
        }
    };
    let task = tokio::spawn(accept_guests(accept, state));
    wait_until(|| calls.load(Ordering::SeqCst) == 2).await;
    task.abort();
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
