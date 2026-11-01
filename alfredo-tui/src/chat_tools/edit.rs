//! Mutating tier (issue #235): `write_file` and `edit_file` on the Coding Workspace
//! working tree. Each call pauses for owner Approval; [`preview`] builds the card
//! (and refuses invalid calls before asking), the run functions re-check and write.
use super::{display, excluded, resolve, text_arg, Tier, Tool};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

/// Largest file `write_file` or `edit_file` produces or edits, in bytes.
pub(crate) const MAX_FILE: usize = 1024 * 1024;
/// Most lines an approval card's diff may hold (issue #269). The count is every line of
/// the diff text: the `@@` header, `-`/`+` lines, line-ending notes and the
/// `new directories:` line. A change over it is refused before any card is shown, so
/// the owner never approves lines the card cut off.
const MAX_PREVIEW_LINES: usize = 80;

pub const WRITE_FILE: Tool = Tool {
    name: "write_file",
    tier: Tier::Mutating,
    description: "Create or replace a UTF-8 text file in the Coding Workspace with the given full content. Needs the owner's approval each call; the owner sees a diff. Prefer edit_file for small changes to an existing file. Creates missing directories.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path relative to the repository root"},
                "content": {"type": "string", "description": "The complete new file content"}
            },
            "required": ["path", "content"]
        })
    },
    run: write_file,
};

pub const EDIT_FILE: Tool = Tool {
    name: "edit_file",
    tier: Tier::Mutating,
    description: "Replace one exact occurrence of old_text with new_text in a text file of the Coding Workspace. old_text must match exactly once (include surrounding lines to make it unique); zero or several matches are refused. Needs the owner's approval each call. read_file first to copy old_text exactly (without line numbers).",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path relative to the repository root"},
                "old_text": {"type": "string", "description": "Exact text to replace; must occur exactly once"},
                "new_text": {"type": "string", "description": "Replacement text"}
            },
            "required": ["path", "old_text", "new_text"]
        })
    },
    run: edit_file,
};

/// What an approved call would change: a one-line summary and a diff.
pub(crate) struct Change {
    pub summary: String,
    pub diff: String,
    /// Target state when previewed (see [`fingerprint`]).
    pub fingerprint: String,
}

/// Content hash of the call's target file, or `absent`. A `write_file` target under
/// missing directories adds `+dirs:` and the directories it would create. `None` when `name` is not
/// one of these tools.
pub(crate) fn fingerprint(
    root: &Path,
    name: &str,
    arguments: &Value,
) -> Option<Result<String, String>> {
    if !matches!(name, "write_file" | "edit_file") {
        return None;
    }
    Some(required(arguments, "path").and_then(|path| {
        let Target {
            path: target,
            new_dirs,
        } = resolve_target(root, path, name == "write_file")?;
        let at = display(root, &target);
        match std::fs::read(&target) {
            Ok(bytes) => {
                use sha2::{Digest, Sha256};
                Ok(format!("{at}:{:x}", Sha256::digest(&bytes)))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let dirs = new_dirs
                    .iter()
                    .map(|dir| display(root, dir))
                    .collect::<Vec<_>>();
                if dirs.is_empty() {
                    Ok(format!("{at}:absent"))
                } else {
                    Ok(format!("{at}:absent+dirs:{}", dirs.join(",")))
                }
            }
            Err(error) => Err(error.to_string()),
        }
    }))
}

/// The file a `write_file/edit_file` call changes and its workspace-relative display form
/// (issue #325: `/undo` keys its pre-image on it). `None` when `name` is not one of these
/// tools.
pub(crate) fn target(
    root: &Path,
    name: &str,
    arguments: &Value,
) -> Option<Result<(PathBuf, String), String>> {
    if !matches!(name, "write_file" | "edit_file") {
        return None;
    }
    Some(required(arguments, "path").and_then(|path| {
        let target = resolve_target(root, path, name == "write_file")?.path;
        let shown = display(root, &target);
        Ok((target, shown))
    }))
}

