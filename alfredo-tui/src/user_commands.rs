//! User-defined slash commands (issue #486): `NAME.md` files in `<State Directory>/commands`.
//!
//! `/NAME args` sends the file's body as an ordinary chat prompt with `$ARGUMENTS` replaced
//! by `args`, so the owner sees it in the transcript and every tool call it triggers meets
//! the normal approval card. Commands are read from the State Directory only, never from the
//! workspace or any repository directory, so a cloned repo cannot plant one. A file is read
//! through a descriptor opened with `O_NOFOLLOW` and checked there (regular file, at most
//! [`MAX_BYTES`], UTF-8), so a symlink or a swap after listing is refused, not followed.
use crate::commands::{CAPABILITIES, COMMANDS};
use crate::model::{Message, Session};
use std::{
    fs,
    io::Read,
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::RwLock,
};

pub const MAX_BYTES: u64 = 64 * 1024;
const MAX_COMMANDS: usize = 64;
const MAX_NAME: usize = 32;
/// Handled outside `COMMANDS`, so reserved by hand.
const RESERVED: &[&str] = &["/help"];

/// Names (`/NAME`) shown by the type-ahead and F1 as user commands.
static LISTED: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// The listing result: loadable commands, and each refused file with its reason.
#[derive(Debug, Default)]
pub struct Scan {
    pub names: Vec<String>,
    pub refused: Vec<(String, String)>,
}

impl Scan {
    /// One line naming every refused command, or `None` when nothing was refused.
    pub fn notice(&self) -> Option<String> {
        if self.refused.is_empty() {
            return None;
        }
        let list: Vec<_> = self
            .refused
            .iter()
            .map(|(name, why)| format!("{name} ({why})"))
            .collect();
        Some(format!("Refused user commands: {}", list.join(", ")))
    }
}

/// Where user commands live for `state_dir`.
pub fn directory(state_dir: &Path) -> PathBuf {
    state_dir.join("commands")
}

/// The names the type-ahead and F1 list as user commands.
pub fn listed() -> Vec<String> {
    LISTED.read().map(|names| names.clone()).unwrap_or_default()
}

/// Makes `scan`'s commands the ones the type-ahead and F1 list.
pub fn publish(scan: &Scan) {
    if let Ok(mut names) = LISTED.write() {
        names.clone_from(&scan.names);
    }
}

fn shown(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .take(40)
        .collect()
}

fn built_in(name: &str) -> bool {
    RESERVED.contains(&name)
        || COMMANDS
            .iter()
            .chain(CAPABILITIES)
            .any(|(known, _)| *known == name)
}

fn valid_name(stem: &str) -> bool {
    !stem.is_empty()
        && stem.len() <= MAX_NAME
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Opens the commands directory itself without following a symlink.
fn open_dir(dir: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(dir)
}

/// Reads `file_name` inside the already-open `dir` without following a symlink (`openat`
/// with `O_NOFOLLOW`, so neither the directory nor the file can be a link); refuses
/// non-files, files over [`MAX_BYTES`], non-UTF-8 and empty bodies. The error names `command`.
fn read_body(dir: &fs::File, file_name: &str, command: &str) -> Result<String, String> {
    let refuse = |why: &str| format!("{command} refused: {why}");
    let name = std::ffi::CString::new(file_name).map_err(|_| refuse("cannot be opened"))?;
    let flags =
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC;
    // SAFETY: `name` is a valid NUL-terminated string and `dir` is an open descriptor;
    // a nonnegative result is a fresh descriptor that nothing else owns.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ELOOP) {
                refuse("symbolic link")
            } else {
                refuse("cannot be opened")
            },
        );
    }
    // SAFETY: `fd` was just returned by `openat` and is owned by no other object.
    let file = unsafe { fs::File::from_raw_fd(fd) };
    let meta = file.metadata().map_err(|_| refuse("cannot be read"))?;
    if !meta.is_file() {
        return Err(refuse("not a regular file"));
    }
    if meta.size() > MAX_BYTES {
        return Err(refuse("over 64 KiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| refuse("cannot be read"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(refuse("over 64 KiB"));
    }
    let body = String::from_utf8(bytes).map_err(|_| refuse("not UTF-8 text"))?;
    if body.trim().is_empty() {
        return Err(refuse("empty"));
    }
    Ok(body)
}

/// Lists the loadable `*.md` commands in `dir`. A missing directory is no commands; a
/// symlinked one is refused. Refusals are collected so a notice can name them.
pub fn scan(dir: &Path) -> Scan {
    let mut found = Scan::default();
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(meta) if meta.file_type().is_symlink() => {
            found
                .refused
                .push(("commands directory".into(), "symbolic link".into()));
            return found;
        }
        _ => return found,
    }
    let (Ok(handle), Ok(entries)) = (open_dir(dir), fs::read_dir(dir)) else {
        return found;
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            Some(name.strip_suffix(".md")?.to_owned())
        })
        .collect();
    files.sort();
    for stem in files {
        let command = format!("/{}", shown(&stem));
        let refuse =
            |found: &mut Scan, why: &str| found.refused.push((command.clone(), why.into()));
        if !valid_name(&stem) {
            refuse(&mut found, "use letters, digits, - and _ (up to 32)");
        } else if built_in(&command) {
            refuse(&mut found, "clashes with a built-in command");
        } else if found.names.len() >= MAX_COMMANDS {
            refuse(&mut found, "more than 64 commands");
        } else {
            match read_body(&handle, &format!("{stem}.md"), &command) {
                Ok(_) => found.names.push(command),
                Err(error) => {
                    let why = error
                        .split_once("refused: ")
                        .map_or(error.as_str(), |(_, w)| w);
                    found.refused.push((command.clone(), why.to_owned()));
                }
            }
        }
    }
    found
}

