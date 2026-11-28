//! Sharing a remote Linux service with teammates (29.12, `docs/spike/multiplayer.md` §3, §8):
//! the Sharing settings and their checks, who a uid is, and the shared folder in `/tmp` that
//! both ends check.
//!
//! - **Identity** comes from the kernel only: the shared socket's peer uid, named through
//!   [`Users`] (`getpwuid`), must be a login on the Sharing list. Nothing an app says is believed.
//! - **The shared folder** is `/tmp/hive-shared-<owner uid>` (Q8; `HIVE_SHARED_ROOT` replaces
//!   `/tmp` for the tests and a development run), mode `0710` in the sharing group, holding
//!   `hive.sock`, mode `0660` in that group: on Linux, connecting to a Unix socket needs write
//!   permission on it, so the group is the first gate. `/tmp` is everyone's, so the owner refuses
//!   a folder someone else made first ([`open`]), and the joining bridge refuses one that is not
//!   the owner's, or that others can write to, and a socket whose peer is not the owner
//!   ([`join`]).

use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use hive_protocol::{
    DISPLAY_LIMIT, Guest, PEOPLE_LIMIT, Person, Sharing, SpaceChoice, check_login,
};
use tokio::net::{UnixListener, UnixStream};

use crate::wrapper::write_atomic;

/// The variable that replaces `/tmp` as the shared folder's parent (tests, development).
pub const ROOT_VAR: &str = "HIVE_SHARED_ROOT";
/// The shared socket's name in the shared folder.
pub const SOCKET: &str = "hive.sock";
/// Largest `sharing.json` read.
const FILE_LIMIT: u64 = 64 * 1024;
/// The longest git author name of a person, in characters.
const GIT_NAME_LIMIT: usize = 100;
/// The longest git author e-mail of a person, in bytes.
const GIT_EMAIL_LIMIT: usize = 254;

/// Who a uid is, and the groups this process is in: the system's ([`System`]), a fake one in
/// tests (real second Unix users are 29.19's).
pub trait Users: Send + Sync {
    /// The login name of `uid`, if it has one.
    fn name(&self, uid: u32) -> Option<String>;
    /// The uid of the login `name`, if there is one.
    fn uid(&self, name: &str) -> Option<u32>;
    /// The groups this process is in, `(gid, name)`, each once.
    fn groups(&self) -> Vec<(u32, String)>;
}

/// The system's users and groups (`getpwuid`, `getpwnam`, `getgroups`).
pub struct System;

impl Users for System {
    fn name(&self, uid: u32) -> Option<String> {
        let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid));
        user.ok().flatten().map(|user| user.name)
    }

    fn uid(&self, name: &str) -> Option<u32> {
        let user = nix::unistd::User::from_name(name).ok().flatten();
        user.map(|user| user.uid.as_raw())
    }

    fn groups(&self) -> Vec<(u32, String)> {
        // macOS has no `getgroups` in nix; it is never shared anyway (a remote is Linux, Q5).
        #[cfg(target_os = "linux")]
        let mut gids = nix::unistd::getgroups().unwrap_or_default();
        #[cfg(not(target_os = "linux"))]
        let mut gids = Vec::new();
        gids.insert(0, nix::unistd::getegid());
        let named = gids.into_iter().filter_map(|gid| {
            let group = nix::unistd::Group::from_gid(gid).ok().flatten()?;
            Some((gid.as_raw(), group.name))
        });
        let mut groups: Vec<(u32, String)> = Vec::new();
        named.for_each(|(gid, name)| {
            if !groups.iter().any(|(id, _)| *id == gid) {
                groups.push((gid, name));
            }
        });
        groups
    }
}

