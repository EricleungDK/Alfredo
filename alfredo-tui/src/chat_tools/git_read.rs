//! Read tier git tools (issue #234): git_status, git_branch, git_log, git_diff on the
//! Coding Workspace. Strictly read-only: no fetch, no optional locks (the index is not
//! refreshed), hooks and fsmonitor off, and model arguments cannot become options
//! (refs starting with `-` are refused, `--end-of-options` and `--` separate them).
use super::{excluded, text_arg, Tier, Tool, MAX_OUTPUT};
use serde_json::{json, Value};
use std::{
    io::Read,
    path::Path,
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

/// Commits git_log shows by default and at most.
const DEFAULT_COMMITS: u64 = 20;
const MAX_COMMITS: u64 = 200;
/// Longest ref or path argument accepted.
const MAX_ARG: usize = 256;
/// Secrets never shown in a diff (the `@path` secret exclusions, any depth, any case).
const SECRET_PATHSPECS: [&str; 5] = [
    ":(exclude,glob,icase)**/.env*",
    ":(exclude,glob,icase)**/*.pem",
    ":(exclude,glob,icase)**/*.key",
    ":(exclude,glob,icase)**/*.p12",
    ":(exclude,glob,icase)**/*.pfx",
];

pub const GIT_STATUS: Tool = Tool {
    name: "git_status",
    tier: Tier::Read,
    description: "Show the Coding Workspace git status: current branch (with upstream ahead/behind) and changed, staged and untracked files in short format (XY path).",
    parameters: || json!({"type": "object", "properties": {}}),
    run: git_status,
};

pub const GIT_BRANCH: Tool = Tool {
    name: "git_branch",
    tier: Tier::Read,
    description: "Name the current git branch (first line `current: NAME`, or `current: detached at SHA`) and list local branches, including alfredo/go-* branches made by /go, with their last commit.",
    parameters: || json!({"type": "object", "properties": {}}),
    run: git_branch,
};

pub const GIT_LOG: Tool = Tool {
    name: "git_log",
    tier: Tier::Read,
    description: "List recent commits (short sha, date, author, subject), newest first, of HEAD or a given branch or ref.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "ref": {"type": "string", "description": "Branch or ref to list, e.g. main or alfredo/go-12 (default HEAD)"},
                "max_count": {"type": "integer", "description": "Commits to show (default 20, at most 200)"}
            }
        })
    },
    run: git_log,
};

pub const GIT_DIFF: Tool = Tool {
    name: "git_diff",
    tier: Tier::Read,
    description: "Show a git diff (stat then patch). No arguments: unstaged working-tree changes. staged=true: staged changes. base only: base vs the working tree. base and target: what target adds since it branched from base (base...target), e.g. base=main target=alfredo/go-12. Secret files are left out.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "base": {"type": "string", "description": "Branch or ref to compare from, e.g. main"},
                "target": {"type": "string", "description": "Branch or ref to compare to, e.g. alfredo/go-12 (needs base)"},
                "staged": {"type": "boolean", "description": "Diff staged changes instead of the working tree (default false)"},
                "path": {"type": "string", "description": "Limit to this file or directory, relative to the repository root"}
            }
        })
    },
    run: git_diff,
};

fn git_status(root: &Path, _: &Value) -> Result<String, String> {
    refuse_filters(root)?;
    let output = git(
        root,
        &["status", "--short", "--branch", "--untracked-files=normal"],
    )?;
    Ok(if output.lines().count() <= 1 {
        format!("{output}(clean: no changes)\n")
    } else {
        output
    })
}

fn git_branch(root: &Path, _: &Value) -> Result<String, String> {
    let current = match git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        Ok(name) => name.trim().to_string(),
        Err(_) => format!(
            "detached at {}",
            git(root, &["rev-parse", "--short", "HEAD"])?.trim()
        ),
    };
    let list = git(
        root,
        &[
            "branch",
            "--no-color",
            "--list",
            "--format=%(HEAD) %(refname:short) %(objectname:short) %(subject)",
        ],
    )?;
    Ok(format!("current: {current}\n{list}"))
}

