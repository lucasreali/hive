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
use tokio::net::UnixStream;
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
            error: Mutex::new(store.problem()),
            store,
            users,
            unavailable: (!can).then_some(UNAVAILABLE),
            root,
            open: Mutex::default(),
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
        let listener = Arc::new(listener);
        let accept = move || {
            let listener = listener.clone();
            async move { listener.accept().await.map(|(stream, _)| stream) }
        };
        let task = tokio::spawn(accept_guests(accept, self.clone()));
        Ok(Open {
            group: group.to_owned(),
            dir,
            id,
            task,
        })
    }

    /// Whether the shared socket is open (`welcome.shared`).
    pub(super) fn shared(&self) -> bool {
        lock(&self.share.open).is_some()
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

/// How long a failed accept on the shared socket waits before the next (e.g. no file left).
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// Serves each connection `accept` gives (the shared socket's), until the task is aborted.
async fn accept_guests<A, F>(mut accept: A, state: Arc<State>)
where
    A: FnMut() -> F,
    F: Future<Output = std::io::Result<UnixStream>>,
{
    loop {
        match accept().await {
            Ok(stream) => drop(tokio::spawn(guest(stream, state.clone()))),
            Err(_) => tokio::time::sleep(ACCEPT_RETRY).await,
        }
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
        shared: true,
    };
    // A failed write shows as the end of its reads.
    let _ = writer.send(Frame::control(0, &welcome)).await;
    // Held until it closes or is closed: its frames are 29.13's to serve.
    loop {
        tokio::select! {
            frame = reader.next() => if !matches!(frame, Some(Ok(_))) { break },
            () = close.notified() => break,
        }
    }
    lock(&share.guests).retain(|seat| seat.id != id);
}

#[cfg(test)]
mod tests;