/// The shared folders' parent: `HIVE_SHARED_ROOT` when absolute, else `/tmp`.
pub fn root(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    var(ROOT_VAR)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// The shared folder of the owner `uid` under `root`.
pub fn dir(root: &Path, uid: u32) -> PathBuf {
    root.join(format!("hive-shared-{uid}"))
}

/// The Sharing settings, kept in `sharing.json` in Hive's folder (`0600`).
pub struct Store {
    file: PathBuf,
    current: Mutex<Sharing>,
    /// Why the file was ignored when read, if it was.
    problem: Option<String>,
}

impl Store {
    /// Reads `file`; a missing, too large or unreadable one is sharing off (a warning says so).
    pub fn load(file: PathBuf) -> Self {
        let read = std::fs::File::open(&file)
            .and_then(|mut f| crate::git::read_limited(&mut f, FILE_LIMIT))
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other));
        let (current, problem) = match read {
            Ok(sharing) => (sharing, None),
            Err(err) if err.kind() == io::ErrorKind::NotFound => (Sharing::default(), None),
            Err(err) => {
                let problem = format!("ignoring {} ({err}): sharing is off", file.display());
                eprintln!("hive: warning: {problem}");
                (Sharing::default(), Some(problem))
            }
        };
        Self {
            file,
            current: Mutex::new(current),
            problem,
        }
    }

    /// Why the file was ignored when read (shown as Settings → Sharing's error), if it was.
    pub fn problem(&self) -> Option<String> {
        self.problem.clone()
    }

    pub fn get(&self) -> Sharing {
        self.current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Saves `sharing` (already checked) and keeps it.
    pub fn set(&self, sharing: Sharing) -> io::Result<()> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        let text = serde_json::to_vec_pretty(&sharing).map_err(io::Error::other)?;
        write_atomic(&self.file, &text, 0o600)?;
        *current = sharing;
        Ok(())
    }

    /// The person on the list whose login is `name`, as a [`Guest`].
    pub fn guest(&self, name: &str) -> Option<Guest> {
        let sharing = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        let person = sharing.people.iter().find(|p| p.login == name)?;
        let display = match person.display.trim() {
            "" => person.login.clone(),
            display => display.to_owned(),
        };
        Some(Guest {
            name: person.login.clone(),
            display,
            role: person.role,
        })
    }
}

/// What a Sharing list is checked against: the owner, their groups and spaces, and the host's
/// users.
pub struct Context<'a> {
    /// The owner's login name.
    pub owner: &'a str,
    pub groups: &'a [String],
    pub spaces: &'a [SpaceChoice],
    pub users: &'a dyn Users,
}

/// Whether `sharing` may be saved: sharing on needs a group; a group is one of the owner's; each
/// space is there, once; at most [`PEOPLE_LIMIT`] people, each a login of this host but the
/// owner, once, with a short printable display name, git name and e-mail.
pub fn check(sharing: &Sharing, context: &Context<'_>) -> Result<(), String> {
    if sharing.on && sharing.group.is_empty() {
        return Err("Pick the Unix group your teammates are in".to_owned());
    }
    if !sharing.group.is_empty() && !context.groups.contains(&sharing.group) {
        return Err(format!("You are not in the group {:?}", sharing.group));
    }
    for (i, space) in sharing.spaces.iter().enumerate() {
        if !context.spaces.iter().any(|s| &s.id == space) {
            return Err(format!("The space {space:?} is not there"));
        }
        if sharing.spaces[..i].contains(space) {
            return Err(format!("The space {space:?} is there twice"));
        }
    }
    if sharing.people.len() > PEOPLE_LIMIT {
        return Err(format!("At most {PEOPLE_LIMIT} people can be added"));
    }
    for (i, person) in sharing.people.iter().enumerate() {
        check_person(person, context)?;
        if sharing.people[..i].iter().any(|p| p.login == person.login) {
            return Err(format!("{} is there twice", person.login));
        }
    }
    Ok(())
}

fn check_person(person: &Person, context: &Context<'_>) -> Result<(), String> {
    let login = &person.login;
    check_login(login)?;
    if login == context.owner {
        return Err(format!("{login} is you: this Hive is yours"));
    }
    if context.users.uid(login).is_none() {
        return Err(format!("There is no user {login} on this host"));
    }
    let printable = |text: &str| !text.chars().any(char::is_control);
    let field = |what: &str, text: &str, limit: usize| {
        if !printable(text) {
            Err(format!("{login}'s {what} must not hold control characters"))
        } else if text.chars().count() > limit {
            Err(format!("{login}'s {what} is at most {limit} characters"))
        } else {
            Ok(())
        }
    };
    field("display name", &person.display, DISPLAY_LIMIT)?;
    field("git name", &person.git_name, GIT_NAME_LIMIT)?;
    field("git e-mail", &person.git_email, GIT_EMAIL_LIMIT)?;
    let email = &person.git_email;
    if !email.is_empty() && (!email.contains('@') || email.contains(char::is_whitespace)) {
        return Err(format!(
            "{login}'s git e-mail {email:?} is not an e-mail address"
        ));
    }
    Ok(())
}

