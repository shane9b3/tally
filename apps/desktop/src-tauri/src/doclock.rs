//! Document lock files, interoperable with Apache OpenOffice 4.1 (and
//! LibreOffice, which kept the format).
//!
//! While AOO has a document open for editing it keeps `.~lock.<name>#` next
//! to it (`svl/source/misc/documentlockfile.cxx`): one entry of five fields,
//! each escaped (`\` before `\`, `,` and `;`), separated by `,` and ended by
//! `;`, UTF-8, no newline (`DocumentLockFile::WriteEntryToStream`):
//!
//! ```text
//! <user name>,<system user>,<host>,<DD.MM.YYYY HH:MM>,<user installation URL>;
//! ```
//!
//! (`LOCKFILE_OOOUSERNAME_ID` … `LOCKFILE_USERURL_ID` in
//! `svl/inc/svl/lockfilecommon.hxx`.) The user name is Tools ▸ Options ▸
//! User Data (may be empty); the time is local time
//! (`LockFileCommon::GetCurrentLocalTime`).
//!
//! What AOO does with it (`SfxMedium::LockOrigFileOnDemand`,
//! `sfx2/source/doc/docfile.cxx`), and Tally with it:
//!
//! - Opening for editing creates the lock with "no replace". If one exists,
//!   it is read: the same system user, host and user installation URL means
//!   our own lock left by a crash, which is taken over silently; the same
//!   system user only means "locked by yourself on a different system"
//!   (Open Read-Only / Open / Cancel); anyone else (or an unreadable lock)
//!   means "locked for editing by <name> (<date>)" (Open Read-Only /
//!   Open Copy / Cancel).
//! - A lock that cannot be written (a read-only directory) is ignored: with
//!   AOO's defaults (`UseDocumentSystemFileLocking`) the document opens for
//!   editing without one.
//! - Closing removes the lock only if it still names us
//!   (`DocumentLockFile::RemoveFile` compares system user, host and URL);
//!   Tally never removes a lock it did not write.
//!
//! Tally's user installation URL is its own config directory, so OpenOffice
//! run by the same account treats Tally's lock as "locked by yourself on a
//! different system", and Tally treats OpenOffice's the same way; nobody
//! takes the other's lock over silently. See docs/security.md.

use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde::Serialize;
use tally_api::ApiError;

use crate::state::lock;

/// Prefix of AOO's document lock file (`DocumentLockFile`).
pub const PREFIX: &str = ".~lock.";
/// AOO reads at most this many bytes; a lock file this large or larger is
/// malformed (`DocumentLockFile::GetLockData`).
pub const MAX_LOCK_BYTES: usize = 32_000;
/// Longest value Tally writes into a field (characters).
const MAX_FIELD: usize = 256;

/// One lock entry (AOO's five fields).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LockEntry {
    /// User name from the application's user data (may be empty).
    pub name: String,
    /// The OS account name.
    pub sys_user: String,
    /// The host name.
    pub host: String,
    /// When the lock was taken, `DD.MM.YYYY HH:MM` local time.
    pub time: String,
    /// The user installation (profile) URL.
    pub user_url: String,
}