/// The prompt for `text` when its first word is a user command in `dir`: `None` for any
/// other text, `Some(Err)` when the file is no longer acceptable. The file is read afresh.
pub fn expand(dir: &Path, text: &str) -> Option<Result<String, String>> {
    let text = text.trim();
    let word = text.split_whitespace().next()?;
    let stem = word.strip_prefix('/')?;
    if !valid_name(stem) || built_in(word) {
        return None;
    }
    let arguments = text[word.len()..].trim();
    // A missing or linked commands directory has no commands; a missing file is the usual
    // "Unknown command". Anything else present is read under the listing's checks.
    let handle = open_dir(dir).ok()?;
    let file_name = format!("{stem}.md");
    // Existence only picks "Unknown command" over a refusal; the read below decides.
    fs::symlink_metadata(dir.join(&file_name)).ok()?;
    Some(read_body(&handle, &file_name, word).map(|body| {
        let body = body.trim();
        if body.contains("$ARGUMENTS") {
            body.replace("$ARGUMENTS", arguments)
        } else if arguments.is_empty() {
            body.to_owned()
        } else {
            format!("{body}\n\nARGUMENTS: {arguments}")
        }
    }))
}

/// Rescans `dir` for the type-ahead and F1; the notice names every refusal.
pub fn reload(dir: &Path) -> Option<String> {
    let found = scan(dir);
    publish(&found);
    found.notice()
}

/// Whether Enter on `draft` belongs to a user command: no agent view is open and the first
/// word names a command file in `dir`.
pub fn claims(agent_shown: bool, dir: &Path, draft: &str) -> bool {
    !agent_shown && expand(dir, draft).is_some()
}

/// A claimed draft after a fresh read: the trimmed text, its command word, and the prompt
/// or the notice to show when the command can no longer run.
#[derive(Debug, PartialEq, Eq)]
pub struct Resolved {
    pub typed: String,
    pub name: String,
    pub prompt: Result<String, String>,
}

/// Re-reads the command for `draft` (the file may have changed since `claims`).
pub fn resolve(dir: &Path, draft: &str) -> Resolved {
    let typed = draft.trim().to_owned();
    let name = typed.split_whitespace().next().unwrap_or("").to_owned();
    let prompt = match expand(dir, &typed) {
        Some(Ok(prompt)) => Ok(prompt),
        Some(Err(error)) => Err(error),
        None => Err(format!("{name} is no longer available")),
    };
    Resolved {
        typed,
        name,
        prompt,
    }
}

/// A command or scope receipt is pending, so a user command must wait.
pub fn is_blocked(wayfinder_active: bool, command_pending: bool) -> bool {
    wayfinder_active || command_pending
}

/// Summarises the staged draft first when the conversation is near its limit: `spawn`
/// runs only when `due`, and its success means the send waits for the summary.
pub fn summarise_first(due: bool, spawn: impl FnOnce() -> bool) -> bool {
    due && spawn()
}

/// The notice after sending a staged command.
pub fn sent_notice(name: &str, sent: &Result<Vec<Message>, &'static str>) -> String {
    match sent {
        Ok(_) => format!("{name} sent as your message"),
        Err(error) => (*error).into(),
    }
}

/// Puts the expanded `prompt` of `typed` (`name` args) into `session`'s draft so it can be
/// sent as an ordinary chat message. Refuses, leaving the typed command in the draft, while
/// a command or scope receipt is pending (`blocked`), while a reply runs, or when the
/// prompt does not fit the draft limit.
pub fn stage(
    session: &mut Session,
    name: &str,
    typed: &str,
    prompt: &str,
    blocked: bool,
) -> Result<(), String> {
    if blocked {
        return Err("Wait for the pending command or scope receipt first".into());
    }
    if session.status.active() {
        return Err("Wait for the reply to end, then run the command".into());
    }
    session.clear_draft();
    if session.insert(prompt).truncated {
        restore(session, typed);
        return Err(format!(
            "{name} refused: the prompt is over the 16 KiB limit"
        ));
    }
    Ok(())
}

/// Sends the staged draft; when the session refuses it, the typed command comes back.
pub fn send(session: &mut Session, typed: &str) -> Result<Vec<Message>, &'static str> {
    let sent = session.begin();
    if sent.is_err() {
        restore(session, typed);
    }
    sent
}

fn restore(session: &mut Session, typed: &str) {
    session.clear_draft();
    session.insert(typed);
    session.reject_draft();
}
