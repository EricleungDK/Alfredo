//! `/undo` (issue #325): restore the files the last chat turn's approved `write_file` /
//! `edit_file` calls changed, from pre-image copies kept in the State Directory.
//!
//! - Owner-typed slash command only: there is no model tool, card or approval path.
//! - Scope: the last turn's edits (a new turn forgets the older ones). No `run_command`
//!   coverage, no Git; the repository's `.git` and the workspace gain no file.
//! - Pre-images and the journal live in `<state dir>/chat-undo/` (private mode). A file
//!   over [`MAX_PREIMAGE`] bytes (or past [`MAX_TURN_BYTES`] for the turn) is never
//!   truncated or copied: undo names it as not available.
//! - Secret and Git paths cannot be edited by chat tools ([`super::excluded`]), so no
//!   pre-image holds one; the same check runs again before anything is undone.
//! - Per file, undo acts only while the file still equals the post-image recorded when
//!   the edit was applied. The check runs on the temp file's way to the atomic rename
//!   (restore) or on the file moved aside (delete of a created file). A changed file, a
//!   symlink target or parent, a read-only file or an unreadable one is refused by name.
//! - A file restored or deleted leaves the journal, so a second `/undo` never re-applies
//!   stale data; a refused one stays and can be retried.
use super::approval::{Card, CardKind, Log, Outcome};
use super::edit::{refuse_read_only, write_atomic, MAX_FILE};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Directory (in the State Directory) holding the journal and pre-image copies.
const DIR: &str = "chat-undo";
const JOURNAL: &str = "journal.json";
/// Largest file whose pre-image is kept.
pub const MAX_PREIMAGE: usize = 256 * 1024;
/// Most pre-image bytes kept for one turn.
const MAX_TURN_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    turn: String,
    /// Canonical Coding Workspace the edits were made in.
    workspace: String,
    next: u32,
    bytes: usize,
    entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Entry {
    /// Workspace-relative path.
    path: String,
    /// The turn made the file: undo deletes it.
    created: bool,
    /// Pre-image copy name and its SHA-256; none for a create or an unavailable file.
    #[serde(default)]
    before: Option<(String, String)>,
    /// SHA-256 of the file as the last approved edit left it.
    after: String,
    /// Why no undo is possible for this file.
    #[serde(default)]
    unavailable: Option<String>,
}

/// What `/undo` would need about one file, read just before its approved edit runs.
pub struct Capture {
    target: PathBuf,
    path: String,
    state: Before,
}

enum Before {
    Image(Vec<u8>),
    Created,
    Unavailable(String),
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A fresh identity for one chat turn.
pub fn new_turn() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "{now:x}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

/// Hold the journal lock while `body` runs. Best effort across threads and processes.
fn locked<T>(dir: &Path, body: impl FnOnce() -> T) -> Option<T> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("chat-undo.lock"))
        .ok()?;
    lock.lock().ok()?;
    Some(body())
}

fn journal_dir(dir: &Path) -> PathBuf {
    dir.join(DIR)
}

fn discard(dir: &Path) {
    let path = journal_dir(dir);
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.is_dir() => {
            let _ = std::fs::remove_dir_all(&path);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(&path);
        }
        Err(_) => {}
    }
}

/// A new turn starts: the previous turn's pre-images are forgotten.
pub fn begin_turn(dir: &Path) {
    if std::fs::symlink_metadata(journal_dir(dir)).is_ok() {
        locked(dir, || discard(dir));
    }
}

