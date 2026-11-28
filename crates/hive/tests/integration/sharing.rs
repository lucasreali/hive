//! Sharing a remote Linux service (29.12): `hive bridge --join <owner>` reaches only its owner's
//! shared socket, and the service opens one for its group, refuses anyone not on its Sharing
//! list, and removes it when sharing ends. Every shared folder is under the test's own
//! `HIVE_SHARED_ROOT`, never the real `/tmp`; a second Unix user is 29.19's (here the test's own
//! user stands for every side).

#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::process::Stdio;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Frame, FrameCodec, Role, Sharing};
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::common::{Env, TIMEOUT};

fn me() -> u32 {
    nix::unistd::getuid().as_raw()
}

/// This user's login name, the owner every test joins.
fn login() -> String {
    nix::unistd::User::from_uid(nix::unistd::getuid())
        .unwrap()
        .unwrap()
        .name
}

/// The shared folder of this user in `env`.
fn shared_dir(env: &Env) -> std::path::PathBuf {
    env.path("tmp").join(format!("hive-shared-{}", me()))
}

/// Runs `hive bridge <args>` with stdin closed at once: its exit and its stderr.
async fn failed_bridge(env: &Env, args: &[&str]) -> (bool, String) {
    let mut cmd = tokio::process::Command::from(env.hive());
    let out = cmd
        .arg("bridge")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let out = tokio::time::timeout(TIMEOUT, out).await.unwrap().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[tokio::test]
async fn a_joining_bridge_reaches_only_its_owners_shared_socket() {
    let env = Env::new();
    let me = login();
    // Not running: no folder.
    assert_eq!(
        failed_bridge(&env, &["--join", &me]).await,
        (false, "hive-join-down\n".into())
    );
    // A folder others can write to.
    let dir = shared_dir(&env);
    std::fs::DirBuilder::new().mode(0o777).create(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o717)).unwrap();
    let why = format!("hive-join: others can write to {}\n", dir.display());
    assert_eq!(failed_bridge(&env, &["--join", &me]).await, (false, why));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o710)).unwrap();
    // No such user, a name that is not a login, and `--replace` with it.
    let (ok, err) = failed_bridge(&env, &["--join", "no-such-user-hive"]).await;
    assert_eq!(
        (ok, err.as_str()),
        (
            false,
            "hive-join: there is no user no-such-user-hive on this host\n"
        )
    );
    let (ok, err) = failed_bridge(&env, &["--join", "A;b"]).await;
    assert!(!ok && err.starts_with("hive-join: The login name"), "{err}");
    let (ok, err) = failed_bridge(&env, &["--join", &me, "--replace"]).await;
    assert!(!ok && err.contains("cannot be used with"), "{err}");
    // The owner's socket: frames pass through both ways, and no service is started.
    let listener = tokio::net::UnixListener::bind(dir.join("hive.sock")).unwrap();
    let mut cmd = tokio::process::Command::from(env.hive());
    let mut child = cmd
        .args(["bridge", "--join", &me])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut to_bridge = FramedWrite::new(child.stdin.take().unwrap(), FrameCodec);
    let mut from_bridge = FramedRead::new(child.stdout.take().unwrap(), FrameCodec);
    let hello = Control::hello(Role::App, hive::VERSION);
    to_bridge.send(Frame::control(0, &hello)).await.unwrap();
    let (stream, _) = tokio::time::timeout(TIMEOUT, listener.accept())
        .await
        .unwrap()
        .unwrap();
    let (mut from_app, mut to_app) = {
        let (read, write) = stream.into_split();
        (
            FramedRead::new(read, FrameCodec),
            FramedWrite::new(write, FrameCodec),
        )
    };
    let got = from_app
        .next()
        .await
        .unwrap()
        .unwrap()
        .to_control()
        .unwrap();
    assert_eq!(got, hello);
    let answer = Control::Error {
        message: "x".into(),
    };
    to_app.send(Frame::control(0, &answer)).await.unwrap();
    let got = from_bridge
        .next()
        .await
        .unwrap()
        .unwrap()
        .to_control()
        .unwrap();
    assert_eq!(got, answer);
    assert!(
        !env.socket().exists(),
        "a joining bridge never starts a service"
    );
    // The owner's side closing ends the bridge.
    drop((from_app, to_app));
    let status = tokio::time::timeout(TIMEOUT, child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
}

#[tokio::test]
async fn a_service_that_is_not_a_remote_linux_one_is_never_shared() {
    // A local service; and off Linux, a remote one too (a remote is Linux, Stage 26 Q5).
    let mut kinds = vec![&["daemon"][..]];
    if !cfg!(target_os = "linux") {
        kinds.push(&["daemon", "--remote"]);
    }
    for args in kinds {
        let env = Env::new();
        let mut cmd = env.hive();
        cmd.args(args).stdin(Stdio::null());
        let daemon = crate::common::Daemon(cmd.spawn().unwrap());
        crate::common::wait_until(|| std::os::unix::net::UnixStream::connect(env.socket()).is_ok());
        let mut app = env.app().await;
        app.send(0, Control::GetSharing).await;
        let (_, reply) = app.control().await;
        let reply = serde_json::to_value(reply).unwrap();
        assert!(reply["unavailable"].is_string(), "{args:?}: {reply}");
        assert_eq!(
            reply["sharing"],
            serde_json::to_value(Sharing::default()).unwrap()
        );
        let on = Sharing {
            on: true,
            group: "x".into(),
            ..Sharing::default()
        };
        app.send(0, Control::SetSharing { sharing: on }).await;
        let (_, reply) = app.control().await;
        assert!(matches!(reply, Control::SharingFailed { .. }), "{reply:?}");
        assert!(!shared_dir(&env).exists());
        drop(app);
        crate::common::stop(daemon);
    }
}