/// Card content for a mutating call of this family, or why it is refused before asking.
/// `None` when `name` is not one of these tools.
pub(crate) fn preview(
    root: &Path,
    name: &str,
    arguments: &Value,
) -> Option<Result<Change, String>> {
    let fingerprint = match fingerprint(root, name, arguments)? {
        Ok(fingerprint) => fingerprint,
        Err(error) => return Some(Err(error)),
    };
    Some(match name {
        "write_file" => plan_write(root, arguments).map(|(path, old, new, new_dirs)| {
            let dirs = new_dirs
                .iter()
                .map(|dir| display(root, dir))
                .collect::<Vec<_>>()
                .join(", ");
            let body = diff(old.as_deref().unwrap_or(""), &new);
            Change {
                summary: match (&old, dirs.is_empty()) {
                    (Some(_), _) => format!("replace {}", display(root, &path)),
                    (None, true) => format!("new file {}", display(root, &path)),
                    (None, false) => {
                        format!(
                            "new file {} (new directories: {dirs})",
                            display(root, &path)
                        )
                    }
                },
                diff: if dirs.is_empty() {
                    body
                } else {
                    format!("new directories: {dirs}\n{body}")
                },
                fingerprint,
            }
        }),
        "edit_file" => plan_edit(root, arguments).map(|(path, old, new)| Change {
            summary: format!("edit {}", display(root, &path)),
            diff: diff(&old, &new),
            fingerprint,
        }),
        _ => return None,
    })
}

fn required<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    text_arg(arguments, name)?.ok_or_else(|| format!("argument {name} is required"))
}

/// A resolved write target and the missing directories (outermost first) that
/// must be created before the file can be written.
struct Target {
    path: PathBuf,
    new_dirs: Vec<PathBuf>,
}

/// Resolve a path that may not exist yet. Its parent must exist inside the workspace,
/// unless `create_dirs` (`write_file`) and every missing parent lies below a real,
/// non-symlink, writable directory inside the workspace; the missing ones are then
/// returned in [`Target::new_dirs`]. The same exclusion policy applies to each of them.
fn resolve_target(root: &Path, path: &str, create_dirs: bool) -> Result<Target, String> {
    refuse_control_characters(path)?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("argument path is required".into());
    }
    let relative = Path::new(trimmed);
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(format!(
            "path {path:?} must be relative to the Coding Workspace and stay inside it"
        ));
    }
    if excluded(&relative.to_string_lossy()) {
        return Err(format!(
            "{path} is excluded (Git metadata, secrets or generated files)"
        ));
    }
    let existing = |path: PathBuf| Target {
        path,
        new_dirs: Vec::new(),
    };
    match std::fs::symlink_metadata(root.join(relative)) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "{path} is a symlink; chat edits change regular files only"
        )),
        Ok(_) => resolve(root, trimmed).map(existing),
        Err(_) => {
            let name = relative
                .file_name()
                .ok_or_else(|| format!("path {path:?} names no file"))?;
            let parent = relative.parent().filter(|p| !p.as_os_str().is_empty());
            let parent = match parent {
                Some(parent) => match resolve(root, &parent.to_string_lossy()) {
                    Ok(parent) => parent,
                    Err(_) if create_dirs => return plan_new_dirs(root, path, relative),
                    Err(_) => return Err(format!("parent directory of {path} does not exist")),
                },
                None => root
                    .canonicalize()
                    .map_err(|_| "Coding Workspace is not available".to_string())?,
            };
            if !parent.is_dir() {
                return Err(format!("parent of {path} is not a directory"));
            }
            let full = parent.join(name);
            if excluded(&display(root, &full)) {
                return Err(format!(
                    "{path} is excluded (Git metadata, secrets or generated files)"
                ));
            }
            Ok(existing(full))
        }
    }
}