fn load(dir: &Path) -> Option<Journal> {
    let path = journal_dir(dir).join(JOURNAL);
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn private_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn save(dir: &Path, journal: &Journal) -> std::io::Result<()> {
    let folder = journal_dir(dir);
    let temporary = folder.join(format!("journal.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&temporary);
    private_write(&temporary, &serde_json::to_vec(journal)?)?;
    std::fs::rename(&temporary, folder.join(JOURNAL)).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

/// Read a regular file without following a symlink at the last component, at most
/// `limit` bytes; more is an error.
fn read_nofollow(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP) => "is a symlink now".to_string(),
            _ if e.kind() == std::io::ErrorKind::NotFound => "no longer exists".to_string(),
            _ => e.to_string(),
        })?;
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err("is not a regular file now".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("changed since the edit (now over the size limit)".into());
    }
    Ok(bytes)
}

/// Read the pre-image of the file the approved `card` is about to change. `None` when
/// nothing can be recorded for it (not an edit card, unreadable, or not the state the
/// owner saw on the card).
pub fn before(root: &Path, card: &Card) -> Option<Capture> {
    if card.kind != CardKind::Edit {
        return None;
    }
    let (target, path) = super::edit::target(root, &card.tool, &card.arguments)?.ok()?;
    if super::excluded(&path) {
        return None;
    }
    let state = match std::fs::symlink_metadata(&target) {
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => return None,
        Ok(m) if m.len() as usize > MAX_PREIMAGE => Before::Unavailable(format!(
            "its pre-image is over the {MAX_PREIMAGE}-byte undo cap, so it is not kept"
        )),
        Ok(_) => {
            let bytes = read_nofollow(&target, MAX_PREIMAGE).ok()?;
            // Only the state the card showed; anything else is refused by `apply` anyway.
            if card.fingerprint != format!("{path}:{}", sha(&bytes))
                || std::str::from_utf8(&bytes).is_err()
            {
                return None;
            }
            Before::Image(bytes)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !card.fingerprint.starts_with(&format!("{path}:absent")) {
                return None;
            }
            Before::Created
        }
        Err(_) => return None,
    };
    Some(Capture {
        target,
        path,
        state,
    })
}

impl Capture {
    /// The edit has been applied: record the file's post-image in the journal of `turn`.
    /// Best effort; a failure only means no undo for this file.
    pub fn applied(self, root: &Path, dir: &Path, turn: &str) {
        let Ok(workspace) = root.canonicalize() else {
            return;
        };
        let workspace = workspace.to_string_lossy().into_owned();
        let Ok(post) = read_nofollow(&self.target, MAX_FILE) else {
            return;
        };
        let after = sha(&post);
        locked(dir, || {
            let folder = journal_dir(dir);
            if std::fs::symlink_metadata(&folder).is_ok_and(|m| m.file_type().is_symlink()) {
                return;
            }
            let mut journal = load(dir)
                .filter(|j| j.turn == turn && j.workspace == workspace)
                .unwrap_or_else(|| {
                    discard(dir);
                    Journal {
                        turn: turn.into(),
                        workspace,
                        next: 0,
                        bytes: 0,
                        entries: Vec::new(),
                    }
                });
            use std::os::unix::fs::DirBuilderExt;
            if std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&folder)
                .is_err()
            {
                return;
            }
            self.record(&folder, &mut journal, after);
            let _ = save(dir, &journal);
        });
    }

    fn record(self, folder: &Path, journal: &mut Journal, after: String) {
        let existing = journal.entries.iter().position(|e| e.path == self.path);
        if let Some(at) = existing {
            // A second edit of the file in this turn keeps the first pre-image, provided
            // nothing else touched the file in between.
            let entry = &mut journal.entries[at];
            let chained = match &self.state {
                Before::Image(bytes) => sha(bytes) == entry.after,
                _ => false,
            };
            if entry.unavailable.is_none() && !chained {
                entry.unavailable = Some("it changed between two edits of the turn".into());
                if let Some((name, _)) = entry.before.take() {
                    let _ = std::fs::remove_file(folder.join(name));
                }
            }
            entry.after = after;
            return;
        }
        let mut entry = Entry {
            path: self.path,
            created: matches!(self.state, Before::Created),
            before: None,
            after,
            unavailable: None,
        };
        match self.state {
            Before::Created => {}
            Before::Unavailable(reason) => entry.unavailable = Some(reason),
            Before::Image(bytes) if journal.bytes + bytes.len() > MAX_TURN_BYTES => {
                entry.unavailable = Some(format!(
                    "this turn's pre-images would pass the {MAX_TURN_BYTES}-byte undo cap"
                ));
            }
            Before::Image(bytes) => {
                let name = format!("{}.pre", journal.next);
                journal.next += 1;
                match private_write(&folder.join(&name), &bytes) {
                    Ok(()) => {
                        journal.bytes += bytes.len();
                        entry.before = Some((name, sha(&bytes)));
                    }
                    Err(_) => entry.unavailable = Some("its pre-image could not be saved".into()),
                }
            }
        }
        journal.entries.push(entry);
    }
}

/// `/undo` as the terminal runs it: the last turn's edits in `workspace`, outcomes logged
/// to the chat-approval log beside the journal in `dir`.
pub fn slash(dir: &Path, workspace: &Path) -> Result<Vec<String>, String> {
    run(workspace, dir, Some(&Log::new(dir)))
}

/// Undo the last turn's edits in `root`; one line per file after a summary line.
pub fn run(root: &Path, dir: &Path, log: Option<&Log>) -> Result<Vec<String>, String> {
    locked(dir, || run_locked(root, dir, log)).unwrap_or_else(|| {
        Err("the undo journal is busy or unavailable; nothing was undone".into())
    })
}

const NOTHING: &str = "Nothing to undo: no chat edit from the last turn is recorded";