/// Escapes one field as `LockFileCommon::EscapeCharacters`.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '\\' | ',' | ';') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// One field as `LockFileCommon::ParseName`: bytes up to an unescaped `,`
/// or `;` (which is not consumed); `\` escapes only `\`, `,` and `;`.
fn parse_name(buf: &[u8], pos: &mut usize) -> Option<String> {
    let mut out = Vec::new();
    let mut escape = false;
    loop {
        let b = *buf.get(*pos)?;
        if escape {
            if !matches!(b, b',' | b';' | b'\\') {
                return None;
            }
            out.push(b);
            escape = false;
            *pos += 1;
        } else if b == b',' || b == b';' {
            break;
        } else {
            if b == b'\\' {
                escape = true;
            } else {
                out.push(b);
            }
            *pos += 1;
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

impl LockEntry {
    /// Parses a lock file as `LockFileCommon::ParseEntry` (five fields,
    /// `,` between them, `;` after the last; anything after is ignored).
    /// `None` when malformed or too large.
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() >= MAX_LOCK_BYTES {
            return None;
        }
        let mut pos = 0;
        let mut fields = Vec::with_capacity(5);
        for i in 0..5 {
            fields.push(parse_name(buf, &mut pos)?);
            let sep = *buf.get(pos)?;
            pos += 1;
            if (i < 4 && sep != b',') || (i == 4 && sep != b';') {
                return None;
            }
        }
        let mut it = fields.into_iter();
        Some(Self {
            name: it.next()?,
            sys_user: it.next()?,
            host: it.next()?,
            time: it.next()?,
            user_url: it.next()?,
        })
    }

    /// The file contents (`DocumentLockFile::WriteEntryToStream`).
    pub fn to_bytes(&self) -> Vec<u8> {
        let fields = [
            &self.name,
            &self.sys_user,
            &self.host,
            &self.time,
            &self.user_url,
        ];
        let mut s = String::new();
        for (i, f) in fields.iter().enumerate() {
            s.push_str(&escape(f));
            s.push(if i < 4 { ',' } else { ';' });
        }
        s.into_bytes()
    }

    /// Written by this user on this host from this installation: ours
    /// (`DocumentLockFile::RemoveFile`, and the crash case of
    /// `LockOrigFileOnDemand`).
    /// An unknown account (`me.sys_user` empty) owns nothing.
    pub fn same_owner(&self, me: &Identity) -> bool {
        self.same_user(me) && self.host == me.host && self.user_url == me.user_url
    }

    /// Written by this OS account (maybe elsewhere): AOO's `bOwnLock`.
    /// An unknown account (`me.sys_user` empty) matches nothing.
    pub fn same_user(&self, me: &Identity) -> bool {
        !me.sys_user.is_empty() && self.sys_user == me.sys_user
    }
}

/// Who Tally writes into its locks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    /// Full name (may be empty, as AOO's with no user data).
    pub name: String,
    /// OS account name.
    pub sys_user: String,
    /// Host name.
    pub host: String,
    /// Tally's config directory as a `file://` URL.
    pub user_url: String,
}

/// Longest owner name shown in the "Document in Use" question (characters).
const MAX_OWNER: usize = 64;

/// Unicode format characters (category Cf: bidi marks, embeddings,
/// overrides and isolates, zero-width characters, tags): a lock file must
/// not reorder or hide the text of the question it appears in.
pub fn is_format_char(c: char) -> bool {
    matches!(
        u32::from(c),
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// Keeps printable characters only (no control or format characters), at
/// most [`MAX_FIELD`] of them.
fn clean(s: &str) -> String {
    clean_max(s, MAX_FIELD)
}

fn clean_max(s: &str, max: usize) -> String {
    s.trim()
        .chars()
        .filter(|c| !c.is_control() && !is_format_char(*c))
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// First line of a command's output (no shell; fixed argv).
fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = clean(text.lines().next().unwrap_or(""));
    (!line.is_empty()).then_some(line)
}

/// The account's full name: `id -F` on macOS, the passwd GECOS field
/// (`getent passwd`) on Linux; empty elsewhere or when unknown.
fn full_name(sys_user: &str) -> String {
    if sys_user.is_empty() {
        return String::new();
    }
    if cfg!(target_os = "macos") {
        return command_line("/usr/bin/id", &["-F", sys_user]).unwrap_or_default();
    }
    if cfg!(target_os = "linux") {
        let line = command_line("/usr/bin/getent", &["passwd", sys_user]).unwrap_or_default();
        return clean(
            line.split(':')
                .nth(4)
                .unwrap_or("")
                .split(',')
                .next()
                .unwrap_or(""),
        );
    }
    String::new()
}

/// The host name (`hostname`, else the environment).
fn host_name() -> String {
    if cfg!(windows) {
        return std::env::var("COMPUTERNAME")
            .map(|h| clean(&h))
            .unwrap_or_default();
    }
    command_line("/bin/hostname", &[])
        .or_else(|| std::env::var("HOSTNAME").ok().map(|h| clean(&h)))
        .unwrap_or_default()
}

/// The OS account name: `USER` / `USERNAME` / `LOGNAME`, else
/// `/usr/bin/id -un` (Unix); empty when unknown (then no lock is ever ours).
fn account_name() -> String {
    let from_env = crate::links::user_name()
        .map(|u| clean(&u))
        .unwrap_or_default();
    if !from_env.is_empty() || cfg!(windows) {
        return from_env;
    }
    command_line("/usr/bin/id", &["-un"]).unwrap_or_default()
}

/// A path as a `file://` URL (bytes outside `A-Za-z0-9-._~/` percent-encoded).
pub fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl Identity {
    /// The current user: full name, account, host, and `config_dir` as the
    /// user installation URL. Runs `id`/`hostname` (no shell).
    pub fn detect(config_dir: Option<&Path>) -> Self {
        let sys_user = account_name();
        Self {
            name: full_name(&sys_user),
            host: host_name(),
            user_url: config_dir.map(file_url).unwrap_or_default(),
            sys_user,
        }
    }

    /// Our entry, stamped now.
    pub fn entry(&self) -> LockEntry {
        LockEntry {
            name: self.name.clone(),
            sys_user: self.sys_user.clone(),
            host: self.host.clone(),
            time: chrono::Local::now().format("%d.%m.%Y %H:%M").to_string(),
            user_url: self.user_url.clone(),
        }
    }
}

/// `<dir>/.~lock.<file name>#` for a document path (AOO's
/// `LockFileCommon` constructor, which first resolves symbolic links: the
/// lock sits next to the real file).
pub fn lock_path(doc: &Path) -> Option<PathBuf> {
    let real = fs::canonicalize(doc).unwrap_or_else(|_| doc.to_path_buf());
    let name = real.file_name()?.to_string_lossy();
    let dir = real.parent()?;
    Some(dir.join(format!("{PREFIX}{name}#")))
}

/// Reads a lock file without following a symbolic link and without
/// reading more than [`MAX_LOCK_BYTES`]. `Ok(None)`: it exists but is not
/// a regular file or is malformed (an unknown user's lock).
pub fn read_lock(lp: &Path) -> std::io::Result<Option<LockEntry>> {
    let meta = fs::symlink_metadata(lp)?;
    if !meta.is_file() {
        return Ok(None);
    }
    let mut opts = fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        // Never follow a link swapped in after the check, never block on a
        // FIFO; the opened file is checked again below.
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match opts.open(lp) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(e),
        // A link (ELOOP) or anything else unreadable: an unknown lock.
        Err(_) => return Ok(None),
    };
    if !file.metadata()?.is_file() {
        return Ok(None);
    }
    let mut buf = Vec::new();
    file.take(MAX_LOCK_BYTES as u64).read_to_end(&mut buf)?;
    Ok(LockEntry::parse(&buf))
}

/// Creates `lp` with `bytes` only if nothing (not even a dangling link)
/// is there (`O_CREAT|O_EXCL`), mode 0644 before the umask.
fn create_new(lp: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o644);
    }
    let mut f = opts.open(lp)?;
    let result = f.write_all(bytes).and_then(|()| f.sync_all());
    if result.is_err() {
        drop(f);
        let _ = fs::remove_file(lp);
    }
    result
}