/// A file's identity, (device, inode).
pub type Id = (u64, u64);

fn id_of(meta: &std::fs::Metadata) -> Id {
    (meta.dev(), meta.ino())
}

/// Makes the owner's (this process's, `uid`) shared folder `dir`, or takes back one it made
/// before, mode `0710` in the group `gid`, and binds its socket, mode `0660` in that group. A
/// folder that is not ours (planted, or a link) is refused, and nothing is created in it.
pub fn open(dir: &Path, uid: u32, gid: u32) -> io::Result<(UnixListener, Id)> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(err) if err.kind() != io::ErrorKind::AlreadyExists => return Err(err),
        _ => {}
    }
    // lstat: a link is refused, whoever it points to.
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != uid {
        return Err(io::Error::other(format!(
            "refusing {}: someone else made it first; the host's admin can remove it",
            dir.display()
        )));
    }
    let group = Some(nix::unistd::Gid::from_raw(gid));
    nix::unistd::chown(dir, None, group)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o710))?;
    let socket = dir.join(SOCKET);
    // One a stopped service left: only we can write in our folder.
    match std::fs::remove_file(&socket) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
        _ => {}
    }
    // Bound with the umask's mode, so no teammate can connect before the group is right.
    let listener = UnixListener::bind(&socket)?;
    nix::unistd::chown(&socket, None, group)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o660))?;
    Ok((listener, id_of(&std::fs::symlink_metadata(&socket)?)))
}

/// Removes the socket [`open`] bound (unless it is another file now) and the folder, once empty.
pub fn close(dir: &Path, id: Id) {
    let socket = dir.join(SOCKET);
    let ours = std::fs::symlink_metadata(&socket).is_ok_and(|meta| id_of(&meta) == id);
    if ours {
        let _ = std::fs::remove_file(socket);
    }
    let _ = std::fs::remove_dir(dir);
}

/// Why joining someone's Hive failed.
#[derive(Debug, PartialEq, Eq)]
pub enum JoinError {
    /// Their Hive is not running (or does not share): the link tries again later.
    Down,
    /// Joining is refused: why.
    Refused(String),
}

impl JoinError {
    /// The line the bridge prints on stderr for the app (`classify` in `src-tauri/src/remote.rs`).
    pub fn line(&self) -> String {
        match self {
            Self::Down => "hive-join-down".to_owned(),
            Self::Refused(why) => format!("hive-join: {why}"),
        }
    }
}

/// Connects to `owner`'s shared socket under `root` (`hive bridge --join`): their folder must be
/// theirs (lstat) with no write for the group or others, and the socket's peer must be them.
pub async fn join(root: &Path, owner: &str, users: &dyn Users) -> Result<UnixStream, JoinError> {
    let refuse = |err: io::Error| JoinError::Refused(err.to_string());
    check_login(owner).map_err(JoinError::Refused)?;
    let uid = users
        .uid(owner)
        .ok_or_else(|| JoinError::Refused(format!("there is no user {owner} on this host")))?;
    let dir = dir(root, uid);
    let meta = match std::fs::symlink_metadata(&dir) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(JoinError::Down),
        meta => meta.map_err(refuse)?,
    };
    check_folder(&dir, &meta, uid, owner)?;
    let stream = UnixStream::connect(dir.join(SOCKET))
        .await
        .map_err(|err| match err.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => JoinError::Down,
            io::ErrorKind::PermissionDenied => JoinError::Refused(format!(
                "you are not in the group {owner}'s Hive is shared to"
            )),
            _ => refuse(err),
        })?;
    let peer = stream.peer_cred().map_err(refuse)?.uid();
    check_peer(&dir, peer, uid, owner)?;
    Ok(stream)
}

/// The owner's (`uid`) shared folder `dir`, as lstat saw it (`meta`): a folder of theirs, and
/// no one else may write to it.
fn check_folder(
    dir: &Path,
    meta: &std::fs::Metadata,
    uid: u32,
    owner: &str,
) -> Result<(), JoinError> {
    if !meta.is_dir() || meta.uid() != uid {
        Err(JoinError::Refused(format!(
            "{} is not {owner}'s: someone else made it",
            dir.display()
        )))
    } else if meta.mode() & 0o022 != 0 {
        Err(JoinError::Refused(format!(
            "others can write to {}",
            dir.display()
        )))
    } else {
        Ok(())
    }
}

