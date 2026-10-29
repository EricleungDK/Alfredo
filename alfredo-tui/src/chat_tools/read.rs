//! Read tier: read_file, list_files, grep on the Coding Workspace working tree.
use super::{
    budget, display, excluded, git_read, open_confined, resolve, text_arg, Tier, Tool, MAX_OUTPUT,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Largest file read_file or grep opens, in bytes.
const MAX_FILE: u64 = 4 * 1024 * 1024;
/// Files listed before the listing is cut.
const MAX_ENTRIES: usize = 500;
/// Directory entries visited by one walk.
const MAX_VISITS: usize = 20_000;
/// Bytes of file content one grep scans before it stops (issue #257).
const GREP_SCAN_BYTES: u64 = 64 * 1024 * 1024;
/// Wall time one grep may run before it stops (issue #257).
const GREP_WALL: Duration = Duration::from_secs(10);
/// Matching lines reported by one grep.
const MAX_MATCHES: usize = 200;
/// Bytes reserved for markers when one line is cut to fit the output.
const MAX_LINE_MARKER: usize = 300;
/// Characters of one matching line shown.
const MAX_LINE: usize = 200;

pub const READ_FILE: Tool = Tool {
    name: "read_file",
    tier: Tier::Read,
    description: "Read a UTF-8 text file in the Coding Workspace (the owner's repository working tree). Returns numbered lines (line<TAB>text). Long files are cut and over-long lines are shown cut; call again with start_line to continue.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path relative to the repository root, e.g. README.md or src/main.rs"},
                "start_line": {"type": "integer", "description": "First line to return (1-based, default 1)"}
            },
            "required": ["path"]
        })
    },
    run: read_file,
};

pub const LIST_FILES: Tool = Tool {
    name: "list_files",
    tier: Tier::Read,
    description: "List files (recursively) in the Coding Workspace or one of its directories, one relative path per line. Skips .git, secrets (.env*, *.key, .npmrc) and generated/vendor trees.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Directory relative to the repository root (default: the root)"},
                "glob": {"type": "string"}
            }
        })
    },
    run: list_files,
};

pub const GREP: Tool = Tool {
    name: "grep",
    tier: Tier::Read,
    description: "Search text files in the Coding Workspace for a literal substring. Returns path:line:text per match. Skips .git, secrets, generated or vendor trees and binary files.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Literal text to find (not a regular expression)"},
                "path": {"type": "string", "description": "File or directory to search, relative to the repository root (default: the root)"},
                "ignore_case": {"type": "boolean", "description": "Match regardless of case (default false)"},
                "glob": {"type": "string"}
            },
            "required": ["pattern"]
        })
    },
    run: grep,
};

/// Contents of a regular text file inside the workspace, or why it cannot be read.
/// `path` is a path [`resolve`] returned; the size and the bytes come from the opened
/// handle, so a file swapped after the check is refused (issue #360).
pub(crate) fn text(root: &Path, path: &Path) -> Result<String, String> {
    let (file, len) = open_confined(root, path)?;
    contents(file, len)
}