/// A private name next to `lp` (`<lock>.tally-<what>-<pid>-<n>`).
fn private_name(lp: &Path, what: &str) -> std::io::Result<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = lp.parent().ok_or(ErrorKind::NotFound)?;
    let name = lp.file_name().ok_or(ErrorKind::NotFound)?.to_string_lossy();
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(dir.join(format!(
        "{name}.tally-{what}-{}-{nanos}-{n}",
        std::process::id()
    )))
}

/// Replaces `lp` with `bytes` (a new file renamed over it, so a link at
/// `lp` is replaced, never written through). Residual: the caller checked
/// the old lock was ours (or the user chose to take it over) just before;
/// a lock another program wrote in between would be replaced.
fn replace(lp: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = private_name(lp, "new")?;
    create_new(&tmp, bytes)?;
    fs::rename(&tmp, lp).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Someone else's lock, as the UI shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockInfo {
    /// The document's file name.
    pub file: String,
    /// The document's path (to open it read-only or as a copy).
    pub path: String,
    /// The user name, else the system user; empty when unknown ("Unknown User").
    pub owner: String,
    /// When it was locked (`DD.MM.YYYY HH:MM`), may be empty.
    pub time: String,
    /// Locked by this OS account on another system or installation
    /// (AOO's "locked for editing by yourself on a different system").
    pub own: bool,
}

impl LockInfo {
    /// The lock of another Tally document of this instance (Save As onto a
    /// file open in another tab): AOO's "locked for editing by" with our name.
    pub fn ours(doc: &Path, me: &Identity, lp: Option<&Path>) -> Self {
        let entry = lp
            .and_then(|lp| read_lock(lp).ok().flatten())
            .unwrap_or_else(|| me.entry());
        let mut info = Self::of(doc, Some(&entry), me);
        info.own = false;
        info
    }

    fn of(doc: &Path, entry: Option<&LockEntry>, me: &Identity) -> Self {
        let (owner, time, own) = match entry {
            Some(e) => (
                if e.name.is_empty() {
                    e.sys_user.clone()
                } else {
                    e.name.clone()
                },
                e.time.clone(),
                e.same_user(me),
            ),
            None => (String::new(), String::new(), false),
        };
        Self {
            file: clean(
                &doc.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            ),
            path: doc.to_string_lossy().into_owned(),
            owner: clean_max(&owner, MAX_OWNER),
            time: clean_max(&time, 32),
            own,
        }
    }

    /// The `locked` error carrying this lock (the message is JSON).
    pub fn error(&self) -> ApiError {
        ApiError::new(
            ApiError::LOCKED,
            serde_json::to_string(self).unwrap_or_default(),
        )
    }
}

/// Outcome of [`acquire`].
#[derive(Debug, PartialEq, Eq)]
pub enum Acquired {
    /// Our lock is at this path (new, or ours from a crash, or taken over).
    Locked(PathBuf),
    /// No lock could be written (read-only directory, no file name): edit
    /// without one, as AOO with its default settings.
    Unlocked,
    /// Locked by someone else.
    Busy(LockInfo),
}

/// Locks `doc` for `me` (AOO's `LockOrigFileOnDemand` for a local file).
/// `take_over_own`: the user chose "Open" for a lock of this account
/// written elsewhere (AOO's `OverwriteOwnLockFile`); never for anyone else's.
/// `other_instance`: another Tally of this user runs (it may hold the lock),
/// so our own lock is asked about ("locked by yourself") instead of being
/// taken over as a crash leftover.
pub fn acquire(me: &Identity, doc: &Path, take_over_own: bool, other_instance: bool) -> Acquired {
    let Some(lp) = lock_path(doc) else {
        return Acquired::Unlocked;
    };
    let ours = me.entry().to_bytes();
    match create_new(&lp, &ours) {
        Ok(()) => return Acquired::Locked(lp),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(_) => return Acquired::Unlocked,
    }
    let existing = match read_lock(&lp) {
        Ok(e) => e,
        // Gone meanwhile: try once more.
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return match create_new(&lp, &ours) {
                Ok(()) => Acquired::Locked(lp),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    Acquired::Busy(LockInfo::of(doc, None, me))
                }
                Err(_) => Acquired::Unlocked,
            };
        }
        Err(_) => None,
    };
    match existing {
        Some(e) if e.same_owner(me) && !other_instance => {
            // Our own lock, left by a crash: take it (fresh time).
            let _ = replace(&lp, &ours);
            Acquired::Locked(lp)
        }
        Some(e) if take_over_own && e.same_user(me) => match replace(&lp, &ours) {
            Ok(()) => Acquired::Locked(lp),
            Err(_) => Acquired::Unlocked,
        },
        other => Acquired::Busy(LockInfo::of(doc, other.as_ref(), me)),
    }
}