fn git_log(root: &Path, arguments: &Value) -> Result<String, String> {
    let reference = ref_arg(arguments, "ref")?.unwrap_or("HEAD");
    let count = match arguments.get("max_count") {
        None | Some(Value::Null) => DEFAULT_COMMITS,
        Some(value) => value
            .as_u64()
            .filter(|count| *count > 0)
            .ok_or("argument max_count must be a positive integer")?
            .min(MAX_COMMITS),
    };
    let count = format!("--max-count={count}");
    git(
        root,
        &[
            "log",
            "--no-color",
            "--no-show-signature",
            "--date=short",
            "--format=%h %ad %an: %s",
            &count,
            "--end-of-options",
            reference,
            "--",
        ],
    )
}

fn git_diff(root: &Path, arguments: &Value) -> Result<String, String> {
    let base = ref_arg(arguments, "base")?;
    let target = ref_arg(arguments, "target")?;
    let staged = match arguments.get("staged") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(staged)) => *staged,
        Some(_) => return Err("argument staged must be true or false".into()),
    };
    let path = path_arg(arguments)?;
    let mut selector: Vec<String> = Vec::new();
    if staged {
        selector.push("--cached".into());
    }
    match (base, target) {
        (Some(_), Some(_)) if staged => return Err("staged cannot be combined with target".into()),
        (Some(base), Some(target)) => {
            selector.extend(["--end-of-options".into(), format!("{base}...{target}")]);
        }
        (Some(base), None) => selector.extend(["--end-of-options".into(), base.into()]),
        (None, Some(_)) => return Err("target needs base (e.g. base=main)".into()),
        (None, None) => {}
    }
    if target.is_none() {
        refuse_filters(root)?;
    }
    let selector: Vec<&str> = selector.iter().map(String::as_str).collect();
    // Files renamed or copied from a secret are secret too: find them across the
    // whole tree, then leave them out of the (rename-free) diff.
    let mut detect = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--name-status",
        "-z",
        "--find-renames",
        "--find-copies",
    ];
    detect.extend(&selector);
    detect.push("--");
    let derived: Vec<String> = secret_copies(&git_limited(root, &detect, 4 << 20)?)
        .into_iter()
        .map(|path| format!(":(exclude,literal){path}"))
        .collect();
    let mut args = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--stat",
        "--patch",
    ];
    args.extend(&selector);
    args.push("--");
    let literal;
    match path {
        Some(path) => {
            literal = format!(":(literal){path}");
            args.push(&literal);
        }
        None => args.push("."),
    }
    args.extend(SECRET_PATHSPECS);
    args.extend(derived.iter().map(String::as_str));
    let output = git(root, &args)?;
    Ok(if output.is_empty() {
        "(no differences)\n".into()
    } else {
        output
    })
}

/// Targets of renames and copies whose source is excluded, from `--name-status -z`.
fn secret_copies(listing: &str) -> Vec<String> {
    let mut fields = listing.split('\0');
    let mut found = Vec::new();
    while let Some(status) = fields.next() {
        if status.starts_with(['R', 'C']) {
            let (Some(source), Some(target)) = (fields.next(), fields.next()) else {
                break;
            };
            if excluded(source) && !excluded(target) {
                found.push(target.to_string());
            }
        } else if fields.next().is_none() {
            break;
        }
    }
    found
}

/// Optional ref argument that cannot be read as an option, a path or a blob.
pub(crate) fn ref_arg<'a>(arguments: &'a Value, name: &str) -> Result<Option<&'a str>, String> {
    let Some(value) = text_arg(arguments, name)?.map(str::trim) else {
        return Ok(None);
    };
    let safe = |c: char| c.is_ascii_alphanumeric() || "._/-~^@{}+".contains(c);
    if value.is_empty()
        || value.len() > MAX_ARG
        || value.starts_with('-')
        || value.contains("..")
        || !value.chars().all(safe)
    {
        return Err(format!(
            "{name} {:?} is not a plain branch or ref name",
            value.chars().take(64).collect::<String>()
        ));
    }
    Ok(Some(value))
}