/// Issue #313: a path with a control character (ESC, BEL, C1 CSI, line break) is refused
/// by its escaped form, never echoed raw, so the card, the logs and the terminal stay text.
/// The same refusal as `git_read`'s path argument.
fn refuse_control_characters(path: &str) -> Result<(), String> {
    if path.chars().any(char::is_control) {
        return Err(format!(
            "path {:?} must be relative to the Coding Workspace and stay inside it",
            path.chars().take(64).collect::<String>()
        ));
    }
    Ok(())
}

/// Walk `relative` from the workspace root one component at a time (no symlink is
/// followed) to the first missing parent; every component before it must be a real
/// directory. Returns the target under the canonical root plus the directories to create.
fn plan_new_dirs(root: &Path, path: &str, relative: &Path) -> Result<Target, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    let names: Vec<_> = relative
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    let (file, parents) = names
        .split_last()
        .ok_or_else(|| format!("path {path:?} names no file"))?;
    let mut current = root.clone();
    let mut new_dirs: Vec<PathBuf> = Vec::new();
    for name in parents {
        current.push(name);
        if !new_dirs.is_empty() {
            new_dirs.push(current.clone());
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(format!(
                    "{path}: {} is a symlink; new directories are not created through symlinks",
                    display(&root, &current)
                ))
            }
            Ok(m) if m.is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "parent of {path} is not a directory: {}",
                    display(&root, &current)
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => new_dirs.push(current.clone()),
            Err(e) => return Err(e.to_string()),
        }
    }
    let anchor = new_dirs
        .first()
        .and_then(|first| first.parent())
        .ok_or_else(|| format!("parent directory of {path} does not exist"))?;
    refuse_read_only(
        &std::fs::metadata(anchor).map_err(|e| e.to_string())?,
        &display(&root, anchor),
    )?;
    let full = current.join(file);
    for checked in new_dirs.iter().chain(std::iter::once(&full)) {
        if excluded(&display(&root, checked)) {
            return Err(format!(
                "{path} is excluded (Git metadata, secrets or generated files)"
            ));
        }
    }
    Ok(Target {
        path: full,
        new_dirs,
    })
}

/// Create `dirs` (outermost first) one level at a time. Each step must create a real
/// directory: before each, the parent is re-canonicalized and must still lie under the
/// canonical root and be a real directory (not a symlink). Any failure removes the
/// directories this call already made (only empty ones go).
fn create_dirs(root: &Path, dirs: &[PathBuf]) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    let mut made: Vec<&PathBuf> = Vec::new();
    let result = (|| {
        for dir in dirs {
            if let Some(parent) = dir.parent() {
                let metadata = std::fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
                let inside = parent
                    .canonicalize()
                    .map(|real| real.starts_with(&root))
                    .unwrap_or(false);
                if metadata.file_type().is_symlink() || !metadata.is_dir() || !inside {
                    return Err(format!(
                        "{} is not a real directory inside the workspace any more; nothing more was created",
                        parent.display()
                    ));
                }
            }
            std::fs::create_dir(dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            made.push(dir);
            let metadata = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!("{} is not a real directory", dir.display()));
            }
        }
        Ok(())
    })();
    if result.is_err() {
        for dir in made.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
    }
    result
}