/// Removes the lock at `lp` if it still names us; anything else (another
/// program's lock, a malformed file, a link) is left alone. To avoid a
/// check-then-delete race, the lock is first renamed to a private name,
/// checked there, and deleted only if it is ours; otherwise it is linked
/// back (`hard_link` never overwrites a lock created meanwhile; if that
/// fails, the moved file stays under its private name and is logged).
pub fn release(me: &Identity, lp: &Path) {
    // Cheap pre-check: never move anything that is not ours.
    if !matches!(read_lock(lp), Ok(Some(e)) if e.same_owner(me)) {
        return;
    }
    let Ok(tmp) = private_name(lp, "release") else {
        return;
    };
    if fs::rename(lp, &tmp).is_err() {
        return;
    }
    if matches!(read_lock(&tmp), Ok(Some(e)) if e.same_owner(me)) {
        if let Err(e) = fs::remove_file(&tmp) {
            eprintln!("tally: removing the lock file: {e}");
        }
        return;
    }
    match fs::hard_link(&tmp, lp) {
        Ok(()) => {
            let _ = fs::remove_file(&tmp);
        }
        Err(e) => eprintln!(
            "tally: a lock file changed while releasing it; kept as {}: {e}",
            tmp.display()
        ),
    }
}

/// `true` if Tally can write `path`: it is a regular file, not flagged
/// locked/immutable (macOS Finder's "Locked", `chflags uchg`), and opens for
/// writing (permissions, ACLs, read-only volumes). AOO likewise opens a
/// read-write stream to decide (`LockOrigFileOnDemand`). Nothing is
/// written: no truncate, no create.
pub fn is_writable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.permissions().readonly() {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        // UF_IMMUTABLE | SF_IMMUTABLE (sys/stat.h).
        const IMMUTABLE: u32 = 0x0000_0002 | 0x0002_0000;
        if meta.st_flags() & IMMUTABLE != 0 {
            return false;
        }
    }
    fs::OpenOptions::new().write(true).open(path).is_ok()
}

/// A file's modification time and size, to tell whether it changed on
/// disk since Tally read or wrote it.
pub type FileStamp = (Option<SystemTime>, u64);

/// The stamp of `path` now.
pub fn stamp(path: &Path) -> Option<FileStamp> {
    let m = fs::metadata(path).ok()?;
    Some((m.modified().ok(), m.len()))
}

/// The shell's locks: who we are, the lock file each document holds and
/// the file stamps. Leaf locks: never held across file I/O or an engine call.
#[derive(Debug, Default)]
pub struct Locks {
    identity: OnceLock<Identity>,
    /// Tally's config dir (the user installation URL; set at start-up).
    config_dir: OnceLock<PathBuf>,
    held: Mutex<HashMap<u32, PathBuf>>,
    stamps: Mutex<HashMap<u32, FileStamp>>,
    /// Canonical paths being opened right now (check and reserve are one
    /// step under this lock, so two opens of one file cannot race).
    opening: Mutex<std::collections::HashSet<PathBuf>>,
    /// The instance lock (`<config>/instance.lock`), held for the process's life.
    instance: Mutex<Option<fs::File>>,
    /// Another Tally of this user runs: our own locks are never taken over silently.
    other_instance: std::sync::atomic::AtomicBool,
}

/// A path reserved by [`Locks::reserve`]; dropping it frees the path.
#[derive(Debug)]
pub struct Reservation<'a> {
    locks: &'a Locks,
    path: PathBuf,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        lock(&self.locks.opening).remove(&self.path);
    }
}

/// Takes the exclusive instance lock on `<dir>/instance.lock` (an advisory
/// `flock`, released by the OS when the process ends, also on a crash).
/// `None` when another process holds it (another Tally runs) or it cannot
/// be created.
pub fn instance_lock(dir: &Path) -> Option<fs::File> {
    fs::create_dir_all(dir).ok()?;
    let f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("instance.lock"))
        .ok()?;
    f.try_lock().ok()?;
    Some(f)
}

impl Locks {
    /// Sets who we are (once; later calls are ignored). Tests.
    pub fn set_identity(&self, id: Identity) {
        let _ = self.identity.set(id);
    }

    /// Start-up: remembers the config dir (identity is detected lazily, on
    /// the first lock, off the main thread) and takes the instance lock.
    pub fn start(&self, config_dir: Option<PathBuf>) {
        if let Some(dir) = config_dir {
            let guard = instance_lock(&dir);
            if guard.is_none() {
                eprintln!("tally: another Tally is running; its document locks are respected");
            }
            self.set_other_instance(guard.is_none());
            *lock(&self.instance) = guard;
            let _ = self.config_dir.set(dir);
        }
    }

    /// Whether another Tally of this user runs (tests set it).
    pub fn set_other_instance(&self, other: bool) {
        self.other_instance
            .store(other, std::sync::atomic::Ordering::Relaxed);
    }

    fn other_instance(&self) -> bool {
        self.other_instance
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Who we are (detected on first use; runs `id`/`hostname`).
    pub fn identity(&self) -> &Identity {
        self.identity
            .get_or_init(|| Identity::detect(self.config_dir.get().map(PathBuf::as_path)))
    }

    /// Reserves canonical `path` for an open; `None` when it is being
    /// opened already.
    pub fn reserve(&self, path: &Path) -> Option<Reservation<'_>> {
        lock(&self.opening)
            .insert(path.to_path_buf())
            .then(|| Reservation {
                locks: self,
                path: path.to_path_buf(),
            })
    }

