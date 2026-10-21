//! Mutating tier (issue #235): write_file and edit_file on the Coding Workspace
//! working tree. Each call pauses for owner Approval; [`preview`] builds the card
//! (and refuses invalid calls before asking), the run functions re-check and write.
use super::{display, excluded, resolve, text_arg, Tier, Tool};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

/// Largest file write_file or edit_file produces or edits, in bytes.
const MAX_FILE: usize = 1024 * 1024;
/// Diff lines shown on an approval card before it is cut.
const MAX_PREVIEW_LINES: usize = 80;

pub const WRITE_FILE: Tool = Tool {
    name: "write_file",
    tier: Tier::Mutating,
    description: "Create or replace a UTF-8 text file in the Coding Workspace with the given full content. Needs the owner's approval each call; the owner sees a diff. Prefer edit_file for small changes to an existing file. The parent directory must exist.",
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

/// Content hash of the call's target file, or `absent`. `None` when `name` is not
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
        let target = resolve_target(root, path)?;
        let at = display(root, &target);
        match std::fs::read(&target) {
            Ok(bytes) => {
                use sha2::{Digest, Sha256};
                Ok(format!("{at}:{:x}", Sha256::digest(&bytes)))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(format!("{at}:absent"))
            }
            Err(error) => Err(error.to_string()),
        }
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
        "write_file" => plan_write(root, arguments).map(|(path, old, new)| Change {
            summary: match &old {
                Some(_) => format!("replace {}", display(root, &path)),
                None => format!("new file {}", display(root, &path)),
            },
            diff: diff(old.as_deref().unwrap_or(""), &new),
            fingerprint,
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

/// Resolve a path that may not exist yet: its parent must exist inside the workspace.
fn resolve_target(root: &Path, path: &str) -> Result<PathBuf, String> {
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
    match std::fs::symlink_metadata(root.join(relative)) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "{path} is a symlink; chat edits change regular files only"
        )),
        Ok(_) => resolve(root, trimmed),
        Err(_) => {
            let name = relative
                .file_name()
                .ok_or_else(|| format!("path {path:?} names no file"))?;
            let parent = relative.parent().filter(|p| !p.as_os_str().is_empty());
            let parent = match parent {
                Some(parent) => resolve(root, &parent.to_string_lossy())
                    .map_err(|_| format!("parent directory of {path} does not exist"))?,
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
            Ok(full)
        }
    }
}

/// Write `text` to `target` without following a symlink placed there since it was
/// resolved.
fn write_regular(target: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    if std::fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("target became a symlink; nothing was written".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(target).map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())
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

/// (target, current content if any, new content)
fn plan_write(root: &Path, arguments: &Value) -> Result<(PathBuf, Option<String>, String), String> {
    let path = required(arguments, "path")?;
    let content = required(arguments, "content")?;
    if content.len() > MAX_FILE {
        return Err(format!("content is over the {MAX_FILE}-byte limit"));
    }
    let target = resolve_target(root, path)?;
    let old = if target.exists() {
        Some(read_existing(&target)?)
    } else {
        None
    };
    Ok((target, old, content.to_string()))
}

/// (target, current content, edited content)
fn plan_edit(root: &Path, arguments: &Value) -> Result<(PathBuf, String, String), String> {
    let path = required(arguments, "path")?;
    let old_text = required(arguments, "old_text")?;
    let new_text = required(arguments, "new_text")?;
    if old_text.is_empty() {
        return Err("old_text must not be empty".into());
    }
    let target = resolve(root, path)?;
    let current = read_existing(&target)?;
    match current.matches(old_text).count() {
        0 => Err(format!(
            "old_text not found in {path}; read_file and copy the text exactly"
        )),
        1 => {
            let edited = current.replacen(old_text, new_text, 1);
            if edited.len() > MAX_FILE {
                return Err(format!("result is over the {MAX_FILE}-byte limit"));
            }
            Ok((target, current, edited))
        }
        count => Err(format!(
            "old_text occurs {count} times in {path}; include more surrounding text so it matches once"
        )),
    }
}

fn write_file(root: &Path, arguments: &Value) -> Result<String, String> {
    let (target, old, new) = plan_write(root, arguments)?;
    write_regular(&target, &new)?;
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
/// `-` old lines then `+` new lines, cut at [`MAX_PREVIEW_LINES`].
pub(crate) fn diff(old: &str, new: &str) -> String {
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
    if lines.len() == 1 {
        return "(no line changes)".into();
    }
    let total = lines.len();
    if total > MAX_PREVIEW_LINES {
        lines.truncate(MAX_PREVIEW_LINES);
        lines.push(format!("[{} more diff lines]", total - MAX_PREVIEW_LINES));
    }
    lines.join("\n")
}