/// The text of an opened file whose handle reports `len` bytes (see [`text`]).
fn contents(file: std::fs::File, len: u64) -> Result<String, String> {
    if len > MAX_FILE {
        return Err(format!(
            "file is {len} bytes, over the {MAX_FILE}-byte read limit; use grep"
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.contains(&0) {
        return Err("binary file; not shown".into());
    }
    String::from_utf8(bytes).map_err(|_| "not a UTF-8 text file; not shown".into())
}

fn read_file(root: &Path, arguments: &Value) -> Result<String, String> {
    read_file_within(root, arguments, MAX_OUTPUT)
}

/// [`read_file`] sized to `limit` bytes (at most [`MAX_OUTPUT`]), so a budgeted result
/// still ends on a whole line and keeps its `start_line=` continuation footer.
pub(crate) fn read_file_within(
    root: &Path,
    arguments: &Value,
    limit: usize,
) -> Result<String, String> {
    let budget = limit.min(MAX_OUTPUT);
    let path = text_arg(arguments, "path")?.ok_or("missing required argument path")?;
    let start = match arguments.get("start_line") {
        None | Some(Value::Null) => 1,
        Some(value) => value
            .as_u64()
            .filter(|line| *line >= 1)
            .ok_or("start_line must be a positive integer")? as usize,
    };
    let full = resolve(root, path)?;
    let content = text(root, &full).map_err(|error| format!("{path}: {error}"))?;
    let total = content.lines().count();
    if total == 0 {
        return Ok(format!("{path} is empty"));
    }
    if start > total {
        return Err(format!(
            "{path} has {total} lines; start_line {start} is past the end"
        ));
    }
    let mut out = String::new();
    for (number, line) in content.lines().enumerate().skip(start - 1) {
        let entry = format!("{}\t{line}\n", number + 1);
        if out.len() + entry.len() <= budget.saturating_sub(120) {
            out.push_str(&entry);
            continue;
        }
        if !out.is_empty() {
            // Stop before it; the next call starts at this line and makes progress.
            out.push_str(&format!(
                "[truncated at line {number} of {total}; call read_file with start_line={} to continue]",
                number + 1
            ));
            return Ok(out);
        }
        // The line alone exceeds the budget: show its start, cut at a char boundary.
        let prefix = format!("{}\t", number + 1);
        let mut end = budget
            .saturating_sub(MAX_LINE_MARKER.min(budget / 2) + prefix.len())
            .min(line.len());
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        out.push_str(&prefix);
        out.push_str(&line[..end]);
        out.push_str(&format!(
            "\n[line {} is {} bytes; showed the first {end}]",
            number + 1,
            line.len()
        ));
        if number + 1 < total {
            out.push_str(&format!(
                "\n[truncated at line {} of {total}; call read_file with start_line={} to continue]",
                number + 1,
                number + 2
            ));
        }
        return Ok(out);
    }
    Ok(out)
}

/// Regular files under `start` (inside the canonical `root`) in sorted order; symlinks are
/// never followed. Each entry is tested by its workspace-relative path, so a rule scoped to
/// a parent directory (`.docker/config.json`) applies here as it does in the git listing.
/// Returns the files and whether the walk stopped early.
fn walk(root: &Path, start: &Path, limit: usize, skip_noise: bool) -> (Vec<PathBuf>, bool) {
    let mut files = Vec::new();
    let mut stack = vec![start.to_path_buf()];
    let mut visits = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        let mut dirs = Vec::new();
        for entry in entries {
            visits += 1;
            if visits > MAX_VISITS || files.len() >= limit {
                return (files, true);
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            // A path outside `root` cannot be tested by its relative name: skip it.
            let Ok(relative) = entry
                .path()
                .strip_prefix(root)
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            else {
                continue;
            };
            if excluded(&relative) || (skip_noise && noise(&name.to_lowercase())) {
                continue;
            }
            if kind.is_dir() {
                dirs.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
        // Depth-first, in name order.
        stack.extend(dirs.into_iter().rev());
    }
    (files, false)
}

/// Directory names skipped in discovery below the listed path, even with no `.gitignore`:
/// Python virtualenvs, bytecode caches and installed packages. `build` is not here, since
/// it can be real source; a `.gitignore` hides it when it is not.
fn noise(lower: &str) -> bool {
    lower
        .split('/')
        .any(|part| matches!(part, ".venv" | "venv" | "__pycache__" | "site-packages"))
}

/// Largest git listing used for discovery, in bytes; a bigger one falls back to the walk.
const MAX_LISTING: usize = 16 << 20;

/// The note for a listing git refused because of the repository config (an include or a
/// filter key): `.gitignore` is not applied, so ignored files may appear. Names the first
/// clause of the reason with control characters escaped as `<U+XXXX>`. Other failures (no
/// repository, git missing) are the ordinary no-git case and carry no note.
fn ignore_not_applied(reason: &str) -> Option<String> {
    if !reason.starts_with("repository config") {
        return None;
    }
    let clause = reason.split(';').next().unwrap_or(reason);
    let mut shown = String::new();
    for c in clause.chars().take(200) {
        if c.is_control() {
            shown.push_str(&format!("<U+{:04X}>", u32::from(c)));
        } else {
            shown.push(c);
        }
    }
    Some(format!("note: .gitignore not applied ({shown})"))
}

/// Files chosen for discovery under `start` (an existing path inside `root`), in the
/// order [`walk`] yields, whether the listing stopped early, and a note when `start` is
/// itself ignored or the repository config made git refuse the listing. In a git workspace
/// git's ignore rules decide (tracked files stay);
/// otherwise, or when git cannot list, the plain [`walk`] is the fallback. [`excluded`]
/// and [`noise`] always apply on top, so ignore files can only hide more.
pub(crate) fn discover(
    root: &Path,
    start: &Path,
    limit: usize,
) -> (Vec<PathBuf>, bool, Option<String>) {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let rel = start
        .strip_prefix(&canonical)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let excludes = git_read::global_excludes();
    let ignored = git_read::is_ignored(root, &rel, excludes.as_deref());
    // Inside an ignored directory (by git, or a virtualenv/cache name) the caller asked
    // for it by name: list it with the plain walk and say so.
    let named = noise(&rel.to_lowercase());
    if named || (!rel.is_empty() && ignored == Ok(true)) {
        let (files, cut) = walk(&canonical, start, limit, false);
        let note = format!("note: {rel} is ignored (by .gitignore or as a virtualenv or cache directory); listed because you named it");
        return (files, cut, Some(note));
    }
    let listing = match git_read::list_unignored(root, &rel, excludes.as_deref(), MAX_LISTING) {
        Ok(listing) => listing,
        Err(reason) => {
            let (files, cut) = walk(&canonical, start, limit, true);
            return (files, cut, ignore_not_applied(&reason));
        }
    };
    let mut paths: Vec<Vec<&str>> = listing
        .iter()
        .map(|path| path.split('/').collect())
        .collect();
    // The walk's order: within a directory its files by name, then its subdirectories.
    paths.sort_by(|a, b| {
        for (index, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            if x != y {
                let (x_file, y_file) = (index + 1 == a.len(), index + 1 == b.len());
                return y_file.cmp(&x_file).then_with(|| x.cmp(y));
            }
        }
        a.len().cmp(&b.len())
    });
    paths.dedup();
    let mut parents: HashMap<String, bool> = HashMap::new();
    let mut files = Vec::new();
    for parts in paths {
        let relative = parts.join("/");
        let lower = relative.to_lowercase();
        // Git does not validate the names in an index it reads: only a clean relative
        // path may become a file (an absolute or `..` name must never leave the root).
        // Each component is tested as well as the whole path, as the walk tested every
        // directory name, so a `certs.key/` directory stays hidden.
        if !crate::planning_context::path_valid(&relative)
            || parts.iter().any(|part| excluded(part))
            || excluded(&relative)
            || noise(&lower)
        {
            continue;
        }
        let full = canonical.join(&relative);
        if !full.starts_with(&canonical) {
            continue;
        }
        // A regular file only (never a symlink, a submodule or a deleted tracked path),
        // reached through real directories: a tracked directory swapped for a symlink
        // must not lead out of the workspace.
        let parent = parts[..parts.len() - 1].join("/");
        let real = *parents.entry(parent.clone()).or_insert_with(|| {
            canonical
                .join(&parent)
                .canonicalize()
                .is_ok_and(|real| real == canonical.join(&parent))
        });
        if !real || !std::fs::symlink_metadata(&full).is_ok_and(|meta| meta.is_file()) {
            continue;
        }
        if files.len() >= limit {
            return (files, true, None);
        }
        files.push(full);
    }
    (files, false, None)
}

/// Longest `glob` accepted, in characters.
const MAX_GLOB_CHARS: usize = 256;
/// Most `*` and `?` characters one `glob` may hold.
const MAX_GLOB_WILDCARDS: usize = 16;

/// One piece of a parsed glob.
enum Piece {
    Literal(char),
    /// `?`: one character other than `/`.
    One,
    /// `*`: any run of characters other than `/`.
    Star,
    /// `**`: any run of characters, `/` included.
    Deep,
    /// `**/`: nothing, or any run of characters ending in `/`.
    DeepDir,
}

/// A filter on the relative paths a listing or search returns (`*`, `?`, `**`).
/// A glob with no `/` is tested against the file name, one with `/` against the whole
/// path relative to the repository root. It only narrows what discovery returned.
struct Glob {
    text: String,
    pieces: Vec<Piece>,
    name_only: bool,
}

impl Glob {
    /// The optional `glob` argument; an empty one means no filter.
    fn from_arguments(arguments: &Value) -> Result<Option<Self>, String> {
        let Some(glob) = text_arg(arguments, "glob")?.filter(|glob| !glob.is_empty()) else {
            return Ok(None);
        };
        let glob = glob.trim_start_matches("./");
        let chars: Vec<char> = glob.chars().collect();
        if chars.len() > MAX_GLOB_CHARS {
            return Err(format!(
                "glob is {} characters; at most {MAX_GLOB_CHARS} are allowed",
                chars.len()
            ));
        }
        if chars.iter().filter(|c| matches!(c, '*' | '?')).count() > MAX_GLOB_WILDCARDS {
            return Err(format!(
                "glob has more than {MAX_GLOB_WILDCARDS} wildcards (* and ?); use a simpler glob"
            ));
        }
        let mut pieces = Vec::new();
        let mut at = 0;
        while at < chars.len() {
            match chars[at] {
                '*' if chars.get(at + 1) == Some(&'*') => {
                    let mut end = at;
                    while chars.get(end) == Some(&'*') {
                        end += 1;
                    }
                    if chars.get(end) == Some(&'/') {
                        pieces.push(Piece::DeepDir);
                        end += 1;
                    } else {
                        pieces.push(Piece::Deep);
                    }
                    at = end;
                    continue;
                }
                '*' => pieces.push(Piece::Star),
                '?' => pieces.push(Piece::One),
                other => pieces.push(Piece::Literal(other)),
            }
            at += 1;
        }
        Ok(Some(Self {
            text: glob.to_string(),
            pieces,
            name_only: !glob.contains('/'),
        }))
    }

    /// Whether the relative path `path` (`/`-separated) matches. A row-by-row table, so
    /// the work is at most pieces x characters whatever the glob holds.
    fn matches(&self, path: &str) -> bool {
        let path = if self.name_only {
            path.rsplit('/').next().unwrap_or(path)
        } else {
            path
        };
        let text: Vec<char> = path.chars().collect();
        // `row[i]`: the pieces so far match the first `i` characters.
        let mut row = vec![false; text.len() + 1];
        row[0] = true;
        for piece in &self.pieces {
            let mut next = vec![false; text.len() + 1];
            // `DeepDir` also matches after any earlier position, up to a `/`.
            let mut seen = false;
            for i in 0..=text.len() {
                let before = i.checked_sub(1).map(|j| text[j]);
                next[i] = match piece {
                    Piece::Literal(c) => before == Some(*c) && row[i - 1],
                    Piece::One => before.is_some_and(|c| c != '/') && row[i - 1],
                    Piece::Star => row[i] || (before.is_some_and(|c| c != '/') && next[i - 1]),
                    Piece::Deep => row[i] || (i > 0 && next[i - 1]),
                    Piece::DeepDir => row[i] || (before == Some('/') && seen),
                };
                seen |= row[i];
            }
            row = next;
        }
        row[text.len()]
    }
}

/// A `[note: …]` line naming arguments the tool does not take, or an empty string.
fn unknown_note(arguments: &Value, known: &[&str]) -> String {
    let unknown: Vec<&str> = arguments
        .as_object()
        .into_iter()
        .flat_map(|map| map.keys())
        .map(String::as_str)
        .filter(|name| !known.contains(name))
        .collect();
    if unknown.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = unknown
        .iter()
        .map(|name| name.chars().take(40).collect())
        .collect();
    format!(
        "[note: unknown argument{} {} ignored; accepted: {}]\n",
        if shown.len() == 1 { "" } else { "s" },
        shown.join(", "),
        known.join(", ")
    )
}

/// `path`, resolved; a name with a glob character that is not a real path is refused
/// with a pointer to the `glob` argument.
fn resolve_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    // Only a path that stays inside is probed, so an outside path is refused by
    // `resolve` exactly as before and its existence is never revealed.
    let inside = Path::new(path.trim()).components().all(|part| {
        matches!(
            part,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    });
    if inside && path.contains(['*', '?']) && !root.join(path.trim()).exists() {
        return Err(format!(
            "path {path:?} is a literal path, not a pattern; pass the directory as path and the pattern as the glob argument (for example glob={path:?})"
        ));
    }
    resolve(root, path)
}

fn list_files(root: &Path, arguments: &Value) -> Result<String, String> {
    let glob = Glob::from_arguments(arguments)?;
    let path = text_arg(arguments, "path")?.unwrap_or(".");
    let full = resolve_path(root, path)?;
    let mut out = unknown_note(arguments, &["path", "glob"]);
    if full.is_file() {
        let shown = display(root, &full);
        if glob.as_ref().is_some_and(|glob| !glob.matches(&shown)) {
            out.push_str(&format!(
                "no files match glob {:?}",
                glob.as_ref().map_or("", |g| g.text.as_str())
            ));
            return Ok(out);
        }
        out.push_str(&shown);
        return Ok(out);
    }
    // A glob filters after discovery, so look further than the listing cap.
    let limit = if glob.is_some() {
        MAX_VISITS
    } else {
        MAX_ENTRIES
    };
    let (mut files, mut cut, note) = discover(root, &full, limit);
    if let Some(glob) = &glob {
        files.retain(|file| glob.matches(&display(root, file)));
        if files.len() > MAX_ENTRIES {
            files.truncate(MAX_ENTRIES);
            cut = true;
        }
    }
    if let Some(note) = note {
        out.push_str(&format!("[{note}]\n"));
    }
    if files.is_empty() {
        match &glob {
            Some(glob) => out.push_str(&format!("no files match glob {:?}", glob.text)),
            None => out.push_str("no files"),
        }
        return Ok(out);
    }
    out.extend(files.iter().map(|file| display(root, file) + "\n"));
    if cut {
        out.push_str(&format!(
            "[truncated: first {} files; list a subdirectory with path]",
            files.len()
        ));
    }
    Ok(out)
}

/// Whether `pattern` holds regular-expression syntax a literal search will not honour.
fn looks_like_regex(pattern: &str) -> bool {
    pattern.contains(['\\', '|', '[', '(', '+', '?']) || pattern.contains(".*")
}

fn grep(root: &Path, arguments: &Value) -> Result<String, String> {
    grep_within(root, arguments, GREP_SCAN_BYTES, GREP_WALL)
}

/// [`grep`] with its scan budget as parameters: it stops once `scan_bytes` of file
/// content were read or `wall` passed, and ends at once when the round is cancelled.
fn grep_within(
    root: &Path,
    arguments: &Value,
    scan_bytes: u64,
    wall: Duration,
) -> Result<String, String> {
    let started = Instant::now();
    let round = budget::current();
    let mut scanned = 0u64;
    let mut stopped = false;
    let pattern = text_arg(arguments, "pattern")?
        .filter(|pattern| !pattern.is_empty())
        .ok_or("missing required argument pattern")?;
    let ignore_case = arguments
        .get("ignore_case")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let needle = if ignore_case {
        pattern.to_lowercase()
    } else {
        pattern.to_string()
    };
    let glob = Glob::from_arguments(arguments)?;
    let path = text_arg(arguments, "path")?.unwrap_or(".");
    let full = resolve_path(root, path)?;
    let (mut files, mut cut, note) = if full.is_file() {
        (vec![full], false, None)
    } else {
        discover(root, &full, MAX_VISITS)
    };
    if let Some(glob) = &glob {
        files.retain(|file| glob.matches(&display(root, file)));
    }
    let mut out = unknown_note(arguments, &["pattern", "path", "ignore_case", "glob"]);
    if let Some(note) = note {
        out.push_str(&format!("[{note}]\n"));
    }
    let mut matches = 0;
    'files: for file in &files {
        if round.as_ref().is_some_and(|b| b.cancel.is_set()) {
            return Err("cancelled".into());
        }
        if scanned >= scan_bytes
            || started.elapsed() >= wall
            || round.as_ref().is_some_and(budget::Budget::expired)
        {
            stopped = true;
            break;
        }
        // The budget counts what the handle reports, as the open saw it (issue #360).
        let Ok((handle, len)) = open_confined(root, file) else {
            continue;
        };
        scanned += len.min(MAX_FILE);
        let Ok(content) = contents(handle, len) else {
            continue;
        };
        for (number, line) in content.lines().enumerate() {
            let found = if ignore_case {
                line.to_lowercase().contains(&needle)
            } else {
                line.contains(&needle)
            };
            if !found {
                continue;
            }
            if matches == MAX_MATCHES || out.len() > MAX_OUTPUT - 400 {
                cut = true;
                break 'files;
            }
            matches += 1;
            let shown: String = line.chars().take(MAX_LINE).collect();
            out.push_str(&format!("{}:{}:{shown}\n", display(root, file), number + 1));
        }
    }
    if matches == 0 && !cut && !stopped {
        out.push_str("no matches");
        if looks_like_regex(pattern) {
            out.push_str(" (grep matches literal text, not regular expressions; search for one literal piece of text, or run several greps; filter files with glob, e.g. glob=\"src/*.rs\")");
        }
        return Ok(out);
    }
    if cut {
        out.push_str(&format!(
            "[truncated: first {matches} matches; narrow with path or a longer pattern]"
        ));
    }
    if stopped {
        if cut {
            out.push('\n');
        }
        out.push_str(&format!(
            "[stopped: scanned {:.1} MiB in {:.1} s; narrow with path]",
            scanned as f64 / (1024.0 * 1024.0),
            started.elapsed().as_secs_f64()
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn the_gitignore_note_escapes_control_and_format_characters_and_caps_its_length() {
        let note =
            ignore_not_applied("repository config sets \"a\u{1b}[31m\u{202e}b\u{2028}c\"; rest")
                .unwrap_or_default();
        assert_eq!(
            note,
            "note: .gitignore not applied (repository config sets \"a<U+001B>[31m<U+202E>b<U+2028>c\")"
        );
        // Multibyte text is cut on a character boundary, at 200 characters.
        let long = format!("repository config sets {}", "é".repeat(300));
        let note = ignore_not_applied(&long).unwrap_or_default();
        let shown = note.trim_start_matches("note: .gitignore not applied (");
        assert_eq!(shown.trim_end_matches(')').chars().count(), 200);
        assert!(note.ends_with("é)"), "{note}");
        assert_eq!(ignore_not_applied("fatal: not a git repository"), None);
    }

    fn tree(files: usize, size: usize) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "alfredo-grep-budget-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for n in 0..files {
            let body = format!("needle {n}\n{}", "x".repeat(size));
            std::fs::write(dir.join(format!("f{n:03}.txt")), body).unwrap();
        }
        dir
    }

    #[test]
    fn grep_stops_at_the_byte_budget_and_keeps_the_matches_found() {
        let dir = tree(10, 1000);
        let out = grep_within(
            &dir,
            &json!({"pattern": "needle"}),
            2500,
            Duration::from_secs(60),
        )
        .unwrap();
        // Three files fit before 2500 bytes are reached (each is ~1008).
        assert_eq!(out.matches("needle").count(), 3, "{out}");
        assert!(out.contains("f000.txt:1:needle 0"), "{out}");
        assert!(out.contains("[stopped: scanned 0.0 MiB in "), "{out}");
        assert!(out.ends_with("narrow with path]"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn grep_stops_at_the_wall_deadline() {
        let dir = tree(5, 100);
        let out = grep_within(
            &dir,
            &json!({"pattern": "needle"}),
            u64::MAX,
            Duration::ZERO,
        )
        .unwrap();
        assert!(out.starts_with("[stopped: scanned 0.0 MiB"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_grep_within_budget_has_no_stop_marker() {
        let dir = tree(3, 100);
        let out = grep_within(
            &dir,
            &json!({"pattern": "needle"}),
            u64::MAX,
            Duration::from_secs(60),
        )
        .unwrap();
        assert!(!out.contains("[stopped"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A workspace with the given `(relative path, content)` files, removed on drop.
    fn files(entries: &[(&str, &str)]) -> (PathBuf, Cleanup) {
        let dir = std::env::temp_dir().join(format!(
            "alfredo-glob-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        for (path, body) in entries {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, body).unwrap();
        }
        (dir.clone(), Cleanup(dir))
    }

    fn sample() -> (PathBuf, Cleanup) {
        files(&[
            ("README.md", "fn readme\n"),
            ("src/a.rs", "fn main() {}\nfn helper_one() {}\n"),
            ("src/b.py", "def b(): pass\n"),
            ("src/deep/c.rs", "fn deep() {}\n"),
        ])
    }

    fn listed(root: &Path, glob: &str) -> Vec<String> {
        let out = list_files(root, &json!({"glob": glob})).unwrap();
        out.lines()
            .filter(|line| !line.starts_with('[') && !line.starts_with("no files"))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn grep_with_regex_metacharacters_and_no_match_says_it_is_literal() {
        let (dir, _cleanup) = sample();
        for pattern in [r"fn \w+\(", "fn|def", "a[bc]", "(x)", "fn.*zz", "q+", "zz?"] {
            let out = grep(&dir, &json!({"pattern": pattern})).unwrap();
            assert!(
                out.starts_with("no matches (grep matches literal text, not regular expressions;"),
                "{pattern}: {out}"
            );
        }
    }

    #[test]
    fn grep_hint_only_appears_for_a_regexish_pattern_with_no_match() {
        let (dir, _cleanup) = sample();
        let plain = grep(&dir, &json!({"pattern": "absent-text"})).unwrap();
        assert_eq!(plain, "no matches");
        // A match is reported as before, with no hint.
        let hit = grep(&dir, &json!({"pattern": "fn main("})).unwrap();
        assert_eq!(hit, "src/a.rs:1:fn main() {}\n");
    }

    #[test]
    fn grep_glob_filters_the_files_searched() {
        let (dir, _cleanup) = sample();
        let rs = grep(&dir, &json!({"pattern": "fn", "glob": "*.rs"})).unwrap();
        assert!(
            rs.contains("src/a.rs:1:") && rs.contains("src/deep/c.rs:1:"),
            "{rs}"
        );
        assert!(!rs.contains("README") && !rs.contains("b.py"), "{rs}");
        let shallow = grep(&dir, &json!({"pattern": "fn", "glob": "src/*.rs"})).unwrap();
        assert!(
            shallow.contains("src/a.rs") && !shallow.contains("deep"),
            "{shallow}"
        );
        let none = grep(&dir, &json!({"pattern": "fn", "glob": "*.zig"})).unwrap();
        assert_eq!(none, "no matches");
    }

    #[test]
    fn list_files_glob_supports_star_question_and_double_star() {
        let (dir, _cleanup) = sample();
        assert_eq!(listed(&dir, "*.rs"), ["src/a.rs", "src/deep/c.rs"]);
        assert_eq!(listed(&dir, "src/*.rs"), ["src/a.rs"]);
        assert_eq!(listed(&dir, "src/?.py"), ["src/b.py"]);
        assert_eq!(listed(&dir, "src/**/*.rs"), ["src/a.rs", "src/deep/c.rs"]);
        assert_eq!(listed(&dir, "**/c.rs"), ["src/deep/c.rs"]);
        assert_eq!(
            listed(&dir, "src/**"),
            ["src/a.rs", "src/b.py", "src/deep/c.rs"]
        );
        assert_eq!(listed(&dir, "README.md"), ["README.md"]);
        // `*` and `?` never cross a directory separator.
        assert!(listed(&dir, "src?a.rs").is_empty());
        assert!(listed(&dir, "src/*c.rs").is_empty());
    }

    #[test]
    fn list_files_glob_with_no_match_names_the_glob() {
        let (dir, _cleanup) = sample();
        let out = list_files(&dir, &json!({"glob": "*.zig"})).unwrap();
        assert!(out.contains("no files") && out.contains("*.zig"), "{out}");
    }

    #[test]
    fn a_glob_never_widens_past_the_exclusions() {
        let (dir, _cleanup) = files(&[
            (".env", "TOPSECRET=1\n"),
            ("deploy.key", "TOPSECRET\n"),
            ("node_modules/x/index.js", "TOPSECRET\n"),
            ("ok.txt", "TOPSECRET\n"),
        ]);
        assert_eq!(listed(&dir, "**"), ["ok.txt"]);
        assert_eq!(listed(&dir, ".env"), Vec::<String>::new());
        let out = grep(&dir, &json!({"pattern": "TOPSECRET", "glob": "**"})).unwrap();
        assert_eq!(out, "ok.txt:1:TOPSECRET\n");
    }

    #[test]
    fn a_path_with_a_glob_character_gets_an_error_naming_glob() {
        let (dir, _cleanup) = sample();
        for result in [
            list_files(&dir, &json!({"path": "src/*.rs"})),
            grep(&dir, &json!({"pattern": "fn", "path": "src/*.rs"})),
            grep(&dir, &json!({"pattern": "fn", "path": "src/?.rs"})),
        ] {
            let error = result.unwrap_err();
            assert!(error.contains("glob"), "{error}");
            assert!(error.contains("src/"), "{error}");
        }
    }

    #[test]
    fn an_outside_path_with_a_glob_character_is_refused_as_before() {
        let (dir, _cleanup) = sample();
        for path in ["../*.txt", "/etc/*", "src/../../*"] {
            let error = list_files(&dir, &json!({"path": path})).unwrap_err();
            assert_eq!(error, resolve(&dir, path).unwrap_err(), "{path}");
            assert!(error.contains("stay inside"), "{path}: {error}");
            assert_eq!(
                grep(&dir, &json!({"pattern": "x", "path": path})).unwrap_err(),
                error
            );
        }
    }

    #[test]
    fn a_leading_dot_slash_in_a_glob_is_ignored() {
        let (dir, _cleanup) = sample();
        assert_eq!(listed(&dir, "./src/*.rs"), ["src/a.rs"]);
        assert_eq!(listed(&dir, "./*.rs"), ["src/a.rs", "src/deep/c.rs"]);
    }

    #[test]
    fn an_unknown_argument_name_produces_a_note() {
        let (dir, _cleanup) = sample();
        let out = list_files(&dir, &json!({"pattern": "*.rs"})).unwrap();
        assert!(
            out.starts_with("[note: unknown argument pattern ignored"),
            "{out}"
        );
        assert!(out.contains("glob"), "{out}");
        assert!(out.contains("README.md"), "{out}");
        let out = grep(
            &dir,
            &json!({"pattern": "fn main", "include": "*.rs", "x": 1}),
        )
        .unwrap();
        assert!(
            out.starts_with("[note: unknown arguments include, x ignored"),
            "{out}"
        );
        assert!(out.contains("src/a.rs:1:fn main"), "{out}");
        // Known arguments make no note.
        let clean = list_files(&dir, &json!({"path": "src", "glob": "*.rs"})).unwrap();
        assert!(!clean.contains("note"), "{clean}");
    }

    #[test]
    fn the_glob_parameter_stays_a_bare_string_in_both_definitions() {
        for tool in [LIST_FILES, GREP] {
            let schema = (tool.parameters)();
            assert_eq!(schema["properties"]["glob"], json!({"type": "string"}));
            assert!(schema["required"]
                .as_array()
                .is_none_or(|required| !required.contains(&json!("glob"))));
        }
    }

    #[test]
    fn a_pathological_glob_finishes_fast() {
        let name = format!("{}c", "a".repeat(200));
        let (dir, _cleanup) = files(&[(name.as_str(), "x\n")]);
        // 16 wildcards (the most allowed), a never-matching tail: exponential in a
        // backtracking matcher.
        let glob = format!("{}b", "a*".repeat(16));
        let started = Instant::now();
        let out = list_files(&dir, &json!({"glob": glob})).unwrap();
        assert!(out.starts_with("no files"), "{out}");
        let deep = format!("{}b", "**/a*".repeat(5));
        list_files(&dir, &json!({"glob": deep})).unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn an_over_long_or_over_wild_glob_is_refused() {
        let (dir, _cleanup) = sample();
        let wild = list_files(&dir, &json!({"glob": "a*".repeat(17)})).unwrap_err();
        assert!(wild.contains("glob") && wild.contains("16"), "{wild}");
        let long = grep(&dir, &json!({"pattern": "fn", "glob": "a".repeat(257)})).unwrap_err();
        assert!(long.contains("glob") && long.contains("256"), "{long}");
        // The limits themselves are allowed.
        list_files(&dir, &json!({"glob": "a*".repeat(16)})).unwrap();
        list_files(&dir, &json!({"glob": "a".repeat(256)})).unwrap();
    }

    #[test]
    fn a_non_string_or_empty_glob_is_handled() {
        let (dir, _cleanup) = sample();
        assert!(list_files(&dir, &json!({"glob": 5}))
            .unwrap_err()
            .contains("glob"));
        // An empty glob means no filter.
        assert_eq!(listed(&dir, "").len(), 4);
    }

    /// Removes its directory on drop, so a failed assertion leaves nothing behind.
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A workspace `repo/` and an outside directory, under a fresh base directory.
    #[cfg(unix)]
    fn confined_sandbox() -> (PathBuf, PathBuf, Cleanup) {
        let base = std::env::temp_dir().join(format!(
            "alfredo-confined-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let cleanup = Cleanup(base.clone());
        let root = base.join("repo");
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::fs::write(root.join("real.txt"), "inside\n").unwrap();
        std::fs::write(base.join("outside/AGENTS.md"), "TOPSECRET-OUTSIDE\n").unwrap();
        (base, root, cleanup)
    }

    #[cfg(unix)]
    #[test]
    fn a_regular_file_opens_and_its_handle_reports_its_size() {
        let (_base, root, _cleanup) = confined_sandbox();
        let (_file, len) = open_confined(&root, &root.join("real.txt")).unwrap();
        assert_eq!(len, "inside\n".len() as u64);
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_swapped_for_a_symlink_after_the_check_is_refused() {
        // `resolve` saw repo/notes/AGENTS.md; the directory is now a link outside.
        let (base, root, _cleanup) = confined_sandbox();
        std::fs::remove_dir(root.join("notes")).unwrap();
        std::os::unix::fs::symlink(base.join("outside"), root.join("notes")).unwrap();
        let result = open_confined(&root, &root.join("notes/AGENTS.md"));
        assert!(
            result.is_err(),
            "outside file opened through a directory link"
        );
        assert!(result
            .err()
            .unwrap()
            .contains("leaves the Coding Workspace"));
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_swapped_for_a_link_into_git_metadata_is_refused_as_excluded() {
        let (_base, root, _cleanup) = confined_sandbox();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::remove_dir(root.join("notes")).unwrap();
        std::os::unix::fs::symlink(root.join(".git"), root.join("notes")).unwrap();
        let error = open_confined(&root, &root.join("notes/HEAD"))
            .err()
            .unwrap();
        assert!(error.contains("excluded"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_as_the_last_component_is_refused_even_inside_the_workspace() {
        let (_base, root, _cleanup) = confined_sandbox();
        std::os::unix::fs::symlink(root.join("real.txt"), root.join("leaf")).unwrap();
        let error = open_confined(&root, &root.join("leaf")).err().unwrap();
        assert!(error.contains("symbolic link"), "{error}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_name_ending_in_deleted_is_refused_as_a_removed_file() {
        // The form Linux reports for a file unlinked after it was opened. The `.pem`
        // suffix match would not see the secret kind behind it.
        let (_base, root, _cleanup) = confined_sandbox();
        let reported = root.join("key.pem (deleted)");
        assert!(crate::chat_tools::confined_name(&root, &reported).is_err());
        assert!(crate::chat_tools::confined_name(&root, &root.join("real.txt")).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_file_removed_after_the_open_is_refused_before_its_name_is_trusted() {
        // Linux names an unlinked file "<path> (deleted)", which the `.pem` suffix
        // match in `secret` would miss. The handle has no links left, so it is refused.
        let (_base, root, _cleanup) = confined_sandbox();
        let key = root.join("key.pem");
        std::fs::write(&key, "PRIVATE KEY\n").unwrap();
        let file = std::fs::File::open(&key).unwrap();
        std::fs::remove_file(&key).unwrap();
        let error = crate::chat_tools::confined_handle(&root, &file).err();
        assert!(error.is_some(), "a removed .pem handle was accepted");
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_at_once_instead_of_blocking_the_open() {
        let (_base, root, _cleanup) = confined_sandbox();
        let fifo = root.join("pipe");
        let c_path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = open_confined(&root, &fifo).err();
            let _ = sender.send(result);
        });
        // A blocking open would never answer; the test then fails rather than hangs.
        let result = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("opening a FIFO blocked");
        assert_eq!(result.as_deref(), Some("not a regular file"));
    }

    #[test]
    fn a_grep_cancelled_before_or_during_the_scan_ends_at_the_next_file() {
        // 40 files of 1 MiB, no match: a full scan would read all of them.
        let dir = tree(40, 1 << 20);
        let _cleanup = Cleanup(dir.clone());
        let owner = budget::CancelOnDrop::default();
        let round = budget::Budget::new(Duration::from_secs(300), owner.flag());
        // The flag is set before the scan starts; the first file check must see it.
        drop(owner);
        let started = Instant::now();
        let out = budget::within(&round, || {
            grep_within(
                &dir,
                &json!({"pattern": "absent-text"}),
                u64::MAX,
                Duration::from_secs(300),
            )
        });
        assert_eq!(out, Err("cancelled".to_string()));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