    /// The document (other than `doc`) that holds the lock of `path`.
    fn holder_of(&self, path: &Path, doc: Option<u32>) -> Option<(u32, PathBuf)> {
        let lp = lock_path(path)?;
        lock(&self.held)
            .iter()
            .find(|(d, p)| Some(**d) != doc && **p == lp)
            .map(|(d, p)| (*d, p.clone()))
    }

    /// Locks `path` (see [`acquire`]) for `doc` (`None`: a document still
    /// being opened, filed later with [`Self::adopt`]). A lock another of
    /// our documents holds is `Busy`, never taken over.
    pub fn try_acquire(&self, doc: Option<u32>, path: &Path, take_over_own: bool) -> Acquired {
        if let Some((_, lp)) = self.holder_of(path, doc) {
            return Acquired::Busy(LockInfo::ours(path, self.identity(), Some(&lp)));
        }
        acquire(self.identity(), path, take_over_own, self.other_instance())
    }

    /// Locks `doc`'s file `path` (see [`Self::try_acquire`]); a lock taken
    /// is remembered for the document (replacing and releasing an older one).
    pub fn acquire(&self, doc: u32, path: &Path, take_over_own: bool) -> Acquired {
        let got = self.try_acquire(Some(doc), path, take_over_own);
        if let Acquired::Locked(lp) = &got {
            self.adopt(doc, lp.clone());
        }
        got
    }

    /// Files a lock taken with [`acquire`] (before the document had an id)
    /// as `doc`'s.
    pub fn adopt(&self, doc: u32, lp: PathBuf) {
        let old = lock(&self.held).insert(doc, lp.clone());
        if let Some(old) = old.filter(|o| *o != lp) {
            release(self.identity(), &old);
        }
    }

    /// `true` if `doc` holds a lock.
    pub fn holds(&self, doc: u32) -> bool {
        lock(&self.held).contains_key(&doc)
    }

    /// The lock `doc` holds, if any.
    pub fn held_path(&self, doc: u32) -> Option<PathBuf> {
        lock(&self.held).get(&doc).cloned()
    }

    /// Someone else's lock on `path` for `doc` (Save As target): another of
    /// our documents' lock, another program's, or (while another Tally of
    /// this user runs) one with our own identity.
    pub fn foreign(&self, path: &Path, doc: u32) -> Option<LockInfo> {
        if let Some((_, lp)) = self.holder_of(path, Some(doc)) {
            return Some(LockInfo::ours(path, self.identity(), Some(&lp)));
        }
        let me = self.identity();
        let lp = lock_path(path)?;
        match read_lock(&lp) {
            Ok(Some(e)) if e.same_owner(me) && !self.other_instance() => None,
            Ok(e) => Some(LockInfo::of(path, e.as_ref(), me)),
            Err(_) => None,
        }
    }

    /// Drops `doc`'s lock (close, Save As elsewhere, read-only).
    pub fn release(&self, doc: u32) {
        let lp = lock(&self.held).remove(&doc);
        if let Some(lp) = lp {
            release(self.identity(), &lp);
        }
    }

    /// Drops every lock (the app exits).
    pub fn release_all(&self) {
        let all: Vec<PathBuf> = lock(&self.held).drain().map(|(_, p)| p).collect();
        if all.is_empty() {
            return;
        }
        for lp in all {
            release(self.identity(), &lp);
        }
    }

    /// Remembers `path`'s stamp for `doc` (after an open, save or reload).
    pub fn remember(&self, doc: u32, path: &Path) {
        if let Some(s) = stamp(path) {
            lock(&self.stamps).insert(doc, s);
        }
    }

    /// `true` if `path` changed on disk since [`Self::remember`].
    pub fn changed_on_disk(&self, doc: u32, path: &Path) -> bool {
        let known = lock(&self.stamps).get(&doc).copied();
        known.is_some_and(|k| stamp(path).is_some_and(|now| now != k))
    }