/// Removes the temp file on drop unless the rename succeeded (`armed` cleared).
struct TempGuard {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Atomically replace `target` with `text` (issue #265): the bytes go to a temp file
/// beside the target (`.NAME.alfredo-tmp-RANDOM`), which is synced, given the original's
/// permission bits and renamed over the target. Any failure leaves the original
/// byte-identical and removes the temp file. A target with several hardlinks is replaced
/// by a new file, so the other links keep their old content (the link is broken, not
/// written through). A symlink at the target is refused; `rename` replaces a symlink
/// rather than following it, and the temp file is created with `create_new`.
/// Owner, group and extended attributes of an existing file are not carried over.
fn write_regular(target: &Path, text: &str) -> Result<(), String> {
    write_atomic(target, text, || Ok(()))
}

/// [`write_regular`] with a seam that runs after the temp file is written and synced,
/// right before the rename; tests inject a failure there.
pub(crate) fn write_atomic(
    target: &Path,
    text: &str,
    before_rename: impl FnOnce() -> std::io::Result<()>,
) -> Result<(), String> {
    use std::io::Write;
    let existing = match std::fs::symlink_metadata(target) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err("target became a symlink; nothing was written".into())
        }
        Ok(m) if !m.is_file() => return Err("target is not a regular file".into()),
        Ok(m) => {
            refuse_read_only(&m, &target.to_string_lossy())?;
            Some(m)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .ok_or_else(|| "target names no file".to_string())?
        .to_string_lossy();
    let name: String = name.chars().take(100).collect();
    let (mut file, mut guard) = {
        let mut attempt = 0u32;
        loop {
            let suffix = {
                use std::hash::{BuildHasher, Hasher};
                let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
                hasher.write_u32(std::process::id());
                hasher.write_u32(attempt);
                hasher.finish()
            };
            let path = dir.join(format!(".{name}.alfredo-tmp-{suffix:016x}"));
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            match options.open(&path) {
                Ok(file) => break (file, TempGuard { path, armed: true }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < 8 => {
                    attempt += 1;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    };
    if let Some(metadata) = &existing {
        file.set_permissions(metadata.permissions())
            .map_err(|e| e.to_string())?;
    }
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    before_rename().map_err(|e| e.to_string())?;
    // Re-check right before the swap so a symlink put there meanwhile is not replaced.
    if std::fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("target became a symlink; nothing was written".into());
    }
    std::fs::rename(&guard.path, target).map_err(|e| e.to_string())?;
    guard.armed = false;
    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Refuse a target whose owner write bit is clear. `rename` ignores the file's own
/// permissions, so without this check the atomic write would replace a `chmod 444` file.
pub(crate) fn refuse_read_only(metadata: &std::fs::Metadata, shown: &str) -> Result<(), String> {
    #[cfg(unix)]
    let read_only = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o200 == 0
    };
    #[cfg(not(unix))]
    let read_only = metadata.permissions().readonly();
    if read_only {
        return Err(format!(
            "{shown} is read-only; nothing was written. Ask the user to make it writable (chmod u+w) if the change is intended"
        ));
    }
    Ok(())
}

fn read_existing(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("not a regular file".into());
    }
    if metadata.len() as usize > MAX_FILE {
        return Err(format!("file is over the {MAX_FILE}-byte edit limit"));
    }
    String::from_utf8(std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| "not a UTF-8 text file".into())
}

/// (target, current content if any, new content, directories to create first)
fn plan_write(
    root: &Path,
    arguments: &Value,
) -> Result<(PathBuf, Option<String>, String, Vec<PathBuf>), String> {
    let path = required(arguments, "path")?;
    let content = required(arguments, "content")?;
    if content.len() > MAX_FILE {
        return Err(format!("content is over the {MAX_FILE}-byte limit"));
    }
    let Target {
        path: target,
        new_dirs,
    } = resolve_target(root, path, true)?;
    let old = if target.exists() {
        refuse_read_only(
            &std::fs::metadata(&target).map_err(|e| e.to_string())?,
            path,
        )?;
        Some(read_existing(&target)?)
    } else {
        None
    };
    refuse_over_cap(
        old.as_deref().unwrap_or(""),
        content,
        usize::from(!new_dirs.is_empty()),
    )?;
    Ok((target, old, content.to_string(), new_dirs))
}

/// Refuses a change whose card diff (plus `extra` card lines) would pass
/// [`MAX_PREVIEW_LINES`], before anything is asked or written.
fn refuse_over_cap(old: &str, new: &str, extra: usize) -> Result<(), String> {
    let lines = diff(old, new).lines().count() + extra;
    if lines > MAX_PREVIEW_LINES {
        return Err(format!(
            "the change's diff is {lines} lines, over the {MAX_PREVIEW_LINES}-line approval card limit; nothing was written. Split it into smaller edits (edit_file on a part, or several write_file calls to separate files)"
        ));
    }
    Ok(())
}

/// (target, current content, edited content)
fn plan_edit(root: &Path, arguments: &Value) -> Result<(PathBuf, String, String), String> {
    let path = required(arguments, "path")?;
    refuse_control_characters(path)?;
    let raw_old_text = required(arguments, "old_text")?;
    let raw_new_text = required(arguments, "new_text")?;
    if raw_old_text.is_empty() {
        return Err("old_text must not be empty".into());
    }
    let target = resolve(root, path)?;
    if let Ok(metadata) = std::fs::metadata(&target) {
        refuse_read_only(&metadata, path)?;
    }
    let current = read_existing(&target)?;
    // A file whose every line ends in CRLF is matched and edited as LF text (the model
    // sees `\r` as nothing), then written back with CRLF. Mixed files are matched on
    // exact bytes and the replacement is inserted as given.
    let crlf = line_endings(&current) == "CRLF";
    let (haystack, old_text, new_text) = if crlf {
        (
            current.replace("\r\n", "\n"),
            raw_old_text.replace("\r\n", "\n"),
            raw_new_text.replace("\r\n", "\n"),
        )
    } else {
        (
            current.clone(),
            raw_old_text.to_string(),
            raw_new_text.to_string(),
        )
    };
    match haystack.matches(old_text.as_str()).count() {
        0 => Err(format!(
            "old_text not found in {path}; read_file and copy the text exactly{}",
            if line_endings(&current) == "mixed" {
                " (the file mixes CRLF and LF line endings, so old_text must match its exact bytes)"
            } else {
                ""
            }
        )),
        1 => {
            let mut edited = haystack.replacen(old_text.as_str(), &new_text, 1);
            if crlf {
                edited = edited.replace('\n', "\r\n");
            }
            if edited.len() > MAX_FILE {
                return Err(format!("result is over the {MAX_FILE}-byte limit"));
            }
            if edited == current {
                return Err(if raw_old_text == raw_new_text {
                    "new_text is identical to old_text; nothing to change".into()
                } else {
                    "edit_file keeps the file's CRLF endings; use write_file to change them".into()
                });
            }
            refuse_over_cap(&current, &edited, 0)?;
            Ok((target, current, edited))
        }
        count => Err(format!(
            "old_text occurs {count} times in {path}; include more surrounding text so it matches once"
        )),
    }
}

/// `CRLF` or `LF` when every line break is that, `mixed`, or `none` without breaks.
fn line_endings(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    match (crlf, lf) {
        (0, 0) => "none",
        (_, 0) => "CRLF",
        (0, _) => "LF",
        _ => "mixed",
    }
}

fn write_file(root: &Path, arguments: &Value) -> Result<String, String> {
    let (target, old, new, new_dirs) = plan_write(root, arguments)?;
    create_dirs(root, &new_dirs)?;
    if let Err(error) = write_regular(&target, &new) {
        // Take back the directories this call made (only empty ones go).
        for dir in new_dirs.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
        return Err(error);
    }
    Ok(format!(
        "{} {} ({} bytes)",
        if old.is_some() { "wrote" } else { "created" },
        display(root, &target),
        new.len()
    ))
}

fn edit_file(root: &Path, arguments: &Value) -> Result<String, String> {
    let (target, _, edited) = plan_edit(root, arguments)?;
    write_regular(&target, &edited)?;
    Ok(format!("edited {} (1 replacement)", display(root, &target)))
}

/// Line diff: unchanged head and tail are skipped; the changed middle shows as
/// `-` old lines then `+` new lines. Never cut: callers refuse a change over
/// [`MAX_PREVIEW_LINES`] first (see [`refuse_over_cap`]).
pub(crate) fn diff(old: &str, new: &str) -> String {
    let (old_text, new_text) = (old, new);
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut lines = vec![format!("@@ line {} @@", head + 1)];
    lines.extend(old[head..old.len() - tail].iter().map(|l| format!("-{l}")));
    lines.extend(new[head..new.len() - tail].iter().map(|l| format!("+{l}")));
    let notes = ending_notes(old_text, new_text);
    if lines.len() == 1 {
        return if notes.is_empty() {
            "(no line changes)".into()
        } else {
            notes.join("\n")
        };
    }
    lines.extend(notes);
    lines.join("\n")
}

/// Card lines for a change to line endings or the final newline, which `lines()` hides.
/// Empty for a new (empty) file or when the bytes are equal.
fn ending_notes(old: &str, new: &str) -> Vec<String> {
    let mut notes = vec![];
    if old.is_empty() || old == new {
        return notes;
    }
    let (before, after) = (line_endings(old), line_endings(new));
    if before != after && before != "none" && after != "none" {
        notes.push(format!("line endings {before} \u{2192} {after}"));
    }
    match (old.ends_with('\n'), new.ends_with('\n')) {
        (true, false) => notes.push("final newline removed".into()),
        (false, true) => notes.push("final newline added".into()),
        _ => {}
    }
    if notes.is_empty() && old.lines().eq(new.lines()) {
        notes.push("line endings changed".into());
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("alfredo-265-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("alfredo-tmp"))
            .collect()
    }

    #[test]
    fn a_failed_write_leaves_the_original_and_no_temp_file() {
        let dir = temp_dir("fail");
        let target = dir.join("a.txt");
        std::fs::write(&target, "original\n").unwrap();
        let err = write_atomic(&target, &"x".repeat(48_000), || {
            Err(std::io::Error::other("injected failure"))
        })
        .unwrap_err();
        assert!(err.contains("injected failure"));
        assert_eq!(std::fs::read(&target).unwrap(), b"original\n");
        assert!(leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unwritable_directory_leaves_the_original() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("ro");
        let target = dir.join("a.txt");
        std::fs::write(&target, "original\n").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = write_regular(&target, "new\n");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Root ignores directory permissions; the write then succeeds.
        if result.is_err() {
            assert_eq!(std::fs::read(&target).unwrap(), b"original\n");
        }
        assert!(leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hardlinked_file_does_not_change_its_other_link() {
        let dir = temp_dir("hl");
        let outside = dir.join("outside.txt");
        let linked = dir.join("hl.txt");
        std::fs::write(&outside, "outside\n").unwrap();
        std::fs::hard_link(&outside, &linked).unwrap();
        write_regular(&linked, "changed\n").unwrap();
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside\n");
        assert_eq!(std::fs::read(&linked).unwrap(), b"changed\n");
        assert!(leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn permission_bits_survive_a_replace() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let target = dir.join("run.sh");
        std::fs::write(&target, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o751)).unwrap();
        write_regular(&target, "#!/bin/sh\necho hi\n").unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o751);
        assert_eq!(std::fs::read(&target).unwrap(), b"#!/bin/sh\necho hi\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_read_only_file_is_refused_at_planning_and_at_the_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("readonly");
        let target = dir.join("locked.txt");
        std::fs::write(&target, "locked\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o444)).unwrap();
        let write = json!({"path": "locked.txt", "content": "new\n"});
        let edit = json!({"path": "locked.txt", "old_text": "locked", "new_text": "open"});
        for (name, args) in [("write_file", &write), ("edit_file", &edit)] {
            let error = preview(&dir, name, args).unwrap().err().expect(name);
            assert!(error.contains("locked.txt is read-only"), "{name}: {error}");
        }
        assert!(write_file(&dir, &write)
            .unwrap_err()
            .contains("is read-only"));
        assert!(edit_file(&dir, &edit).unwrap_err().contains("is read-only"));
        let error = write_regular(&target, "new\n").unwrap_err();
        assert!(error.contains("is read-only"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), b"locked\n");
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o444);
        assert!(leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_file_is_created_and_a_symlink_target_is_refused() {
        let dir = temp_dir("new");
        let fresh = dir.join("fresh.txt");
        write_regular(&fresh, "hi\n").unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"hi\n");
        let link = dir.join("link.txt");
        std::os::unix::fs::symlink(&fresh, &link).unwrap();
        assert!(write_regular(&link, "no\n").is_err());
        assert_eq!(std::fs::read(&fresh).unwrap(), b"hi\n");
        assert!(leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Issue #510: the card shows minimal hunks, one `@@ line N @@` header each.
    fn hundred(changes: &[(usize, &str)]) -> (String, String) {
        let old: String = (1..=100).map(|n| format!("x{n}\n")).collect();
        let new: String = (1..=100)
            .map(|n| match changes.iter().find(|(at, _)| *at == n) {
                Some((_, text)) => format!("{text}\n"),
                None => format!("x{n}\n"),
            })
            .collect();
        (old, new)
    }

    #[test]
    fn scattered_changes_show_one_hunk_each_and_no_unchanged_lines() {
        let (old, new) = hundred(&[(1, "first"), (100, "last")]);
        assert_eq!(
            diff(&old, &new),
            "@@ line 1 @@\n-x1\n+first\n@@ line 100 @@\n-x100\n+last"
        );
        assert!(refuse_over_cap(&old, &new, 0).is_ok());
    }

    #[test]
    fn insertions_deletions_and_replacements_get_their_own_headers() {
        let old = "a\nb\nc\nd\ne\nf\n";
        let new = "a\nB\nc\nd\nnew\ne\n";
        assert_eq!(
            diff(old, new),
            "@@ line 2 @@\n-b\n+B\n@@ line 5 @@\n+new\n@@ line 6 @@\n-f"
        );
    }

    #[test]
    fn a_single_contiguous_change_is_one_span_hunk_as_before() {
        let old = "a\nb\nc\nd\n";
        assert_eq!(diff(old, "a\nX\nY\nd\n"), "@@ line 2 @@\n-b\n-c\n+X\n+Y");
    }

    #[test]
    fn a_minimal_diff_still_over_the_cap_is_refused_with_todays_message() {
        // 41 scattered one-line changes: 41 headers + 82 changed lines.
        let changes: Vec<(usize, &str)> = (0..41).map(|i| (2 * i + 1, "changed")).collect();
        let (old, new) = hundred(&changes);
        let error = refuse_over_cap(&old, &new, 0).unwrap_err();
        assert!(
            error.starts_with("the change's diff is 123 lines, over the 80-line approval card limit; nothing was written. Split it into smaller edits"),
            "{error}"
        );
        // 26 changes: 26 headers + 52 changed lines = 78 passes; the extra line counts.
        let changes: Vec<(usize, &str)> = (0..26).map(|i| (2 * i + 1, "changed")).collect();
        let (old, new) = hundred(&changes);
        assert_eq!(diff(&old, &new).lines().count(), 78);
        assert!(refuse_over_cap(&old, &new, 2).is_ok());
        assert!(refuse_over_cap(&old, &new, 3).is_err());
    }

    #[test]
    fn a_change_beyond_the_diff_bound_falls_back_to_the_span_and_is_refused() {
        let old: String = (0..400).map(|n| format!("o{n}\n")).collect();
        let new: String = (0..400).map(|n| format!("n{n}\n")).collect();
        assert!(diff(&old, &new).starts_with("@@ line 1 @@\n-o0\n"));
        assert!(refuse_over_cap(&old, &new, 0).is_err());
    }

    /// Applies a card diff to `old` line by line (the reviewer's reading of it).
    fn apply_card(old: &str, card: &str) -> Vec<String> {
        let old: Vec<&str> = old.lines().collect();
        let mut out: Vec<String> = vec![];
        let mut at = 0; // next old line index
        let mut removed = 0;
        let mut pending: Vec<String> = vec![];
        let mut flush = |out: &mut Vec<String>,
                         at: &mut usize,
                         removed: &mut usize,
                         pending: &mut Vec<String>| {
            *at += *removed;
            out.append(pending);
            *removed = 0;
        };
        for line in card.lines() {
            if let Some(n) = line
                .strip_prefix("@@ line ")
                .and_then(|r| r.strip_suffix(" @@"))
            {
                flush(&mut out, &mut at, &mut removed, &mut pending);
                let start: usize = n.parse().unwrap();
                while at < start - 1 {
                    out.push(old[at].to_string());
                    at += 1;
                }
            } else if let Some(gone) = line.strip_prefix('-') {
                assert_eq!(
                    old[at + removed],
                    gone,
                    "a removed line must match the old text"
                );
                removed += 1;
            } else if let Some(added) = line.strip_prefix('+') {
                pending.push(added.to_string());
            }
        }
        flush(&mut out, &mut at, &mut removed, &mut pending);
        out.extend(old[at..].iter().map(|l| l.to_string()));
        out
    }

    struct Prng(u64);
    impl Prng {
        fn next(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as usize
        }
    }

    #[test]
    fn applying_the_hunks_to_the_old_text_reproduces_the_new_text() {
        let mut rng = Prng(0x9E37_79B9_7F4A_7C15);
        let mut pairs = 0;
        for _ in 0..600 {
            let eol = ["\n", "\r\n"][rng.next(2)];
            let len = rng.next(14);
            let old_lines: Vec<String> = (0..len).map(|_| format!("l{}", rng.next(6))).collect();
            let mut new_lines = old_lines.clone();
            for _ in 0..rng.next(5) {
                match rng.next(3) {
                    0 if !new_lines.is_empty() => {
                        let at = rng.next(new_lines.len());
                        new_lines.remove(at);
                    }
                    1 => {
                        let at = rng.next(new_lines.len() + 1);
                        new_lines.insert(at, format!("n{}", rng.next(6)));
                    }
                    _ if !new_lines.is_empty() => {
                        let at = rng.next(new_lines.len());
                        new_lines[at] = format!("r{}", rng.next(6));
                    }
                    _ => {}
                }
            }
            let join = |lines: &[String], eol: &str, final_eol: bool| {
                let mut text = lines.join(eol);
                if final_eol && !lines.is_empty() {
                    text.push_str(eol);
                }
                text
            };
            let new_eol = if rng.next(4) == 0 {
                ["\n", "\r\n"][rng.next(2)]
            } else {
                eol
            };
            let old = join(&old_lines, eol, rng.next(3) != 0);
            let new = join(&new_lines, new_eol, rng.next(3) != 0);
            let card = diff(&old, &new);
            let expected: Vec<String> = new.lines().map(str::to_string).collect();
            assert_eq!(
                apply_card(&old, &card),
                expected,
                "old={old:?} new={new:?}\n{card}"
            );
            if old != new {
                assert_ne!(card, "(no line changes)", "old={old:?} new={new:?}");
            }
            // Every changed line is on the card: removed and added counts match the lines
            // the old/new texts hold beyond their common subsequence.
            let removed = card.lines().filter(|l| l.starts_with('-')).count();
            let added = card.lines().filter(|l| l.starts_with('+')).count();
            assert_eq!(
                old.lines().count() - removed,
                new.lines().count() - added,
                "unchanged lines must agree: old={old:?} new={new:?}"
            );
            pairs += 1;
        }
        assert_eq!(pairs, 600);
    }

    #[test]
    fn crlf_and_final_newline_changes_keep_their_notes_beside_the_hunks() {
        assert_eq!(
            diff("a\r\nb\r\n", "a\nb\n"),
            "line endings CRLF \u{2192} LF"
        );
        assert_eq!(
            diff("a\nb\nc\n", "a\nB\nc"),
            "@@ line 2 @@\n-b\n+B\nfinal newline removed"
        );
        assert_eq!(diff("a\nb", "a\nb\n"), "final newline added");
        assert_eq!(diff("a\n", "a\n"), "(no line changes)");
    }
}