/// Optional repository-relative path: no options, pathspec magic, `..` or exclusions.
pub(crate) fn path_arg(arguments: &Value) -> Result<Option<String>, String> {
    let Some(path) = text_arg(arguments, "path")?.map(str::trim) else {
        return Ok(None);
    };
    let path = path.trim_start_matches("./").trim_end_matches('/');
    if path.is_empty() || path == "." {
        return Ok(None);
    }
    let unsafe_path = path.len() > MAX_ARG
        || path.starts_with(['-', ':', '/'])
        || path.chars().any(char::is_control)
        || path.split('/').any(|part| part == ".." || part.is_empty());
    if unsafe_path {
        return Err(format!(
            "path {:?} must be relative to the Coding Workspace and stay inside it",
            path.chars().take(64).collect::<String>()
        ));
    }
    if excluded(path) {
        return Err(format!(
            "{path} is excluded (Git metadata, secrets or generated files)"
        ));
    }
    Ok(Some(path.to_string()))
}

/// Run read-only git in `root`; stdout is read up to just past [`MAX_OUTPUT`] (the
/// dispatcher caps it with the marker), then git is stopped.
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    git_limited(root, args, MAX_OUTPUT * 2)
}

/// Refuse when the repository config defines filter drivers or includes: `git status`
/// and working-tree diffs would run the repository's clean filters (as worker host git).
pub(crate) fn refuse_filters(root: &Path) -> Result<(), String> {
    let config = git_limited(root, &crate::worker::REPOSITORY_CONFIG_ARGS, 1 << 20)?;
    if crate::worker::config_needs_qualification(&config) {
        return Err("repository config defines filter drivers or includes; git status and working-tree diffs are not run (git_log and git_diff with base and target still work)".into());
    }
    Ok(())
}

/// [`git`] reading at most `limit` bytes of stdout.
pub(crate) fn git_limited(root: &Path, args: &[&str], limit: usize) -> Result<String, String> {
    let ceiling = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    let ceiling = ceiling.parent().unwrap_or(&ceiling).to_path_buf();
    let mut command = Command::new("/usr/bin/git");
    hardened(&mut command)
        .env("GIT_CEILING_DIRECTORIES", ceiling)
        .arg("-C")
        .arg(root)
        .args(args);
    let (status, out, error) = run_bounded(command, limit, DEADLINE)?;
    match status {
        Some(status) if !status.success() => {
            let first = error.lines().find(|line| !line.trim().is_empty());
            Err(match first {
                Some(line) => line.trim().trim_start_matches("fatal: ").to_string(),
                None => format!("git {} failed", args[0]),
            })
        }
        _ => Ok(String::from_utf8_lossy(&out).into_owned()),
    }
}

/// Longest one git call may run.
pub(crate) const DEADLINE: Duration = Duration::from_secs(15);

/// Run `command`, reading at most `limit` bytes of stdout. Stops it once the limit is
/// reached (status `None`: output was cut) or `deadline` passes (an error).
pub(crate) fn run_bounded(
    mut command: Command,
    limit: usize,
    deadline: Duration,
) -> Result<(Option<ExitStatus>, Vec<u8>, String), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start git: {error}"))?;
    let stderr = child.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.take(4096).read_to_string(&mut text);
            text
        })
    });
    let stdout = child.stdout.take().map(|stdout| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = stdout.take(limit as u64).read_to_end(&mut out);
            out
        })
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break Some(status);
        }
        if stdout.as_ref().is_some_and(|thread| thread.is_finished()) {
            // Limit reached (or stdout closed): give a closing git a moment, then stop it.
            std::thread::sleep(Duration::from_millis(20));
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                break Some(status);
            }
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        if started.elapsed() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "git took longer than {} s and was stopped",
                deadline.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let out = stdout
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default();
    let error = stderr
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default();
    let status = status.filter(|_| out.len() < limit);
    Ok((status, out, error))
}

/// The clean environment and command-line overrides every git call here uses.
pub(crate) fn hardened(command: &mut Command) -> &mut Command {
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_PAGER", "cat")
        .arg("--no-optional-locks")
        .arg("--no-pager")
        .args(crate::worker::HARDENED_GIT_ARGS)
        .args(["-c", "diff.external=", "-c", "core.pager=cat"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_past_its_deadline_is_stopped_with_an_error() {
        let mut command = Command::new("/bin/sleep");
        command.arg("5");
        let started = Instant::now();
        let error = run_bounded(command, 1024, Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("stopped"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn output_past_the_limit_stops_the_command() {
        let mut command = Command::new("/usr/bin/yes");
        let (status, out, _) = {
            command.arg("x");
            run_bounded(command, 4096, DEADLINE)
        }
        .unwrap();
        assert!(status.is_none());
        assert_eq!(out.len(), 4096);
    }
}