/// The shared socket in `dir` is served by its owner (`uid`): its `peer` is them, so a socket
/// swapped in after the folder check is refused too.
fn check_peer(dir: &Path, peer: u32, uid: u32, owner: &str) -> Result<(), JoinError> {
    if peer == uid {
        Ok(())
    } else {
        Err(JoinError::Refused(format!(
            "the socket in {} is not run by {owner} (uid {peer})",
            dir.display()
        )))
    }
}

/// A host's users for tests: `users`, `(uid, name)`, and this process's `groups`.
#[cfg(test)]
pub(crate) struct Fake {
    pub users: Vec<(u32, String)>,
    pub groups: Vec<(u32, String)>,
}

#[cfg(test)]
impl Users for Fake {
    fn name(&self, uid: u32) -> Option<String> {
        let found = self.users.iter().find(|(id, _)| *id == uid);
        found.map(|(_, name)| name.clone())
    }
    fn uid(&self, name: &str) -> Option<u32> {
        let found = self.users.iter().find(|(_, n)| n == name);
        found.map(|(id, _)| *id)
    }
    fn groups(&self) -> Vec<(u32, String)> {
        self.groups.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_protocol::ShareRole;

    fn me() -> u32 {
        nix::unistd::getuid().as_raw()
    }

    fn fake() -> Fake {
        Fake {
            users: vec![
                (me(), "alice".into()),
                (2000, "bob".into()),
                (2001, "carol".into()),
            ],
            groups: vec![(1, "team".into())],
        }
    }

    fn person(login: &str) -> Person {
        Person {
            login: login.into(),
            display: String::new(),
            git_name: String::new(),
            git_email: String::new(),
            role: ShareRole::Viewer,
        }
    }

    fn checked(sharing: &Sharing) -> Result<(), String> {
        let users = fake();
        let groups = ["team".to_owned()];
        let spaces = [SpaceChoice {
            id: "w".into(),
            name: "Work".into(),
        }];
        let context = Context {
            owner: "alice",
            groups: &groups,
            spaces: &spaces,
            users: &users,
        };
        check(sharing, &context)
    }

    #[test]
    fn the_systems_users_name_this_process_and_its_group() {
        let users = System;
        let name = users.name(me()).expect("this user has a name");
        assert_eq!(users.uid(&name), Some(me()));
        assert_eq!(users.uid("no-such-user-hive-test"), None);
        let gid = nix::unistd::getegid().as_raw();
        let groups = users.groups();
        assert_eq!(groups.first().map(|g| g.0), Some(gid), "{groups:?}");
        let mut ids: Vec<u32> = groups.iter().map(|g| g.0).collect();
        ids.dedup();
        assert_eq!(ids.len(), groups.len(), "each once: {groups:?}");
    }

    #[test]
    fn the_shared_folder_is_under_tmp_unless_an_absolute_root_is_given() {
        let var = |value: Option<&str>| {
            let value = value.map(std::ffi::OsString::from);
            root(move |key| (key == ROOT_VAR).then(|| value.clone()).flatten())
        };
        assert_eq!(var(None), PathBuf::from("/tmp"));
        assert_eq!(var(Some("rel")), PathBuf::from("/tmp"));
        assert_eq!(var(Some("/x")), PathBuf::from("/x"));
        assert_eq!(
            dir(Path::new("/tmp"), 1000),
            PathBuf::from("/tmp/hive-shared-1000")
        );
    }

    #[test]
    fn a_sharing_list_is_checked_against_the_host() {
        let mut sharing = Sharing {
            on: true,
            group: "team".into(),
            spaces: vec!["w".into()],
            people: vec![person("bob")],
        };
        assert_eq!(checked(&sharing), Ok(()));
        // Off, nothing is needed.
        assert_eq!(checked(&Sharing::default()), Ok(()));
        let with = |change: &dyn Fn(&mut Sharing)| {
            let mut s = sharing.clone();
            change(&mut s);
            checked(&s).unwrap_err()
        };
        assert_eq!(
            with(&|s| s.group.clear()),
            "Pick the Unix group your teammates are in"
        );
        assert_eq!(
            with(&|s| s.group = "wheel".into()),
            "You are not in the group \"wheel\""
        );
        assert_eq!(
            with(&|s| s.spaces.push("x".into())),
            "The space \"x\" is not there"
        );
        assert_eq!(
            with(&|s| s.spaces.push("w".into())),
            "The space \"w\" is there twice"
        );
        assert_eq!(
            with(&|s| s.people.push(person("bob"))),
            "bob is there twice"
        );
        assert_eq!(
            with(&|s| s.people.push(person("alice"))),
            "alice is you: this Hive is yours"
        );
        assert_eq!(
            with(&|s| s.people.push(person("dave"))),
            "There is no user dave on this host"
        );
        assert_eq!(
            with(&|s| s.people.push(person("Bob"))),
            check_login("Bob").unwrap_err()
        );
        let many = |s: &mut Sharing| s.people = (0..=PEOPLE_LIMIT).map(|_| person("bob")).collect();
        assert_eq!(with(&many), "At most 32 people can be added");
        let at_most = |s: &mut Sharing| {
            s.people = (0..PEOPLE_LIMIT).map(|_| person("bob")).collect();
        };
        // The limit itself is fine (the doubles then fail).
        assert_eq!(with(&at_most), "bob is there twice");
        // A person's texts.
        let mut bob = |change: &dyn Fn(&mut Person)| {
            let mut p = person("bob");
            change(&mut p);
            sharing.people = vec![p];
            checked(&sharing)
        };
        assert_eq!(bob(&|p| p.display = "B".repeat(DISPLAY_LIMIT)), Ok(()));
        assert_eq!(
            bob(&|p| p.display = "B".repeat(DISPLAY_LIMIT + 1)),
            Err("bob's display name is at most 40 characters".into())
        );
        assert_eq!(
            bob(&|p| p.display = "B\u{1b}[2J".into()),
            Err("bob's display name must not hold control characters".into())
        );
        assert_eq!(bob(&|p| p.git_name = "n".repeat(GIT_NAME_LIMIT)), Ok(()));
        assert_eq!(
            bob(&|p| p.git_name = "n".repeat(GIT_NAME_LIMIT + 1)),
            Err("bob's git name is at most 100 characters".into())
        );
        assert_eq!(
            bob(&|p| p.git_name = "a\nb".into()),
            Err("bob's git name must not hold control characters".into())
        );
        assert_eq!(bob(&|p| p.git_email = "bob@example.com".into()), Ok(()));
        assert_eq!(
            bob(&|p| p.git_email = format!("{}@x", "e".repeat(GIT_EMAIL_LIMIT))),
            Err("bob's git e-mail is at most 254 characters".into())
        );
        for bad in ["bob", "bob @x", "b@x y"] {
            assert_eq!(
                bob(&|p| p.git_email = bad.into()),
                Err(format!("bob's git e-mail {bad:?} is not an e-mail address")),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_sharing_list_is_kept_private_and_read_back() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("sharing.json");
        let store = Store::load(file.clone());
        assert_eq!(store.get(), Sharing::default());
        assert_eq!(store.guest("bob"), None);
        // A missing file is no problem.
        assert_eq!(store.problem(), None);
        let mut bob = person("bob");
        bob.role = ShareRole::Collaborator;
        let mut carol = person("carol");
        carol.display = "  Carol C ".into();
        let sharing = Sharing {
            on: true,
            group: "team".into(),
            spaces: vec![],
            people: vec![bob, carol],
        };
        store.set(sharing.clone()).unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(Store::load(file.clone()).get(), sharing);
        // A display name defaults to the login.
        let guest = |name, display: &str, role| {
            Some(Guest {
                name: String::from(name),
                display: display.into(),
                role,
            })
        };
        assert_eq!(
            store.guest("bob"),
            guest("bob", "bob", ShareRole::Collaborator)
        );
        assert_eq!(
            store.guest("carol"),
            guest("carol", "Carol C", ShareRole::Viewer)
        );
        assert_eq!(store.guest("alice"), None);
        // A file as large as the limit is read.
        let mut text = serde_json::to_vec(&sharing).unwrap();
        text.resize(FILE_LIMIT as usize, b' ');
        assert_eq!(FILE_LIMIT, 64 * 1024);
        assert_eq!(Store::load(file.clone()).problem(), None);
        std::fs::write(&file, &text).unwrap();
        let read = Store::load(file.clone());
        assert_eq!((read.get(), read.problem()), (sharing.clone(), None));
        // Unreadable or too large: sharing is off, and why is kept.
        std::fs::write(&file, "{").unwrap();
        let broken = Store::load(file.clone());
        assert_eq!(broken.get(), Sharing::default());
        let problem = broken.problem().unwrap();
        assert!(
            problem.starts_with(&format!("ignoring {} (", file.display())),
            "{problem}"
        );
        assert!(problem.ends_with("): sharing is off"), "{problem}");
        text.push(b' ');
        std::fs::write(&file, &text).unwrap();
        assert_eq!(Store::load(file.clone()).get(), Sharing::default());
        // A failed write keeps the list in use.
        let gone = Store::load(tmp.path().join("missing/sharing.json"));
        assert!(gone.set(sharing).is_err());
        assert_eq!(gone.get(), Sharing::default());
    }

    #[tokio::test]
    async fn the_shared_folder_and_socket_are_made_for_the_group_and_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let gid = nix::unistd::getegid().as_raw();
        let dir = dir(tmp.path(), me());
        let (listener, id) = open(&dir, me(), gid).unwrap();
        let folder = std::fs::symlink_metadata(&dir).unwrap();
        assert_eq!(folder.mode() & 0o777, 0o710);
        assert_eq!(folder.gid(), gid);
        let socket = std::fs::symlink_metadata(dir.join(SOCKET)).unwrap();
        assert_eq!(socket.mode() & 0o777, 0o660);
        assert_eq!(socket.gid(), gid);
        // The owner joins its own folder: the checks pass.
        let users = fake();
        let joined = join(tmp.path(), "alice", &users).await.unwrap();
        listener.accept().await.unwrap();
        drop((joined, listener));
        // Taken back by a new service: the old socket is replaced by one that listens.
        let (listener, again) = open(&dir, me(), gid).unwrap();
        let users = fake();
        let joined = join(tmp.path(), "alice", &users).await.unwrap();
        listener.accept().await.unwrap();
        drop(joined);
        // Another file put at the socket's path (alive with it, so never its inode) is not
        // removed, and the folder then stays.
        let other = dir.join("other");
        std::fs::write(&other, "").unwrap();
        assert_ne!(id_of(&std::fs::metadata(&other).unwrap()), again);
        std::fs::rename(&other, dir.join(SOCKET)).unwrap();
        close(&dir, again);
        assert!(dir.join(SOCKET).exists());
        std::fs::remove_file(dir.join(SOCKET)).unwrap();
        close(&dir, id);
        assert!(!dir.exists());
        // Nothing to close: nothing happens.
        close(&dir, again);
    }

    #[test]
    fn a_planted_shared_folder_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let gid = nix::unistd::getegid().as_raw();
        let dir = dir(tmp.path(), me());
        std::fs::create_dir(&dir).unwrap();
        // Ours, but taken as another uid's: someone else made it first.
        let err = open(&dir, me() + 1, gid).unwrap_err();
        assert!(
            err.to_string().contains("someone else made it first"),
            "{err}"
        );
        assert!(!dir.join(SOCKET).exists());
        // A link to a folder of ours is refused too.
        std::fs::remove_dir(&dir).unwrap();
        let target = tmp.path().join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &dir).unwrap();
        let err = open(&dir, me(), gid).unwrap_err();
        assert!(
            err.to_string().contains("someone else made it first"),
            "{err}"
        );
        assert!(!target.join(SOCKET).exists());
        // A parent that is not there.
        let err = open(&tmp.path().join("no/such"), me(), gid).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn joining_checks_the_owners_folder_and_socket() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let users = fake();
        let refused = |why: &str| Err(JoinError::Refused(why.into()));
        let joined = |owner: &'static str| async move {
            let users = fake();
            join(root, owner, &users).await.map(drop)
        };
        assert_eq!(
            joined("Bad").await,
            refused(&check_login("Bad").unwrap_err())
        );
        assert_eq!(
            joined("dave").await,
            refused("there is no user dave on this host")
        );
        // Not running: no folder, or a folder with no socket listening.
        assert_eq!(joined("alice").await, Err(JoinError::Down));
        let mine = dir(root, me());
        std::fs::DirBuilder::new()
            .mode(0o710)
            .create(&mine)
            .unwrap();
        assert_eq!(joined("alice").await, Err(JoinError::Down));
        // A socket a stopped service left.
        drop(std::os::unix::net::UnixListener::bind(mine.join(SOCKET)).unwrap());
        assert_eq!(joined("alice").await, Err(JoinError::Down));
        std::fs::remove_file(mine.join(SOCKET)).unwrap();
        // Others or the group can write to it.
        for mode in [0o712, 0o730] {
            std::fs::set_permissions(&mine, std::fs::Permissions::from_mode(mode)).unwrap();
            let why = format!("others can write to {}", mine.display());
            assert_eq!(joined("alice").await, refused(&why), "{mode:o}");
        }
        // bob's folder (uid 2000) is ours, not bob's: planted.
        let bobs = dir(root, 2000);
        std::fs::create_dir(&bobs).unwrap();
        let why = format!("{} is not bob's: someone else made it", bobs.display());
        assert_eq!(joined("bob").await, refused(&why));
        // A file in its place.
        let carols = dir(root, 2001);
        std::fs::write(&carols, "").unwrap();
        let why = format!("{} is not carol's: someone else made it", carols.display());
        assert_eq!(joined("carol").await, refused(&why));
        // A socket we cannot reach (a folder we may not enter stands for the group's gate).
        std::fs::set_permissions(&mine, std::fs::Permissions::from_mode(0o710)).unwrap();
        let _listener = UnixListener::bind(mine.join(SOCKET)).unwrap();
        std::fs::set_permissions(mine.join(SOCKET), std::fs::Permissions::from_mode(0o000))
            .unwrap();
        let denied = join(root, "alice", &users).await.map(drop);
        assert_eq!(
            denied,
            refused("you are not in the group alice's Hive is shared to")
        );
        // A socket run by someone else: its peer is not the owner (we stand for the planter,
        // the owner is taken as another uid).
        std::fs::set_permissions(mine.join(SOCKET), std::fs::Permissions::from_mode(0o660))
            .unwrap();
        let other = Fake {
            users: vec![(me() + 1, "erin".into())],
            groups: vec![],
        };
        let erins = dir(root, me() + 1);
        std::os::unix::fs::symlink(&mine, &erins).unwrap();
        let planted = join(root, "erin", &other).await.map(drop);
        let why = format!("{} is not erin's: someone else made it", erins.display());
        assert_eq!(planted, refused(&why));
        assert_eq!(JoinError::Down.line(), "hive-join-down");
        assert_eq!(JoinError::Refused("x".into()).line(), "hive-join: x");
    }

    #[test]
    fn a_socket_whose_peer_is_not_the_owner_is_refused() {
        let dir = Path::new("/tmp/hive-shared-7");
        assert_eq!(check_peer(dir, 7, 7, "alice"), Ok(()));
        assert_eq!(
            check_peer(dir, 8, 7, "alice"),
            Err(JoinError::Refused(
                "the socket in /tmp/hive-shared-7 is not run by alice (uid 8)".into()
            ))
        );
    }

    #[tokio::test]
    async fn a_shared_folder_that_cannot_be_read_or_reached_is_refused_with_why() {
        let tmp = tempfile::tempdir().unwrap();
        let users = fake();
        // The root is a file: no folder can be under it.
        let file = tmp.path().join("file");
        std::fs::write(&file, "").unwrap();
        let err = join(&file, "alice", &users).await.unwrap_err();
        assert!(matches!(err, JoinError::Refused(_)), "{err:?}");
        // A socket path too long to connect to.
        let long = tmp.path().join("l".repeat(120));
        let mine = dir(&long, me());
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::set_permissions(&mine, std::fs::Permissions::from_mode(0o710)).unwrap();
        let err = join(&long, "alice", &users).await.unwrap_err();
        assert!(matches!(err, JoinError::Refused(_)), "{err:?}");
        // The owner's side: a folder at the socket's path cannot be cleared, and says so.
        let gid = nix::unistd::getegid().as_raw();
        let ours = dir(tmp.path(), me());
        std::fs::create_dir_all(ours.join(SOCKET)).unwrap();
        let cleared = std::fs::remove_file(ours.join(SOCKET)).unwrap_err();
        assert_eq!(open(&ours, me(), gid).unwrap_err().kind(), cleared.kind());
        // A parent it may not write in: the folder cannot be made, and says why.
        let closed = tmp.path().join("closed");
        std::fs::create_dir(&closed).unwrap();
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o500)).unwrap();
        let err = open(&dir(&closed, me()), me(), gid).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }
}
