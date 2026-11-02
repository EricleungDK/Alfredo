//! User-defined slash commands (issue #486): `NAME.md` files in `<State Directory>/commands`.
//!
//! `/NAME args` sends the file's body as an ordinary chat prompt with `$ARGUMENTS` replaced
//! by `args`, so the owner sees it in the transcript and every tool call it triggers meets
//! the normal approval card. Commands are read from the State Directory only, never from the
//! workspace or any repository directory, so a cloned repo cannot plant one. A file is read
//! through a descriptor opened with `O_NOFOLLOW` and checked there (regular file, at most
//! [`MAX_BYTES`], UTF-8), so a symlink or a swap after listing is refused, not followed.
use crate::commands::{CAPABILITIES, COMMANDS};
use std::{
    fs,
    io::Read,
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

/// Reads `path` without following a symlink; refuses non-files, files over [`MAX_BYTES`],
/// non-UTF-8 and empty bodies. The error names `command`.
fn read_body(path: &Path, command: &str) -> Result<String, String> {
    let refuse = |why: &str| format!("{command} refused: {why}");
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| {
            if error.raw_os_error() == Some(libc::ELOOP) {
                refuse("symbolic link")
            } else {
                refuse("cannot be opened")
            }
        })?;
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
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            Some((name.strip_suffix(".md")?.to_owned(), entry.path()))
        })
        .collect();
    files.sort();
    for (stem, path) in files {
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
            match read_body(&path, &command) {
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
    let path = dir.join(format!("{stem}.md"));
    // Absent is not a user command (the usual "Unknown command"); anything else present
    // is read under the same checks as the listing.
    fs::symlink_metadata(&path).ok()?;
    Some(read_body(&path, word).map(|body| {
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