/// The name of this process's group.
#[cfg(target_os = "linux")]
fn group() -> String {
    nix::unistd::Group::from_gid(nix::unistd::getegid())
        .unwrap()
        .unwrap()
        .name
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_remote_linux_service_shares_a_group_socket_and_refuses_anyone_not_listed() {
    let env = Env::new();
    let mut cmd = env.hive();
    cmd.args(["daemon", "--remote"]).stdin(Stdio::null());
    let child = cmd.spawn().unwrap();
    let daemon = crate::common::Daemon(child);
    crate::common::wait_until(|| std::os::unix::net::UnixStream::connect(env.socket()).is_ok());
    let mut app = env.app().await;
    // A real user of every Linux host, on the list; this user (the owner) never is.
    let nobody = hive_protocol::Person {
        login: "nobody".into(),
        display: "Nobody".into(),
        git_name: String::new(),
        git_email: String::new(),
        role: hive_protocol::ShareRole::Viewer,
    };
    let on = Sharing {
        on: true,
        group: group(),
        spaces: vec![],
        people: vec![nobody],
    };
    app.send(
        0,
        Control::SetSharing {
            sharing: on.clone(),
        },
    )
    .await;
    let (_, reply) = app.control().await;
    let Control::Sharing {
        sharing,
        owner,
        unavailable,
        error,
        ..
    } = reply
    else {
        panic!("{reply:?}")
    };
    assert_eq!(
        (sharing, owner, unavailable, error),
        (on, login(), None, None)
    );
    let dir = shared_dir(&env);
    let meta = std::fs::metadata(&dir).unwrap();
    assert_eq!(
        (meta.mode() & 0o777, meta.gid()),
        (0o710, nix::unistd::getegid().as_raw())
    );
    let socket = std::fs::metadata(dir.join("hive.sock")).unwrap();
    assert_eq!(socket.mode() & 0o777, 0o660);
    // Joining as this user: the kernel names the owner, who is not on the list.
    let mut cmd = tokio::process::Command::from(env.hive());
    let mut child = cmd
        .args(["bridge", "--join", &login()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut to_bridge = FramedWrite::new(child.stdin.take().unwrap(), FrameCodec);
    let mut from_bridge = FramedRead::new(child.stdout.take().unwrap(), FrameCodec);
    let hello = Control::hello(Role::App, hive::VERSION);
    to_bridge.send(Frame::control(0, &hello)).await.unwrap();
    let next = tokio::time::timeout(TIMEOUT, from_bridge.next())
        .await
        .unwrap();
    let refused = next.unwrap().unwrap().to_control().unwrap();
    let message = format!("{} is not on {}'s Sharing list", login(), login());
    assert_eq!(refused, Control::Error { message });
    // The owner's own socket keeps one app: a second is told the service is shared and is
    // refused as before (several apps are 29.13's).
    let mut second = env.raw().await;
    second
        .send(0, Control::hello(Role::App, hive::VERSION))
        .await;
    let (_, welcome) = second.control().await;
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
    let refused = Control::Error {
        message: "another app is already connected".into(),
    };
    assert_eq!(second.control().await, (0, refused));
    // A hook on the shared socket gets nothing either.
    let stream = tokio::net::UnixStream::connect(dir.join("hive.sock"))
        .await
        .unwrap();
    let (read, write) = stream.into_split();
    let mut writer = FramedWrite::new(write, FrameCodec);
    writer
        .send(Frame::control(
            0,
            &Control::hello(Role::Hook, hive::VERSION),
        ))
        .await
        .unwrap();
    let mut reader = FramedRead::new(read, FrameCodec);
    let answer = reader.next().await.unwrap().unwrap().to_control().unwrap();
    assert!(matches!(answer, Control::Error { .. }), "{answer:?}");
    // Off: the socket and its folder go.
    app.send(
        0,
        Control::SetSharing {
            sharing: Sharing::default(),
        },
    )
    .await;
    let (_, reply) = app.control().await;
    assert!(matches!(reply, Control::Sharing { .. }), "{reply:?}");
    assert!(!dir.exists());
    // A folder planted in its place (a link to one of ours stands for another user's): sharing
    // on says why, and nothing is created in it.
    let target = env.path("planted");
    std::fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, &dir).unwrap();
    let on = Sharing {
        on: true,
        group: group(),
        ..Sharing::default()
    };
    app.send(
        0,
        Control::SetSharing {
            sharing: on.clone(),
        },
    )
    .await;
    let (_, reply) = app.control().await;
    let Control::Sharing { error, .. } = reply else {
        panic!("{reply:?}")
    };
    assert!(error.unwrap().contains("someone else made it first"));
    assert!(!target.join("hive.sock").exists());
    // Gone, on again, then the service ends: the socket and folder go too.
    std::fs::remove_file(&dir).unwrap();
    app.send(
        0,
        Control::SetSharing {
            sharing: Sharing::default(),
        },
    )
    .await;
    let _ = app.control().await;
    app.send(0, Control::SetSharing { sharing: on }).await;
    let _ = app.control().await;
    assert!(dir.join("hive.sock").exists());
    drop(app);
    crate::common::stop(daemon);
    assert!(!dir.exists());
}
