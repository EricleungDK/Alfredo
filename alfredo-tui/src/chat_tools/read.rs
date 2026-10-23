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
    description: "List files (recursively) in the Coding Workspace or one of its directories, one relative path per line. Skips .git, secrets (.env*, *.key, *.pem) and generated or vendor trees.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Directory relative to the repository root (default: the root)"}
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
                "ignore_case": {"type": "boolean", "description": "Match regardless of case (default false)"}
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

/// Regular files under `start` in sorted order; symlinks are never followed.
/// Returns the files and whether the walk stopped early.
fn walk(start: &Path, limit: usize, skip_noise: bool) -> (Vec<PathBuf>, bool) {
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
            if excluded(&name) || (skip_noise && noise(&name.to_lowercase())) {
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

/// Files chosen for discovery under `start` (an existing path inside `root`), in the
/// order [`walk`] yields, whether the listing stopped early, and a note when `start` is
/// itself ignored. In a git workspace git's ignore rules decide (tracked files stay);
/// otherwise, or when git cannot list, the plain [`walk`] is the fallback. [`excluded`]
/// and [`noise`] always apply on top, so ignore files can only hide more.
fn discover(root: &Path, start: &Path, limit: usize) -> (Vec<PathBuf>, bool, Option<String>) {
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
        let (files, cut) = walk(start, limit, false);
        let note = format!("note: {rel} is ignored (by .gitignore or as a virtualenv or cache directory); listed because you named it");
        return (files, cut, Some(note));
    }
    let Ok(listing) = git_read::list_unignored(root, &rel, excludes.as_deref(), MAX_LISTING) else {
        let (files, cut) = walk(start, limit, true);
        return (files, cut, None);
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

fn list_files(root: &Path, arguments: &Value) -> Result<String, String> {
    let path = text_arg(arguments, "path")?.unwrap_or(".");
    let full = resolve(root, path)?;
    if full.is_file() {
        return Ok(display(root, &full));
    }
    let (files, cut, note) = discover(root, &full, MAX_ENTRIES);
    let mut out = note.map(|note| format!("[{note}]\n")).unwrap_or_default();
    if files.is_empty() {
        out.push_str("no files");
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
    let path = text_arg(arguments, "path")?.unwrap_or(".");
    let full = resolve(root, path)?;
    let (files, mut cut, note) = if full.is_file() {
        (vec![full], false, None)
    } else {
        discover(root, &full, MAX_VISITS)
    };
    let mut out = note.map(|note| format!("[{note}]\n")).unwrap_or_default();
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