fn run_locked(root: &Path, dir: &Path, log: Option<&Log>) -> Result<Vec<String>, String> {
    let Some(mut journal) = load(dir).filter(|j| !j.entries.is_empty()) else {
        return Ok(vec![NOTHING.into()]);
    };
    let workspace = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    if journal.workspace != workspace.to_string_lossy() {
        return Err(
            "the last turn's edits were made in another workspace; nothing was undone".into(),
        );
    }
    let record = |path: &str, outcome: Outcome| {
        if let Some(log) = log {
            log.append(
                &Card {
                    tool: "undo".into(),
                    arguments: json!({}),
                    kind: CardKind::Edit,
                    summary: format!("undo {path}"),
                    preview: String::new(),
                    fingerprint: String::new(),
                },
                &outcome,
            );
        }
    };
    let (mut done, mut refused) = (0, 0);
    let mut lines = Vec::new();
    let mut kept = Vec::new();
    for entry in journal.entries.drain(..).rev() {
        if let Some(reason) = &entry.unavailable {
            refused += 1;
            lines.push(format!("not available {}: {reason}", entry.path));
            record(
                &entry.path,
                Outcome::Failed(format!("undo not available: {reason}")),
            );
            continue;
        }
        match undo_one(&workspace, dir, &entry) {
            Ok(message) => {
                done += 1;
                lines.push(message);
                record(&entry.path, Outcome::Applied);
                if let Some((name, _)) = &entry.before {
                    let _ = std::fs::remove_file(journal_dir(dir).join(name));
                }
            }
            Err(reason) => {
                refused += 1;
                lines.push(format!("refused {}: {reason}", entry.path));
                record(&entry.path, Outcome::Failed(reason));
                kept.push(entry);
            }
        }
    }
    kept.reverse();
    journal.entries = kept;
    if journal.entries.is_empty() {
        discard(dir);
    } else {
        let _ = save(dir, &journal);
    }
    lines.insert(
        0,
        format!("/undo · {done} restored, {refused} refused or not available"),
    );
    Ok(lines)
}

/// Walk `relative` below the canonical `root` one component at a time; no symlink is
/// followed and the path must be an ordinary, non-excluded workspace path.
fn locate(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
        || super::excluded(relative)
    {
        return Err("not an editable path inside the Coding Workspace".into());
    }
    let names: Vec<_> = path.components().collect();
    let mut current = root.to_path_buf();
    for (index, name) in names.iter().enumerate() {
        current.push(name);
        let last = index + 1 == names.len();
        match std::fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("it or a parent directory is a symlink now".into())
            }
            Ok(m) if !last && !m.is_dir() => return Err("a parent is not a directory now".into()),
            Ok(m) if last && !m.is_file() => return Err("it is not a regular file now".into()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err("it no longer exists".into())
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(current)
}

const CHANGED: &str = "changed since the edit; left as it is";

fn undo_one(root: &Path, dir: &Path, entry: &Entry) -> Result<String, String> {
    let target = locate(root, &entry.path)?;
    let parent = target
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .filter(|p| p.starts_with(root))
        .ok_or_else(|| "its directory is not inside the workspace".to_string())?;
    let metadata = std::fs::symlink_metadata(&target).map_err(|e| e.to_string())?;
    refuse_read_only(&metadata, &entry.path)?;
    // Right before the swap: still this file, in this directory, with the recorded content.
    let unchanged = |path: &Path| -> Result<(), String> {
        let here = target
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .is_some_and(|p| p == parent);
        match read_nofollow(path, MAX_FILE) {
            Ok(bytes) if here && sha(&bytes) == entry.after => Ok(()),
            Ok(_) => Err(CHANGED.into()),
            Err(error) => Err(format!("{CHANGED} ({error})")),
        }
    };
    if entry.created {
        unchanged(&target)?;
        delete_if_unchanged(&target, &unchanged)?;
        return Ok(format!("deleted {} (created by the turn)", entry.path));
    }
    let (name, expected) = entry
        .before
        .as_ref()
        .ok_or_else(|| "no pre-image is recorded".to_string())?;
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("the journal names an invalid pre-image".into());
    }
    let bytes = read_nofollow(&journal_dir(dir).join(name), MAX_PREIMAGE)
        .map_err(|e| format!("pre-image unreadable: {e}"))?;
    if &sha(&bytes) != expected {
        return Err("the saved pre-image no longer matches its record".into());
    }
    let text =
        String::from_utf8(bytes).map_err(|_| "the saved pre-image is not text".to_string())?;
    write_atomic(&target, &text, || {
        unchanged(&target).map_err(std::io::Error::other)
    })
    .map_err(|e| e.trim_end_matches("; nothing was written").to_string())?;
    Ok(format!("restored {}", entry.path))
}

/// Delete `target` only if the file moved aside still passes `unchanged`: the rename is
/// atomic, so a write after the check cannot be lost; a changed file is moved back.
fn delete_if_unchanged(
    target: &Path,
    unchanged: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().chars().take(100).collect::<String>())
        .ok_or_else(|| "names no file".to_string())?;
    let aside = target.with_file_name(format!(".{name}.alfredo-undo-{}", new_turn()));
    std::fs::rename(target, &aside).map_err(|e| e.to_string())?;
    match unchanged(&aside) {
        Ok(()) => std::fs::remove_file(&aside).map_err(|e| e.to_string()),
        Err(reason) => {
            // `hard_link` + remove never replaces something created meanwhile.
            if std::fs::hard_link(&aside, target).is_ok() {
                let _ = std::fs::remove_file(&aside);
                Err(reason)
            } else {
                Err(format!(
                    "{reason}; it was moved to {} and could not be put back",
                    aside.display()
                ))
            }
        }
    }
}