    /// Forgets `doc` (closed); its lock is released.
    pub fn forget(&self, doc: u32) {
        self.release(doc);
        lock(&self.stamps).remove(&doc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under `.targets/scratch` (never the system temp
    /// dir: lock files only go there), removed when dropped.
    pub(crate) struct Scratch(PathBuf);

    impl AsRef<Path> for Scratch {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    impl std::ops::Deref for Scratch {
        type Target = PathBuf;
        fn deref(&self) -> &PathBuf {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            // Undo read-only bits a test may have left, then remove.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
            }
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn scratch(name: &str) -> Scratch {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.targets/scratch/doclock-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn me() -> Identity {
        Identity {
            name: "Ana Pérez".into(),
            sys_user: "ana".into(),
            host: "mac.local".into(),
            user_url: "file:///Users/ana/Library/Application%20Support/app.tally".into(),
        }
    }

    /// A line as AOO 4.1 writes it on macOS.
    const AOO_LINE: &[u8] = b"Robin Example,robin,Robins-MacBook-Pro.local,02.11.2026 15:04,file:///Users/robin/Library/Application%20Support/OpenOffice/4;";

    #[test]
    fn parses_an_aoo_lock_line() {
        let e = LockEntry::parse(AOO_LINE).unwrap();
        assert_eq!(e.name, "Robin Example");
        assert_eq!(e.sys_user, "robin");
        assert_eq!(e.host, "Robins-MacBook-Pro.local");
        assert_eq!(e.time, "02.11.2026 15:04");
        assert_eq!(
            e.user_url,
            "file:///Users/robin/Library/Application%20Support/OpenOffice/4"
        );
        assert_eq!(e.to_bytes(), AOO_LINE, "written back byte for byte");
        // An empty user name (no Options ▸ User Data) is allowed.
        let e = LockEntry::parse(b",robin,host,01.01.2026 09:00,;").unwrap();
        assert_eq!((e.name.as_str(), e.user_url.as_str()), ("", ""));
    }

    #[test]
    fn escapes_commas_semicolons_backslashes_and_keeps_unicode() {
        let e = LockEntry {
            name: "Doe, Jane; \\ Müller 日本".into(),
            sys_user: "jane".into(),
            host: "h".into(),
            time: "31.12.2026 23:59".into(),
            user_url: "file:///a,b;c".into(),
        };
        let bytes = e.to_bytes();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert_eq!(
            text,
            "Doe\\, Jane\\; \\\\ Müller 日本,jane,h,31.12.2026 23:59,file:///a\\,b\\;c;"
        );
        assert_eq!(LockEntry::parse(&bytes).unwrap(), e);
        assert!(!text.ends_with('\n'), "no newline, as AOO");
    }

    #[test]
    fn malformed_locks_are_rejected_as_aoo_does() {
        for bad in [
            &b""[..],
            b"a,b,c,d,e",     // no ';'
            b"a,b,c,d;e;",    // ';' too early
            b"a,b,c,d,e,",    // ',' after the last field
            b"a\\x,b,c,d,e;", // bad escape
            b"a,b,c,d,e\\",   // dangling escape
            b"a,b,c",         // too few fields
        ] {
            assert_eq!(
                LockEntry::parse(bad),
                None,
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        // Trailing bytes after the entry are ignored (AOO parses one entry).
        assert!(LockEntry::parse(b"a,b,c,d,e;\n").is_some());
        let mut huge = b"x".repeat(MAX_LOCK_BYTES);
        huge.extend_from_slice(b",b,c,d,e;");
        assert_eq!(LockEntry::parse(&huge), None);
    }

    #[test]
    fn lock_path_is_next_to_the_real_file() {
        let dir = scratch("path");
        let doc = dir.join("Budget 2026.ods");
        fs::write(&doc, b"x").unwrap();
        let lp = lock_path(&doc).unwrap();
        assert_eq!(lp.parent(), Some(fs::canonicalize(&dir).unwrap().as_path()));
        assert_eq!(lp.file_name().unwrap(), ".~lock.Budget 2026.ods#");
        // A not-yet-existing file (Save As target) too.
        let lp = lock_path(&dir.join("new.xlsx")).unwrap();
        assert_eq!(lp.file_name().unwrap(), ".~lock.new.xlsx#");
        #[cfg(unix)]
        {
            // A symlink: the lock sits next to its target (AOO ResolveLinks).
            let other = dir.join("sub");
            fs::create_dir_all(&other).unwrap();
            let link = other.join("alias.ods");
            std::os::unix::fs::symlink(&doc, &link).unwrap();
            assert_eq!(
                lock_path(&link).unwrap(),
                fs::canonicalize(&dir)
                    .unwrap()
                    .join(".~lock.Budget 2026.ods#")
            );
        }
    }

    #[test]
    fn acquire_writes_aoo_format_and_release_removes_it() {
        let dir = scratch("acquire");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let Acquired::Locked(lp) = acquire(&me(), &doc, false, false) else {
            panic!("not locked");
        };
        let e = LockEntry::parse(&fs::read(&lp).unwrap()).unwrap();
        assert_eq!(e.name, "Ana Pérez");
        assert_eq!((e.sys_user.as_str(), e.host.as_str()), ("ana", "mac.local"));
        assert_eq!(e.time.len(), 16, "DD.MM.YYYY HH:MM: {}", e.time);
        assert_eq!(&e.time[2..3], ".");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&lp).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode & 0o022, 0, "not group/world writable: {mode:o}");
        }
        // Our own lock (a crash) is taken over silently.
        assert_eq!(
            acquire(&me(), &doc, false, false),
            Acquired::Locked(lp.clone())
        );
        release(&me(), &lp);
        assert!(!lp.exists());
    }

    #[test]
    fn a_foreign_lock_is_reported_and_never_removed() {
        let dir = scratch("foreign");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let lp = lock_path(&doc).unwrap();
        fs::write(&lp, AOO_LINE).unwrap();
        let Acquired::Busy(info) = acquire(&me(), &doc, false, false) else {
            panic!("not busy");
        };
        assert_eq!(info.owner, "Robin Example");
        assert_eq!(info.time, "02.11.2026 15:04");
        assert_eq!(info.file, "Book.ods");
        assert!(!info.own);
        // "Open" may only take over a lock of our own account.
        assert!(matches!(
            acquire(&me(), &doc, true, false),
            Acquired::Busy(_)
        ));
        release(&me(), &lp);
        assert_eq!(fs::read(&lp).unwrap(), AOO_LINE, "untouched");
        let locks = Locks::default();
        locks.set_identity(me());
        assert!(locks.foreign(&doc, 1).is_some());
        // The error carries the lock for the UI.
        let err = info.error();
        assert_eq!(err.code, ApiError::LOCKED);
        let v: serde_json::Value = serde_json::from_str(&err.message).unwrap();
        assert_eq!(v["owner"], "Robin Example");
        assert_eq!(v["own"], false);
        // A malformed or non-regular lock is an unknown user's: kept too.
        fs::write(&lp, b"garbage").unwrap();
        let Acquired::Busy(info) = acquire(&me(), &doc, false, false) else {
            panic!("not busy");
        };
        assert_eq!((info.owner.as_str(), info.time.as_str()), ("", ""));
        release(&me(), &lp);
        assert_eq!(fs::read(&lp).unwrap(), b"garbage");
    }

    #[test]
    fn own_lock_elsewhere_is_asked_then_taken_over() {
        let dir = scratch("own");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let lp = lock_path(&doc).unwrap();
        // Same account, another installation (OpenOffice) or host.
        let aoo = LockEntry {
            name: "Ana".into(),
            sys_user: "ana".into(),
            host: "mac.local".into(),
            time: "01.11.2026 08:00".into(),
            user_url: "file:///Users/ana/Library/Application%20Support/OpenOffice/4".into(),
        };
        fs::write(&lp, aoo.to_bytes()).unwrap();
        let Acquired::Busy(info) = acquire(&me(), &doc, false, false) else {
            panic!("not busy");
        };
        assert!(info.own);
        assert_eq!(info.time, "01.11.2026 08:00");
        // "Open": the user chose to ignore their own lock (AOO overwrites it).
        assert_eq!(
            acquire(&me(), &doc, true, false),
            Acquired::Locked(lp.clone())
        );
        let now = LockEntry::parse(&fs::read(&lp).unwrap()).unwrap();
        assert!(now.same_owner(&me()));
        release(&me(), &lp);
        assert!(!lp.exists());
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_directory_means_no_lock_and_no_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("rodir");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let got = acquire(&me(), &doc, false, false);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        // Root can write anywhere; then the lock simply works.
        if let Acquired::Locked(lp) = &got {
            release(&me(), lp);
        } else {
            assert_eq!(got, Acquired::Unlocked);
        }
        assert!(!lock_path(&doc).unwrap().exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_lock_path_is_never_followed() {
        let dir = scratch("link");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let victim = dir.join("victim.txt");
        fs::write(&victim, b"keep").unwrap();
        let lp = lock_path(&doc).unwrap();
        std::os::unix::fs::symlink(&victim, &lp).unwrap();
        // Busy (an unknown lock), the target untouched and not read.
        assert!(matches!(
            acquire(&me(), &doc, false, false),
            Acquired::Busy(_)
        ));
        release(&me(), &lp);
        assert_eq!(fs::read(&victim).unwrap(), b"keep");
        assert!(fs::symlink_metadata(&lp).unwrap().file_type().is_symlink());
    }

    #[test]
    fn writability_follows_permissions() {
        let dir = scratch("writable");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        assert!(is_writable(&doc));
        let mut p = fs::metadata(&doc).unwrap().permissions();
        p.set_readonly(true);
        fs::set_permissions(&doc, p).unwrap();
        assert!(!is_writable(&doc));
        assert!(!is_writable(&dir), "a directory is not a document");
        assert!(!is_writable(&dir.join("missing.ods")));
        let mut p = fs::metadata(&doc).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        p.set_readonly(false);
        fs::set_permissions(&doc, p).unwrap();
        assert_eq!(fs::read(&doc).unwrap(), b"x", "nothing written");
    }

    /// Finder's "Locked" checkbox is the `uchg` flag: the file is not
    /// writable even though its permissions say so.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_finder_locked_file_is_not_writable() {
        let dir = scratch("uchg");
        let doc = dir.join("Locked.ods");
        fs::write(&doc, b"x").unwrap();
        let flag = |f: &str| {
            std::process::Command::new("/usr/bin/chflags")
                .arg(f)
                .arg(&doc)
                .status()
                .is_ok_and(|s| s.success())
        };
        assert!(flag("uchg"));
        let writable = is_writable(&doc);
        assert!(flag("nouchg"));
        assert!(!writable);
        assert!(is_writable(&doc));
    }

    #[test]
    fn locks_track_documents_and_stamps() {
        let dir = scratch("locks");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let locks = Locks::default();
        locks.set_identity(me());
        assert!(matches!(locks.acquire(7, &doc, false), Acquired::Locked(_)));
        assert!(locks.holds(7));
        // Save As elsewhere: the new lock replaces (and removes) the old one.
        let other = dir.join("Other.ods");
        fs::write(&other, b"y").unwrap();
        let old = locks.held_path(7).unwrap();
        assert!(matches!(
            locks.acquire(7, &other, false),
            Acquired::Locked(_)
        ));
        assert!(!old.exists());
        locks.remember(7, &other);
        assert!(!locks.changed_on_disk(7, &other));
        fs::write(&other, b"changed elsewhere").unwrap();
        assert!(locks.changed_on_disk(7, &other));
        locks.forget(7);
        assert!(!locks.holds(7));
        assert!(!lock_path(&other).unwrap().exists());
    }

    #[test]
    fn file_urls_are_percent_encoded() {
        assert_eq!(
            file_url(Path::new(
                "/Users/ana/Library/Application Support/app.tally"
            )),
            "file:///Users/ana/Library/Application%20Support/app.tally"
        );
    }

    #[test]
    fn bidi_and_format_characters_are_stripped_and_owner_is_capped() {
        let dir = scratch("bidi");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let evil = LockEntry {
            name: format!(
                "Eve\u{202E}gnp.exe\u{2066}\u{200B}\u{FEFF}{}",
                "x".repeat(200)
            ),
            sys_user: "eve".into(),
            host: "h".into(),
            time: "01.01.2026\u{200F} 10:00".into(),
            user_url: String::new(),
        };
        fs::write(lock_path(&doc).unwrap(), evil.to_bytes()).unwrap();
        let Acquired::Busy(info) = acquire(&me(), &doc, false, false) else {
            panic!("not busy");
        };
        assert!(info.owner.starts_with("Evegnp.exex"), "{}", info.owner);
        assert!(info.owner.chars().all(|c| !is_format_char(c)));
        assert_eq!(info.owner.chars().count(), MAX_OWNER);
        assert_eq!(info.time, "01.01.2026 10:00");
        for c in ['\u{061C}', '\u{200E}', '\u{202A}', '\u{2069}', '\u{E0041}'] {
            assert!(is_format_char(c), "{c:?}");
        }
        assert!(!is_format_char('é') && !is_format_char('日'));
    }

    #[test]
    fn an_unknown_account_never_owns_a_lock() {
        let nobody = Identity {
            sys_user: String::new(),
            ..me()
        };
        let empty = LockEntry {
            sys_user: String::new(),
            host: nobody.host.clone(),
            user_url: nobody.user_url.clone(),
            ..LockEntry::default()
        };
        assert!(!empty.same_owner(&nobody));
        assert!(!empty.same_user(&nobody));
        let dir = scratch("nobody");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let lp = lock_path(&doc).unwrap();
        fs::write(&lp, empty.to_bytes()).unwrap();
        // Neither taken over nor removed.
        assert!(matches!(
            acquire(&nobody, &doc, true, false),
            Acquired::Busy(_)
        ));
        release(&nobody, &lp);
        assert!(lp.exists());
    }

    #[test]
    fn a_lock_held_by_another_document_is_busy() {
        let dir = scratch("held");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let locks = Locks::default();
        locks.set_identity(me());
        assert!(matches!(locks.acquire(1, &doc, false), Acquired::Locked(_)));
        // Document 2 may not take document 1's lock, not even with "Open".
        let Acquired::Busy(info) = locks.acquire(2, &doc, true) else {
            panic!("not busy");
        };
        assert_eq!(info.owner, "Ana Pérez");
        assert!(!info.own);
        assert!(locks.foreign(&doc, 2).is_some());
        assert!(locks.foreign(&doc, 1).is_none());
        assert!(!locks.holds(2));
        // Re-acquiring its own lock is fine.
        assert!(matches!(locks.acquire(1, &doc, false), Acquired::Locked(_)));
        locks.forget(1);
        assert!(!lock_path(&doc).unwrap().exists());
    }

    #[test]
    fn reservations_are_exclusive_until_dropped() {
        let locks = Locks::default();
        let p = Path::new("/x/Book.ods");
        let first = locks.reserve(p);
        assert!(first.is_some());
        assert!(locks.reserve(p).is_none());
        drop(first);
        assert!(locks.reserve(p).is_some());
    }

    #[test]
    fn the_instance_lock_detects_another_tally() {
        let dir = scratch("instance");
        let first = instance_lock(&dir);
        assert!(first.is_some());
        // A second Tally (another open file description) cannot take it.
        assert!(instance_lock(&dir).is_none());
        drop(first);
        // Retried: a child process another test is spawning may briefly
        // share the descriptor between fork and exec.
        let again = (0..100).find_map(|_| {
            instance_lock(&dir).or_else(|| {
                std::thread::sleep(std::time::Duration::from_millis(10));
                None
            })
        });
        assert!(again.is_some());
        drop(again);

        // While another Tally runs, our own identity's lock may be its live
        // lock: asked about ("locked by yourself"), not taken over.
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let lp = lock_path(&doc).unwrap();
        fs::write(&lp, me().entry().to_bytes()).unwrap();
        let locks = Locks::default();
        locks.set_identity(me());
        locks.set_other_instance(true);
        let Acquired::Busy(info) = locks.acquire(1, &doc, false) else {
            panic!("not busy");
        };
        assert!(info.own);
        assert!(
            locks.foreign(&doc, 1).is_some(),
            "Save As onto it is refused too"
        );
        // The user's explicit "Open" still takes it.
        assert!(matches!(locks.acquire(1, &doc, true), Acquired::Locked(_)));
        locks.forget(1);
    }

    #[test]
    fn release_restores_a_lock_that_is_not_ours() {
        let dir = scratch("restore");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let lp = lock_path(&doc).unwrap();
        fs::write(&lp, AOO_LINE).unwrap();
        release(&me(), &lp);
        assert_eq!(fs::read(&lp).unwrap(), AOO_LINE);
        let names: Vec<_> = fs::read_dir(&*dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.contains(".tally-")), "{names:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_lock_does_not_block() {
        let dir = scratch("fifo");
        let doc = dir.join("Book.ods");
        fs::write(&doc, b"x").unwrap();
        let lp = lock_path(&doc).unwrap();
        let made = std::process::Command::new("/usr/bin/mkfifo")
            .arg(&lp)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return;
        }
        assert_eq!(read_lock(&lp).unwrap(), None);
        assert!(matches!(
            acquire(&me(), &doc, false, false),
            Acquired::Busy(_)
        ));
    }
}
