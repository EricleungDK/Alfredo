//! Endpoint-shared client admission. Owner locks are live eligibility, never saved
//! permission to replay inference. Releasing a slot does not prove server-side abort.
use crate::tasks::StoreLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const MAX_TICKETS: usize = 256;
const MAX_BYTES: usize = 512 * 1024;
const MAX_FILES: usize = 576;
static IDS: AtomicU64 = AtomicU64::new(0);
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    Foreground,
    Background,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    /// Current projected grant order, including this waiter and excluding active slots.
    pub position: usize,
    pub waiting: usize,
    pub active: usize,
    pub capacity: usize,
    pub class: Class,
}
#[derive(Clone, Debug)]
pub struct Coordinator {
    origin: String,
    capacity: usize,
    root: PathBuf,
    namespace: String,
}
pub struct Queue {
    coordinator: Coordinator,
    class: Class,
    ticket: Option<Ticket>,
    observation: Option<Observation>,
    completed: bool,
}
pub struct Permit {
    _ticket: Ticket,
}
struct Ticket {
    coordinator: Coordinator,
    id: String,
    owner: Option<StoreLock>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum State {
    Waiting,
    Active,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    sequence: u64,
    owner_device: u64,
    owner_inode: u64,
    class: Class,
    state: State,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    schema_version: u32,
    origin: String,
    capacity: usize,
    next_sequence: u64,
    foreground_streak: u8,
    entries: Vec<Entry>,
}

impl Coordinator {
    /// Pure construction. The host-wide directory is independent of TMPDIR and mission state.
    pub fn new(normalized_origin: String, capacity: usize) -> Result<Self> {
        let url =
            reqwest::Url::parse(&normalized_origin).map_err(|_| "Invalid inference origin")?;
        if !(1..=8).contains(&capacity)
            || normalized_origin.len() > 2048
            || !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || url.origin().ascii_serialization() != normalized_origin
        {
            return Err(
                "Inference admission requires a normalized HTTP(S) origin and capacity 1–8".into(),
            );
        }
        Ok(Self {
            namespace: format!("{:x}", Sha256::digest(normalized_origin.as_bytes())),
            origin: normalized_origin,
            capacity,
            root: PathBuf::from(format!("/tmp/alfredo-inference-{}", uid()?)),
        })
    }
    pub fn queue(&self, class: Class) -> Queue {
        Queue {
            coordinator: self.clone(),
            class,
            ticket: None,
            observation: None,
            completed: false,
        }
    }
    pub async fn acquire(&self, class: Class) -> Result<Permit> {
        let mut queue = self.queue(class);
        loop {
            if let Some(permit) = queue.poll()? {
                return Ok(permit);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    fn directory(&self) -> Result<PathBuf> {
        let parent = self
            .root
            .parent()
            .ok_or("Invalid inference admission root")?
            .canonicalize()
            .map_err(|e| format!("Inference admission parent unavailable: {e}"))?;
        check_parent(&parent)?;
        let root = parent.join(self.root.file_name().ok_or("Invalid inference root name")?);
        private_directory(&root)?;
        let namespace = root.join(&self.namespace);
        private_directory(&namespace)?;
        Ok(namespace)
    }
    fn transaction(&self) -> Result<Option<(PathBuf, StoreLock)>> {
        let directory = self.directory()?;
        let file = private_file(&directory.join("ledger.lock"), true, false)?;
        match file.try_lock() {
            Ok(()) => Ok(Some((directory, StoreLock(file)))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(error) => Err(format!("Inference admission lock failed: {error}")),
        }
    }
    fn empty(&self) -> Ledger {
        Ledger {
            schema_version: 1,
            origin: self.origin.clone(),
            capacity: self.capacity,
            next_sequence: 1,
            foreground_streak: 0,
            entries: vec![],
        }
    }
    fn read(&self, directory: &Path) -> Result<Ledger> {
        let path = directory.join("ledger.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(self.empty()),
            Err(e) => return Err(format!("Cannot inspect inference ledger: {e}")),
            Ok(_) => {}
        }
        let mut bytes = vec![];
        private_file(&path, false, false)?
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Inference admission ledger exceeds bounds; preserved".into());
        }
        let ledger: Ledger = serde_json::from_slice(&bytes)
            .map_err(|_| "Malformed inference admission ledger; preserved")?;
        let mut ids = std::collections::BTreeSet::new();
        let mut sequences = std::collections::BTreeSet::new();
        if ledger.schema_version != 1
            || ledger.origin != self.origin
            || !(1..=8).contains(&ledger.capacity)
            || ledger.foreground_streak > 3
            || ledger.entries.len() > MAX_TICKETS
            || ledger.next_sequence == 0
            || ledger
                .entries
                .iter()
                .filter(|e| e.state == State::Active)
                .count()
                > ledger.capacity
            || ledger.entries.iter().any(|e| {
                !valid_id(&e.id)
                    || !ids.insert(&e.id)
                    || e.sequence == 0
                    || e.sequence >= ledger.next_sequence
                    || !sequences.insert(e.sequence)
            })
        {
            return Err("Invalid inference admission ledger; preserved".into());
        }
        Ok(ledger)
    }
    fn release(&self, id: &str) -> Result<()> {
        let Some((directory, _lock)) = self.transaction()? else {
            return Ok(());
        };
        let mut ledger = self.read(&directory)?;
        let before = ledger.clone();
        let garbage = reap(&directory, &mut ledger)?;
        if ledger.entries.iter().any(|entry| entry.id == id) {
            return Err("Inference owner was not released".into());
        }
        if ledger != before {
            publish(&directory, &ledger)?;
        }
        discard(garbage)?;
        Ok(())
    }
}
impl Queue {
    pub fn observation(&self) -> Option<Observation> {
        self.observation
    }
    /// All IO is bounded and global lock acquisition is nonblocking. No lock spans an await.
    pub fn poll(&mut self) -> Result<Option<Permit>> {
        if self.completed {
            return Err("Inference admission queue has already completed".into());
        }
        let Some((directory, _lock)) = self.coordinator.transaction()? else {
            return Ok(None);
        };
        let mut ledger = self.coordinator.read(&directory)?;
        let before = ledger.clone();
        let garbage = reap(&directory, &mut ledger)?;
        if ledger.capacity != self.coordinator.capacity {
            if !ledger.entries.is_empty() {
                return Err(format!(
                    "Shared inference capacity conflict: live endpoint uses {}, requested {}",
                    ledger.capacity, self.coordinator.capacity
                ));
            }
            ledger.capacity = self.coordinator.capacity;
            ledger.foreground_streak = 0;
        }
        if self.ticket.is_none() {
            if ledger.entries.len() >= MAX_TICKETS {
                return Err("Shared inference queue is full".into());
            }
            if ledger.entries.is_empty() {
                ledger.next_sequence = 1;
                ledger.foreground_streak = 0;
            }
            let sequence = ledger.next_sequence;
            ledger.next_sequence = sequence
                .checked_add(1)
                .ok_or("Inference admission sequence exhausted; wait for the queue to drain")?;
            let id = format!(
                "owner-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                IDS.fetch_add(1, Ordering::Relaxed)
            );
            let owner = private_file(&directory.join(&id), false, true)?;
            owner
                .try_lock()
                .map_err(|_| "New inference owner could not be locked")?;
            let (owner_device, owner_inode) = file_identity(&owner)?;
            self.ticket = Some(Ticket {
                coordinator: self.coordinator.clone(),
                id: id.clone(),
                owner: Some(StoreLock(owner)),
            });
            ledger.entries.push(Entry {
                id,
                sequence,
                owner_device,
                owner_inode,
                class: self.class,
                state: State::Waiting,
            });
        }
        let id = &self.ticket.as_ref().unwrap().id;
        if !ledger.entries.iter().any(|entry| &entry.id == id) {
            return Err(
                "Live inference admission disappeared; request refused without replay".into(),
            );
        }
        while ledger
            .entries
            .iter()
            .filter(|e| e.state == State::Active)
            .count()
            < ledger.capacity
        {
            let Some(index) = choose(&ledger.entries, &mut ledger.foreground_streak) else {
                break;
            };
            ledger.entries[index].state = State::Active;
        }
        let active = ledger
            .entries
            .iter()
            .filter(|e| e.state == State::Active)
            .count();
        let mut projected = ledger.entries.clone();
        let waiting = projected.len() - active;
        let mut streak = ledger.foreground_streak;
        let mut position = 0;
        while let Some(index) = choose(&projected, &mut streak) {
            position += 1;
            if &projected[index].id == id {
                break;
            }
            projected[index].state = State::Active;
        }
        let observation = if ledger
            .entries
            .iter()
            .any(|e| &e.id == id && e.state == State::Waiting)
        {
            Some(Observation {
                position,
                waiting,
                active,
                capacity: ledger.capacity,
                class: self.class,
            })
        } else {
            None
        };
        if ledger != before {
            publish(&directory, &ledger)?;
        }
        discard(garbage)?;
        self.observation = observation;
        if ledger
            .entries
            .iter()
            .any(|e| &e.id == id && e.state == State::Active)
        {
            self.completed = true;
            return Ok(Some(Permit {
                _ticket: self.ticket.take().unwrap(),
            }));
        }
        Ok(None)
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        // Explicit unlock precedes best-effort metadata cleanup. Another scheduler
        // removes the orphan even when cleanup cannot take the transaction lock.
        drop(self.owner.take());
        let _ = self.coordinator.release(&self.id);
    }
}
fn valid_id(id: &str) -> bool {
    id.starts_with("owner-")
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'-' || b.is_ascii_lowercase())
}
fn choose(entries: &[Entry], streak: &mut u8) -> Option<usize> {
    let oldest = |class| {
        entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.class == class && e.state == State::Waiting)
            .min_by_key(|(_, e)| e.sequence)
            .map(|(i, _)| i)
    };
    match (oldest(Class::Foreground), oldest(Class::Background)) {
        (Some(foreground), Some(_)) if *streak < 3 => {
            *streak += 1;
            Some(foreground)
        }
        (_, Some(background)) => {
            *streak = 0;
            Some(background)
        }
        (Some(foreground), None) => {
            *streak = 0;
            Some(foreground)
        }
        (None, None) => None,
    }
}
fn reap(directory: &Path, ledger: &mut Ledger) -> Result<Vec<PathBuf>> {
    let mut live = std::collections::BTreeSet::new();
    // Validate/scan the bounded directory before removing any abandoned owner.
    let mut owners = Vec::new();
    let mut temporaries = Vec::new();
    for (index, entry) in fs::read_dir(directory)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if index >= MAX_FILES {
            return Err("Inference admission directory exceeds bounds".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Invalid inference admission filename")?;
        if valid_id(&name) {
            owners.push((name, entry.path()));
        } else if name.starts_with(".ledger-") && name.ends_with(".tmp") {
            temporaries.push(entry.path());
        } else if name != "ledger.json" && name != "ledger.lock" {
            return Err("Unexpected inference admission entry; preserved".into());
        }
    }
    let names: std::collections::BTreeSet<_> =
        owners.iter().map(|(name, _)| name.as_str()).collect();
    if ledger
        .entries
        .iter()
        .any(|entry| !names.contains(entry.id.as_str()))
    {
        return Err("Inference owner proof is missing; shared capacity is unconfirmed and new requests are refused".into());
    }
    let mut garbage = Vec::new();
    for (name, path) in owners {
        let owner = private_file(&path, false, false)?;
        if let Some(entry) = ledger.entries.iter().find(|entry| entry.id == name) {
            if file_identity(&owner)? != (entry.owner_device, entry.owner_inode) {
                return Err("Inference owner identity changed; shared capacity is unconfirmed and new requests are refused".into());
            }
        }
        match owner.try_lock() {
            Ok(()) => {
                let _guard = StoreLock(owner);
                garbage.push(path);
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                live.insert(name);
            }
            Err(error) => return Err(format!("Cannot inspect inference owner: {error}")),
        }
    }
    if live
        .iter()
        .any(|id| !ledger.entries.iter().any(|entry| &entry.id == id))
    {
        return Err("Live inference owner has no saved admission; shared capacity is unconfirmed and new requests are refused".into());
    }
    for path in temporaries {
        let _file = private_file(&path, false, false)?;
        garbage.push(path);
    }
    ledger.entries.retain(|entry| live.contains(&entry.id));
    Ok(garbage)
}
fn discard(garbage: Vec<PathBuf>) -> Result<()> {
    // Called only after the ledger no longer references these unlocked owners.
    // A crash before unlink leaves safe, unowned files for the next transaction.
    for path in garbage {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn publish(directory: &Path, ledger: &Ledger) -> Result<()> {
    let bytes = serde_json::to_vec(ledger).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Inference admission ledger exceeds bounds".into());
    }
    let path = directory.join(format!(
        ".ledger-{}-{}.tmp",
        std::process::id(),
        IDS.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = private_file(&path, false, true)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("Cannot save inference admission: {e}"))?;
        fs::rename(&path, directory.join("ledger.json")).map_err(|e| e.to_string())?;
        File::open(directory)
            .and_then(|f| f.sync_all())
            .map_err(|e| format!("Inference admission publication unconfirmed: {e}"))
    })();
    let _ = fs::remove_file(path);
    result
}
#[cfg(unix)]
fn uid() -> Result<u32> {
    Ok(unsafe { libc::geteuid() })
}
#[cfg(not(unix))]
fn uid() -> Result<u32> {
    Err("Shared inference admission requires a supported Unix host".into())
}
#[cfg(unix)]
fn check_parent(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || (metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0)
    {
        return Err("Unsafe inference admission parent directory".into());
    }
    Ok(())
}
#[cfg(not(unix))]
fn check_parent(_: &Path) -> Result<()> {
    Err("Shared inference admission requires a supported Unix host".into())
}
fn private_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::DirBuilder::new().mode(0o700).create(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(format!(
                            "Cannot create inference admission directory: {error}"
                        ))
                    }
                }
            }
            Err(error) => return Err(error.to_string()),
            Ok(_) => {}
        }
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != uid()?
            || metadata.mode() & 0o777 != 0o700
        {
            return Err("Inference admission directory must be owned by this user with mode 0700 and no symlink".into());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("Shared inference admission requires a supported Unix host".into())
    }
}
fn file_identity(file: &File) -> Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Err("Shared inference admission requires a supported Unix host".into())
    }
}
fn private_file(path: &Path, create: bool, exclusive: bool) -> Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        match fs::symlink_metadata(path) {
            Ok(metadata)
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.uid() != uid()?
                    || metadata.mode() & 0o777 != 0o600 =>
            {
                return Err(
                    "Inference admission entry must be an owned mode-0600 regular file; preserved"
                        .into(),
                )
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error.to_string())
            }
            _ => {}
        }
        let file = OpenOptions::new()
            .read(true)
            .write(create || exclusive)
            .create(create)
            .create_new(exclusive)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| format!("Cannot open inference admission entry: {e}"))?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.uid() != uid()? || metadata.mode() & 0o777 != 0o600 {
            return Err("Unsafe inference admission file identity".into());
        }
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, create, exclusive);
        Err("Shared inference admission requires a supported Unix host".into())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    struct Fixture {
        coordinator: Coordinator,
    }
    impl Fixture {
        fn new(capacity: usize) -> Self {
            let mut coordinator =
                Coordinator::new("http://127.0.0.1:11434".into(), capacity).unwrap();
            coordinator.root = PathBuf::from(format!(
                "/tmp/alfredo-admission-unit-{}-{}",
                std::process::id(),
                IDS.fetch_add(1, Ordering::Relaxed)
            ));
            Self { coordinator }
        }
        fn queue(&self, class: Class) -> Queue {
            self.coordinator.queue(class)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.coordinator.root);
        }
    }
    #[test]
    fn construction_is_inert_and_normalized_endpoint_identity_is_exact() {
        let f = Fixture::new(2);
        let _queue = f.queue(Class::Foreground);
        assert!(!f.coordinator.root.exists());
        for origin in [
            "http://LOCALHOST:11434",
            "http://localhost:11434/",
            "http://localhost:80",
            "http://localhost/path",
            "http://user@localhost",
            "http://localhost?secret",
        ] {
            assert!(Coordinator::new(origin.into(), 2).is_err(), "{origin}");
        }
        let localhost = Coordinator::new("http://localhost:11434".into(), 2).unwrap();
        assert_ne!(localhost.namespace, f.coordinator.namespace);
        assert!(Coordinator::new("http://localhost".into(), 0).is_err());
        assert!(Coordinator::new("http://localhost".into(), 9).is_err());
    }
    #[test]
    fn independent_coordinators_share_capacity_and_conflicts_only_clear_after_drain() {
        let f = Fixture::new(1);
        let mut first = f.queue(Class::Foreground);
        let first = first.poll().unwrap().unwrap();
        let mut another = f.coordinator.clone();
        another.capacity = 2;
        assert!(another
            .queue(Class::Foreground)
            .poll()
            .err()
            .unwrap()
            .contains("conflict"));
        let mut waiting = f.queue(Class::Background);
        assert!(waiting.poll().unwrap().is_none());
        assert_eq!(
            waiting.observation().unwrap(),
            Observation {
                position: 1,
                waiting: 1,
                active: 1,
                capacity: 1,
                class: Class::Background
            }
        );
        drop(first);
        assert!(another
            .queue(Class::Foreground)
            .poll()
            .err()
            .unwrap()
            .contains("conflict"));
        let waiting = waiting.poll().unwrap().unwrap();
        drop(waiting);
        let mut next = another.queue(Class::Foreground);
        let next = next.poll().unwrap().unwrap();
        let mut simultaneous = another.queue(Class::Foreground);
        let simultaneous = simultaneous.poll().unwrap().unwrap();
        drop(next);
        drop(simultaneous);
    }
    #[test]
    fn foreground_priority_is_bounded_and_fifo_within_each_class() {
        let f = Fixture::new(1);
        let occupied = f.queue(Class::Foreground).poll().unwrap().unwrap();
        let mut background = f.queue(Class::Background);
        assert!(background.poll().unwrap().is_none());
        let mut second_background = f.queue(Class::Background);
        assert!(second_background.poll().unwrap().is_none());
        let mut foreground: Vec<_> = (0..4)
            .map(|_| {
                let mut queue = f.queue(Class::Foreground);
                assert!(queue.poll().unwrap().is_none());
                queue
            })
            .collect();
        assert!(background.poll().unwrap().is_none());
        assert_eq!(background.observation().unwrap().position, 4);
        drop(occupied);
        for queue in foreground.iter_mut().take(3) {
            assert!(background.poll().unwrap().is_none());
            assert!(second_background.poll().unwrap().is_none());
            let permit = queue.poll().unwrap().unwrap();
            drop(permit);
        }
        assert!(foreground[3].poll().unwrap().is_none());
        let background = background.poll().unwrap().unwrap();
        drop(background);
        assert!(second_background.poll().unwrap().is_none());
        let foreground = foreground[3].poll().unwrap().unwrap();
        drop(foreground);
        assert!(second_background.poll().unwrap().is_some());
    }
    #[test]
    fn cancellation_removes_eligibility_even_when_cleanup_cannot_take_global_lock() {
        let f = Fixture::new(1);
        let occupied = f.queue(Class::Foreground).poll().unwrap().unwrap();
        let mut cancelled = f.queue(Class::Background);
        assert!(cancelled.poll().unwrap().is_none());
        let id = cancelled.ticket.as_ref().unwrap().id.clone();
        let (directory, lock) = f.coordinator.transaction().unwrap().unwrap();
        let mut blocked = f.queue(Class::Foreground);
        assert!(blocked.poll().unwrap().is_none());
        assert!(blocked.observation().is_none());
        drop(cancelled);
        assert!(f
            .coordinator
            .read(&directory)
            .unwrap()
            .entries
            .iter()
            .any(|e| e.id == id));
        drop(lock);
        drop(occupied);
        let permit = blocked.poll().unwrap().unwrap();
        let entries = f.coordinator.read(&directory).unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert!(entries.iter().all(|e| e.id != id));
        drop(permit);
    }
    #[test]
    fn cancelling_a_reserved_grant_before_poll_releases_it_without_replay() {
        let f = Fixture::new(1);
        let occupied = f.queue(Class::Foreground).poll().unwrap().unwrap();
        let mut cancelled = f.queue(Class::Foreground);
        assert!(cancelled.poll().unwrap().is_none());
        let mut next = f.queue(Class::Foreground);
        assert!(next.poll().unwrap().is_none());
        drop(occupied);
        // Another caller schedules the oldest ticket; it still owns no HTTP request.
        assert!(next.poll().unwrap().is_none());
        drop(cancelled);
        assert!(next.poll().unwrap().is_some());
    }
    #[test]
    fn corrupt_ledger_and_unsafe_directory_are_preserved_and_fail_closed() {
        let f = Fixture::new(1);
        let permit = f.queue(Class::Foreground).poll().unwrap().unwrap();
        drop(permit);
        let directory = f.coordinator.directory().unwrap();
        let ledger = directory.join("ledger.json");
        fs::write(&ledger, b"{broken").unwrap();
        assert!(f.queue(Class::Foreground).poll().is_err());
        assert_eq!(fs::read(&ledger).unwrap(), b"{broken");
        fs::remove_file(&ledger).unwrap();
        symlink(directory.join("absent"), &ledger).unwrap();
        assert!(f.queue(Class::Foreground).poll().is_err());
        assert!(fs::symlink_metadata(&ledger)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_file(&ledger).unwrap();
        fs::set_permissions(&f.coordinator.root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(f.queue(Class::Foreground).poll().is_err());
        assert!(!ledger.exists());
    }
    #[test]
    fn removing_live_owner_or_ledger_proof_never_reopens_capacity() {
        for remove_ledger in [false, true] {
            let f = Fixture::new(1);
            let permit = f.queue(Class::Foreground).poll().unwrap().unwrap();
            let directory = f.coordinator.directory().unwrap();
            let path = if remove_ledger {
                directory.join("ledger.json")
            } else {
                directory.join(&permit._ticket.id)
            };
            let bytes = fs::read(&path).unwrap();
            fs::remove_file(&path).unwrap();
            assert!(f.queue(Class::Foreground).poll().is_err());
            assert!(!path.exists());
            // Recreating an owner filename cannot restore the still-open
            // descriptor's identity or ownership proof.
            let mut file = private_file(&path, false, true).unwrap();
            file.write_all(&bytes).unwrap();
            if !remove_ledger {
                assert!(f.queue(Class::Foreground).poll().is_err());
            }
            drop(permit);
        }
    }
    #[test]
    fn interrupted_owner_cleanup_retains_proof_until_ledger_removal_is_published() {
        let f = Fixture::new(1);
        let mut queue = f.queue(Class::Foreground);
        let permit = queue.poll().unwrap().unwrap();
        let directory = f.coordinator.directory().unwrap();
        let id = permit._ticket.id.clone();
        let (_directory, lock) = f.coordinator.transaction().unwrap().unwrap();
        drop(permit); // unlock but cannot publish cleanup while our transaction is held
        let original = fs::read(directory.join("ledger.json")).unwrap();
        let mut ledger = f.coordinator.read(&directory).unwrap();
        let garbage = reap(&directory, &mut ledger).unwrap();
        assert!(ledger.entries.is_empty());
        assert!(directory.join(&id).exists());
        assert_eq!(fs::read(directory.join("ledger.json")).unwrap(), original);
        // Simulate death before publishing or discarding this prepared cleanup.
        drop(garbage);
        drop(lock);
        assert!(f.queue(Class::Foreground).poll().unwrap().is_some());
        assert!(!directory.join(id).exists());
    }
    #[test]
    fn bounded_queue_refuses_without_erasing_live_owners_or_adding_a_ticket() {
        let f = Fixture::new(1);
        let directory = f.coordinator.directory().unwrap();
        let mut ledger = f.coordinator.empty();
        let mut owners = Vec::new();
        for index in 0..MAX_TICKETS {
            let id = format!("owner-capacity-{index}");
            let owner = private_file(&directory.join(&id), false, true).unwrap();
            owner.try_lock().unwrap();
            let (owner_device, owner_inode) = file_identity(&owner).unwrap();
            owners.push(StoreLock(owner));
            ledger.entries.push(Entry {
                id,
                sequence: index as u64 + 1,
                owner_device,
                owner_inode,
                class: Class::Background,
                state: if index == 0 {
                    State::Active
                } else {
                    State::Waiting
                },
            });
        }
        ledger.next_sequence = MAX_TICKETS as u64 + 1;
        publish(&directory, &ledger).unwrap();
        let before = fs::read(directory.join("ledger.json")).unwrap();
        assert!(f
            .queue(Class::Foreground)
            .poll()
            .err()
            .unwrap()
            .contains("full"));
        assert_eq!(fs::read(directory.join("ledger.json")).unwrap(), before);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), MAX_TICKETS + 2);
        drop(owners);
        assert!(f.queue(Class::Foreground).poll().unwrap().is_some());
    }
    #[test]
    fn symlink_root_is_refused_before_creating_endpoint_state() {
        let f = Fixture::new(1);
        let other = f.coordinator.root.with_extension("other");
        fs::create_dir(&other).unwrap();
        symlink(&other, &f.coordinator.root).unwrap();
        assert!(f.queue(Class::Foreground).poll().is_err());
        assert!(fs::read_dir(&other).unwrap().next().is_none());
        fs::remove_file(&f.coordinator.root).unwrap();
        fs::remove_dir(other).unwrap();
    }
}
