//! Read tier git tools (issue #234): git_status, git_branch, git_log, git_diff on the
//! Coding Workspace. Strictly read-only: no fetch, no optional locks (the index is not
//! refreshed), hooks and fsmonitor off, and model arguments cannot become options
//! (refs starting with `-` are refused, `--end-of-options` and `--` separate them).
use super::{excluded, text_arg, Tier, Tool, MAX_OUTPUT};
use serde_json::{json, Value};
use std::os::unix::fs::FileTypeExt;
use std::{
    cell::Cell,
    collections::HashSet,
    fs,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

/// Commits git_log shows by default and at most.
const DEFAULT_COMMITS: u64 = 20;
const MAX_COMMITS: u64 = 200;
/// Longest ref or path argument accepted.
const MAX_ARG: usize = 256;
/// Largest name listing `git_diff` checks for secrets; a longer one is refused, not cut.
const DIFF_NAMES: usize = 4 << 20;

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

/// The tool call's git work runs under one budget (issue #359): see [`call_scope`].
fn git_status(root: &Path, arguments: &Value) -> Result<String, String> {
    call_scope(DEADLINE, || status_output(root, arguments))
}

fn status_output(root: &Path, _: &Value) -> Result<String, String> {
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

fn git_branch(root: &Path, arguments: &Value) -> Result<String, String> {
    call_scope(DEADLINE, || branch_output(root, arguments))
}

fn branch_output(root: &Path, _: &Value) -> Result<String, String> {
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
    call_scope(DEADLINE, || log_output(root, arguments))
}

fn log_output(root: &Path, arguments: &Value) -> Result<String, String> {
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
    call_scope(DEADLINE, || diff_output(root, arguments))
}

fn diff_output(root: &Path, arguments: &Value) -> Result<String, String> {
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
    // whole tree, then leave them out of the (rename-free) diff. The listing is read
    // whole (git_limited): a cut listing would leave a secret copy out of the exclusions.
    let mut detect = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--name-status",
        "-z",
        "--find-renames",
        "--find-copies",
        "--find-copies-harder",
        // No rename limit: past it git stops inexact copy detection and only warns.
        "-l0",
    ];
    detect.extend(&selector);
    detect.push("--");
    // Every changed name in this selector (whole tree, any mode) that is a secret is
    // left out by name, so the patch never carries its content.
    let listing = git_complete_listing(root, &detect, DIFF_NAMES)?;
    if listing.len() >= DIFF_NAMES {
        return Err(
            "too many changed files to check for secret files; narrow with path or a smaller range"
                .into(),
        );
    }
    // Names are read lossily: a replacement character may hide the bytes that made a
    // name secret, so such a name is refused rather than matched.
    if listing.contains('\u{FFFD}') {
        return Err("a changed file name is not UTF-8; git_diff is not run".into());
    }
    let hidden: Vec<String> = secret_names(&listing)?
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
    args.extend(hidden.iter().map(String::as_str));
    let output = git(root, &args)?;
    Ok(if output.is_empty() {
        "(no differences)\n".into()
    } else {
        output
    })
}

/// Changed paths that are secret files, from `--name-status -z`. A rename or copy
/// counts as a whole: if either name is a secret, both names are left out, because the
/// content moved out of or into a secret file. A malformed listing is an error.
fn secret_names(listing: &str) -> Result<Vec<String>, String> {
    let malformed =
        || "git diff listed names that could not be read; nothing was shown".to_string();
    let mut fields = listing.split('\0');
    let mut found = Vec::new();
    while let Some(status) = fields.next() {
        if status.is_empty() {
            continue;
        }
        let names: Vec<&str> = if status.starts_with(['R', 'C']) {
            let (Some(source), Some(target)) = (fields.next(), fields.next()) else {
                return Err(malformed());
            };
            vec![source, target]
        } else {
            vec![fields.next().ok_or_else(malformed)?]
        };
        if names
            .iter()
            .any(|name| crate::planning_context::secret(name))
        {
            found.extend(names.into_iter().map(String::from));
        }
    }
    Ok(found)
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

/// Run read-only git in `root`; stdout is read up to twice [`MAX_OUTPUT`] and then git is
/// stopped. Display only: a listing past that is shown cut (the dispatcher caps it again
/// with the marker). Callers either show the output or need a small value (a ref, a count,
/// a remote name) or its emptiness, which a cut cannot flip (audited for #308). A caller
/// that decides on a whole listing uses [`git_limited`], which fails closed.
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    git_output(root, args, MAX_OUTPUT * 2).map(|(_, output)| output)
}

/// Config keys that can run a filter driver, pull in another config file, move the working
/// tree outside the workspace (`core.worktree`, #309), or make git lazy-fetch through a
/// partial clone's promisor and its transport program (#467), in every scope git reads
/// (includes are followed; no scope flag, as `REPOSITORY_CONFIG_ARGS`). git prints only the
/// matching key names, so a large config cannot push one out of the output the way it cuts
/// a full `config --list` (#308). `remote\..+` spans subsections that hold `=`, as
/// `qualification_key` does.
const FILTER_KEYS_ARGS: [&str; 4] = [
    "config",
    "--name-only",
    "--get-regexp",
    "^(filter|include|includeif)\\.|^core\\.(worktree|sshcommand|gitproxy)$|^extensions\\.partialclone$|^remote\\..+\\.(promisor|uploadpack|receivepack)$",
];

/// Most bytes of matching key names read; a listing past it is refused.
const FILTER_KEYS_LIMIT: usize = 64 << 10;

/// Refuse when the repository config defines filter drivers or includes: `git status`
/// and working-tree diffs would run the repository's clean filters (as worker host git).
/// Fails closed: a listing that cannot be read whole is refused, never trusted as a prefix.
pub(crate) fn refuse_filters(root: &Path) -> Result<(), String> {
    refuse_filters_within(root, FILTER_KEYS_LIMIT, call_clock())
}

/// [`refuse_filters`] with the listing read up to `limit` bytes within `deadline`. A cut or
/// late listing is an error (run_bounded stops git), so the gate never trusts a prefix.
fn refuse_filters_within(root: &Path, limit: usize, clock: Clock) -> Result<(), String> {
    const NOT_RUN: &str = "git status and working-tree diffs are not run (git_log and git_diff with base and target still work)";
    let command = git_command_within(root, &FILTER_KEYS_ARGS, clock)?;
    let (status, out, error) = run_bounded(command, limit, clock.left())?;
    match status {
        None => Err(format!(
            "repository config is too large to check for filter drivers; {NOT_RUN}"
        )),
        // Every listed key is a qualifying key, so any output refuses; the first three are named.
        Some(status) if status.success() => {
            let listing = String::from_utf8_lossy(&out);
            let keys: Vec<&str> = listing.lines().collect();
            if keys.is_empty() {
                return Ok(());
            }
            let named: Vec<String> = keys
                .iter()
                .take(3)
                .map(|key| format!("{:?}", key.chars().take(96).collect::<String>()))
                .collect();
            let more = match keys.len().saturating_sub(3) {
                0 => String::new(),
                count => format!(" and {count} more"),
            };
            Err(format!(
                "repository config sets {}{more}, which git would run or follow (filter drivers, includes, core.worktree, partial-clone or transport settings); {NOT_RUN}",
                named.join(", ")
            ))
        }
        // `--get-regexp` exits 1 when no key matches; a broken config exits 128.
        Some(status) if status.code() == Some(1) && out.is_empty() => Ok(()),
        Some(_) => {
            let first = error
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("git config failed");
            Err(format!(
                "repository config cannot be read ({}); {NOT_RUN}",
                first.trim().trim_start_matches("fatal: ")
            ))
        }
    }
}

/// [`git`] reading at most `limit` bytes of stdout. A listing past `limit` is an error,
/// never a prefix: a caller that decides on the listing (a secret check, a gate) must see
/// all of it or nothing.
pub(crate) fn git_limited(root: &Path, args: &[&str], limit: usize) -> Result<String, String> {
    match git_output(root, args, limit)? {
        (false, output) => Ok(output),
        (true, _) => Err(format!(
            "git {} output is larger than {limit} bytes and cannot be checked; refused",
            args[0]
        )),
    }
}

/// Git's warnings that its copy or rename search was cut short by the rename limit
/// (the listing then misses copies and renames). Matched against stderr lines.
const CUT_SEARCH_WARNINGS: [&str; 3] = [
    "only found copies from modified paths",
    "inexact rename detection was skipped",
    "diff.renameLimit",
];

/// [`git_limited`] for a listing that decides a secret check. A copy or rename search that
/// git cut short (see [`CUT_SEARCH_WARNINGS`]) still exits 0 with an incomplete listing, so
/// that stderr refuses it; other stderr is ignored.
pub(crate) fn git_complete_listing(
    root: &Path,
    args: &[&str],
    limit: usize,
) -> Result<String, String> {
    let clock = call_clock();
    let (status, out, error) =
        run_bounded(git_command_within(root, args, clock)?, limit, clock.left())?;
    let Some(status) = status else {
        return Err(format!(
            "git {} output is larger than {limit} bytes and cannot be checked; refused",
            args[0]
        ));
    };
    if !status.success() {
        let first = error.lines().find(|line| !line.trim().is_empty());
        return Err(match first {
            Some(line) => line.trim().trim_start_matches("fatal: ").to_string(),
            None => format!("git {} failed", args[0]),
        });
    }
    // Only git's copy and rename search warnings mean the listing is incomplete. Other
    // stderr (line-ending notes under core.autocrlf, for example) does not change it.
    if let Some(line) = error.lines().find(|line| {
        CUT_SEARCH_WARNINGS
            .iter()
            .any(|warning| line.contains(warning))
    }) {
        return Err(format!(
            "git {} cut its copy or rename search short, so its listing is incomplete ({}); refused",
            args[0],
            line.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Run git in `root`, stdout read up to `limit` bytes. The flag is whether the output was
/// cut at `limit`; a git that fails is an error with its first message line.
fn git_output(root: &Path, args: &[&str], limit: usize) -> Result<(bool, String), String> {
    let clock = call_clock();
    read_output(
        git_command_within(root, args, clock)?,
        args[0],
        limit,
        clock.left(),
    )
}

/// Run `command` (git, named `verb` in messages), stdout read up to `limit` bytes within
/// `deadline`; see [`git_output`].
fn read_output(
    command: Command,
    verb: &str,
    limit: usize,
    deadline: Duration,
) -> Result<(bool, String), String> {
    let (status, out, error) = run_bounded(command, limit, deadline)?;
    let cut = status.is_none();
    if status.is_some_and(|status| !status.success()) {
        let first = error.lines().find(|line| !line.trim().is_empty());
        return Err(match first {
            Some(line) => line.trim().trim_start_matches("fatal: ").to_string(),
            None => format!("git {verb} failed"),
        });
    }
    Ok((cut, String::from_utf8_lossy(&out).into_owned()))
}

/// One call's time budget, shared by its verification and its git run, so a call never takes
/// longer than its deadline in all (issue #359).
#[derive(Clone, Copy)]
pub(crate) struct Clock {
    started: Instant,
    deadline: Duration,
}

impl Clock {
    pub(crate) fn start(deadline: Duration) -> Self {
        Self {
            started: Instant::now(),
            deadline,
        }
    }

    /// The budget not yet spent, for the next git process of the call.
    pub(crate) fn left(&self) -> Duration {
        self.deadline.saturating_sub(self.started.elapsed())
    }
}

thread_local! {
    /// The budget of the tool call running on this thread (issue #359, one per call).
    static CALL_CLOCK: Cell<Option<Clock>> = const { Cell::new(None) };
}

/// Clears the running call's budget when the call ends, also when it unwinds.
struct LeaveCallScope;

impl Drop for LeaveCallScope {
    fn drop(&mut self) {
        CALL_CLOCK.with(|clock| clock.set(None));
    }
}

/// Runs one tool call (or one approval step of a git write) under one budget: every
/// verification and git process it starts draws on `deadline` together, so the call as a
/// whole stops at it. A call already inside a scope keeps the outer budget.
pub(crate) fn call_scope<T>(deadline: Duration, run: impl FnOnce() -> T) -> T {
    if CALL_CLOCK.with(Cell::get).is_some() {
        return run();
    }
    CALL_CLOCK.with(|clock| clock.set(Some(Clock::start(deadline))));
    let _leave = LeaveCallScope;
    run()
}

/// The budget of the running call; a lone git call outside any call gets a fresh one.
pub(crate) fn call_clock() -> Clock {
    CALL_CLOCK
        .with(Cell::get)
        .unwrap_or_else(|| Clock::start(DEADLINE))
}

/// `git -C root ARGS` with the hardened environment, pinned to the git directory that
/// [`verified_git_dir`] checked (issue #359). The verification draws on `clock`.
fn git_command_within(root: &Path, args: &[&str], clock: Clock) -> Result<Command, String> {
    let verified = verified_git_dir_within(root, clock, Config::Hardened)?;
    let mut command = hardened_command(root)?;
    pin_git_dir(&mut command, root, &verified);
    command.args(args);
    Ok(command)
}

/// `git -C root` with the hardened environment; discovery stops above the parent. Not
/// pinned: only [`verified_git_dir`] runs it unpinned, to learn where the repository is.
fn hardened_command(root: &Path) -> Result<Command, String> {
    let ceiling = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    let ceiling = ceiling.parent().unwrap_or(&ceiling).to_path_buf();
    let mut command = Command::new("/usr/bin/git");
    hardened(&mut command)
        .env("GIT_CEILING_DIRECTORIES", ceiling)
        .arg("-C")
        .arg(root);
    Ok(command)
}

/// Tracked and untracked-but-not-ignored files under `rel` (a workspace-relative `/` path,
/// empty for the whole workspace), as git lists them: `.gitignore` files at any depth,
/// `.git/info/exclude` and the global excludes file (`excludes`; the hardened environment
/// hides the user's home, so it is passed in) apply, tracked files stay listed. Discovery
/// only: the caller still applies its own exclusions and checks each path on disk. Fails
/// closed: repository filter drivers, a failing git or a listing past `limit` bytes is an
/// error and the caller falls back to its own walk.
pub(crate) fn list_unignored(
    root: &Path,
    rel: &str,
    excludes: Option<&Path>,
    limit: usize,
) -> Result<Vec<String>, String> {
    refuse_filters(root)?;
    let config = excludes.map(|path| format!("core.excludesFile={}", path.display()));
    let mut args = vec!["--literal-pathspecs"];
    if let Some(config) = &config {
        args.extend(["-c", config]);
    }
    args.extend([
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
    ]);
    if !rel.is_empty() {
        args.extend(["--", rel]);
    }
    let out = git_limited(root, &args, limit)?;
    Ok(out
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect())
}

/// Whether git ignores the untracked path `rel` (a tracked path is never ignored).
pub(crate) fn is_ignored(root: &Path, rel: &str, excludes: Option<&Path>) -> Result<bool, String> {
    let config = excludes.map(|path| format!("core.excludesFile={}", path.display()));
    // check-ignore rejects `--literal-pathspecs`; the `./` prefix keeps a directory named
    // like pathspec magic (`:(top)`) a plain path.
    let rel = format!("./{rel}");
    let mut args = vec![];
    if let Some(config) = &config {
        args.extend(["-c", config]);
    }
    args.extend(["check-ignore", "-q", "--", &rel]);
    let clock = call_clock();
    let command = git_command_within(root, &args, clock)?;
    let (status, _, error) = run_bounded(command, 1024, clock.left())?;
    match status.and_then(|status| status.code()) {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(error
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("git check-ignore failed")
            .to_string()),
    }
}

/// The user's global git excludes file: `core.excludesFile` of the global config, else
/// git's default `$XDG_CONFIG_HOME/git/ignore` or `~/.config/git/ignore`. Only if it
/// exists. It can only hide more files from discovery, never show one.
pub(crate) fn global_excludes() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let mut command = Command::new("/usr/bin/git");
    hardened(&mut command)
        .env("HOME", &home)
        .env_remove("GIT_CONFIG_GLOBAL")
        .args([
            "config",
            "--global",
            "--type=path",
            "--get",
            "core.excludesFile",
        ]);
    let configured = match run_bounded(command, 4096, DEADLINE) {
        Ok((Some(status), out, _)) if status.success() => {
            String::from_utf8_lossy(&out).trim().to_string()
        }
        _ => String::new(),
    };
    let path = if configured.is_empty() {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
            .join("git/ignore")
    } else {
        PathBuf::from(configured)
    };
    (path.is_absolute() && path.is_file()).then_some(path)
}

/// The git directories of the Coding Workspace's repository, verified by
/// [`verified_git_dir`] (issue #359).
pub(crate) struct VerifiedGit {
    /// The directory git works in: `<root>/.git`, or `<common>/worktrees/<name>` of a linked
    /// worktree.
    pub(crate) git_dir: PathBuf,
    /// `git_dir` itself, or the main repository's `.git` for a linked worktree.
    pub(crate) common_dir: PathBuf,
}

/// Refusal when the workspace's repository is not its own (issue #359).
const OUTSIDE: &str = "the workspace's .git points outside the Coding Workspace (its git directory is neither the workspace's own nor a linked worktree of it); git tools refuse";

/// Where git keeps the repository of the Coding Workspace at `root`, verified before any
/// git call trusts it (issue #359). Accepted: `root/.git` as a real directory that is both
/// the git and the common directory; or `root/.git` as a gitfile to a linked worktree
/// `<common>/worktrees/<name>`, where `<common>` is a directory and the entry's `gitdir`
/// file points back at `root/.git`. Anything else, including a `commondir` file that
/// leads elsewhere, is refused. Paths are compared canonically, so symlinks, `..`, relative
/// back-references and whitespace in the gitfile cannot hide another repository.
pub(crate) fn verified_git_dir(root: &Path, config: Config) -> Result<VerifiedGit, String> {
    verified_git_dir_within(root, call_clock(), config)
}

/// The git configuration a check runs under: the read tier's hardened one, or a write's,
/// which also reads the owner's global config (identity and `safe.directory`).
#[derive(Clone, Copy)]
pub(crate) enum Config {
    Hardened,
    Owner,
}

/// [`verified_git_dir`] whose one git call draws on `clock`. A repository config that blocks
/// git (a FIFO include) is then an error within the caller's budget.
pub(crate) fn verified_git_dir_within(
    root: &Path,
    clock: Clock,
    config: Config,
) -> Result<VerifiedGit, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    // Before git runs at all: git opens these files itself while it locates the repository.
    refuse_special_start_up_files(&root, clock)?;
    let mut command = hardened_command(&root)?;
    if matches!(config, Config::Owner) {
        owner_config(&mut command);
    }
    command.args([
        "rev-parse",
        "--path-format=absolute",
        "--absolute-git-dir",
        "--git-common-dir",
    ]);
    let (cut, listing) = read_output(command, "rev-parse", 64 << 10, clock.left())?;
    let lines: Vec<&str> = listing.lines().collect();
    let [git_dir, common_dir] = lines[..] else {
        return Err(format!("cannot locate the repository; {OUTSIDE}"));
    };
    if cut {
        return Err(format!("cannot locate the repository; {OUTSIDE}"));
    }
    let git_dir = canonical_dir(git_dir)?;
    let common_dir = canonical_dir(common_dir)?;
    let dot_git = root.join(".git");
    let kind = fs::symlink_metadata(&dot_git).map_err(|_| OUTSIDE.to_string())?;
    let dot_git = dot_git.canonicalize().map_err(|_| OUTSIDE.to_string())?;
    let verified = if kind.is_dir() {
        // A plain repository: nothing redirects it (a `commondir` file would).
        git_dir == dot_git && common_dir == dot_git
    } else if kind.is_file() {
        // A linked worktree: its entry sits in the main repository's worktrees directory,
        // and the entry points back at this workspace. The main repository is not inside
        // the workspace: a repository the chat can write to is not the owner's.
        let entry_of_main = git_dir != common_dir
            && !common_dir.starts_with(&root)
            && git_dir.parent() == Some(common_dir.join("worktrees").as_path());
        let back = read_control_file(&git_dir.join("gitdir"), clock, OUTSIDE)?
            .ok_or_else(|| OUTSIDE.to_string())?;
        let back = git_dir.join(back.trim_end());
        entry_of_main && back.canonicalize().is_ok_and(|back| back == dot_git)
    } else {
        false
    };
    if !verified {
        return Err(OUTSIDE.to_string());
    }
    // Nothing in the git directory, the common directory, refs or info may be a link, and
    // every object store git reads (its own and the alternates it names) is walked for links
    // and kept inside the repository: a link or a borrowed store shows another repository.
    walk_repository(&git_dir, &common_dir, clock)?;
    Ok(VerifiedGit {
        git_dir,
        common_dir,
    })
}

const LINKED: &str = "the workspace's repository holds a symbolic link in its git directory, refs or object store; git tools refuse";
const UNCHECKED: &str = "the workspace's repository could not be checked for links or borrowed objects within the call's time limit; git tools refuse";
const UNREADABLE: &str = "the workspace's repository changed or could not be read while it was checked for links or borrowed objects (a background git gc or repack may have been running); git tools refuse, try again";
const ALTERNATES: &str = "the workspace's repository borrows objects from outside it (objects/info/alternates); git tools refuse";

/// Refuses a symbolic link, FIFO, socket or device among the entries of `dir`, or, when
/// `recursive`, anywhere under it. A missing directory is fine. The walk is charged to `clock`: running out is a refusal,
/// never a partial check (issue #359).
fn walk_without_links(dir: &Path, clock: Clock, recursive: bool) -> Result<(), String> {
    let unreadable = || UNREADABLE.to_string();
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => return Err(LINKED.to_string()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(unreadable()),
    }
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if clock.left().is_zero() {
            return Err(UNCHECKED.to_string());
        }
        for entry in fs::read_dir(&dir).map_err(|_| unreadable())? {
            let entry = entry.map_err(|_| unreadable())?;
            let kind = entry.file_type().map_err(|_| unreadable())?;
            if kind.is_symlink() {
                return Err(LINKED.to_string());
            }
            // A FIFO, socket or device blocks any git process that opens it (issue #461).
            // The one exception: git's fsmonitor daemon socket directly in a git directory
            // (opening a socket fails, it cannot block; Alfredo's git runs without fsmonitor).
            let daemon =
                !recursive && kind.is_socket() && entry.file_name() == "fsmonitor--daemon.ipc";
            if !kind.is_file() && !kind.is_dir() && !daemon {
                return Err(special(&entry.file_name().to_string_lossy()));
            }
            if recursive && kind.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

/// Every object store the repository reads: its own, and each store its `info/alternates`
/// names, followed transitively. Each store is walked for links and special files and each
/// entry must stay inside `common_dir`. A store already seen is skipped, so a cycle or a store shared by
/// many paths is checked once (issue #359). Charged to `clock`.
fn object_stores_within(common_dir: &Path, clock: Clock) -> Result<(), String> {
    let first = common_dir.join("objects");
    let mut seen: HashSet<PathBuf> = HashSet::from([first.clone()]);
    let mut pending = vec![first];
    while let Some(store) = pending.pop() {
        if clock.left().is_zero() {
            return Err(UNCHECKED.to_string());
        }
        walk_without_links(&store, clock, true)?;
        let Some(text) =
            read_control_file(&store.join("info").join("alternates"), clock, ALTERNATES)?
        else {
            continue;
        };
        queue_alternates(&store, common_dir, &text, clock, (&mut seen, &mut pending))?;
    }
    Ok(())
}

/// Everything under the git directory, the common directory, `refs`, `info`, `logs` and
/// every object store that is not a plain file or directory: no link, FIFO, socket or device
/// where git may open a file (issue #461). Types only, nothing is opened.
fn walk_repository(git_dir: &Path, common_dir: &Path, clock: Clock) -> Result<(), String> {
    walk_without_links(git_dir, clock, false)?;
    walk_without_links(common_dir, clock, false)?;
    for tree in ["refs", "info", "logs"] {
        walk_without_links(&common_dir.join(tree), clock, true)?;
        if git_dir != common_dir {
            walk_without_links(&git_dir.join(tree), clock, true)?;
        }
    }
    object_stores_within(common_dir, clock)
}

/// Queues the stores listed in the `alternates` file `text` of `store` that are not seen yet
/// (so a crowd of stores listing each other holds each once). Lines are read as git reads
/// them: split at `\n`, `#` lines and empty ones skipped, nothing trimmed. A quoted or
/// whitespace-padded line, which git reads differently from a plain path, and any store that
/// is missing or outside `common_dir` are refused, never skipped. The clock is checked per
/// line (issue #461).
fn queue_alternates(
    store: &Path,
    common_dir: &Path,
    text: &str,
    clock: Clock,
    (seen, pending): (&mut HashSet<PathBuf>, &mut Vec<PathBuf>),
) -> Result<(), String> {
    for line in text.split('\n') {
        if clock.left().is_zero() {
            return Err(UNCHECKED.to_string());
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let padded = |c: char| c.is_whitespace();
        if line.starts_with('"') || line.starts_with(padded) || line.ends_with(padded) {
            return Err(ALTERNATES.to_string());
        }
        let entry = store
            .join(line)
            .canonicalize()
            .map_err(|_| ALTERNATES.to_string())?;
        if !entry.starts_with(common_dir) {
            return Err(ALTERNATES.to_string());
        }
        if seen.insert(entry.clone()) {
            pending.push(entry);
        }
    }
    Ok(())
}

/// Refuses a repository git could trip over before any git process starts (issues #458,
/// #461): a FIFO, device, socket or link where git opens a file while it locates the
/// repository (`HEAD`, `config`, `commondir`, a gitfile, the alternates chain) or later
/// (`index`, `packed-refs`, `refs`, packs ...; see [`walk_repository`]), and a layout git
/// would give up on and then read the workspace root as a bare repository (no `.git`, no
/// `HEAD`, no `objects/` or `refs/`), which is refused as [`OUTSIDE`] too. The gitfile and
/// alternates lines are parsed as git parses them. Newer git (2.56 on CI) opens
/// `objects/info/alternates` at start-up; that was inferred from CI, not from git's source.
/// This is a best-effort fast path: a layout it cannot foresee still ends at git's 15 s time
/// limit, and the full verification after git runs still decides. Charged to `clock`.
fn refuse_special_start_up_files(root: &Path, clock: Clock) -> Result<(), String> {
    let outside = || OUTSIDE.to_string();
    let dot_git = root.join(".git");
    let git_dir = match fs::symlink_metadata(&dot_git) {
        Ok(meta) if meta.is_file() => {
            let text = read_control_file(&dot_git, clock, &special(".git"))?.unwrap_or_default();
            // As git: only trailing CR and LF are dropped, then the exact prefix.
            let target = text.trim_end_matches(['\r', '\n']).strip_prefix("gitdir: ");
            root.join(target.ok_or_else(outside)?)
        }
        Ok(_) => dot_git,
        Err(_) => return Err(outside()),
    };
    let git_dir = git_dir.canonicalize().map_err(|_| outside())?;
    let mut common_dir = git_dir.clone();
    for name in ["HEAD", "config", "commondir"] {
        ordinary_or_missing(&git_dir.join(name), clock, name)?;
    }
    if fs::symlink_metadata(git_dir.join("HEAD")).is_err() {
        return Err(outside());
    }
    if let Some(text) = read_control_file(&git_dir.join("commondir"), clock, &special("commondir"))?
    {
        common_dir = git_dir
            .join(text.trim_end_matches(['\r', '\n']))
            .canonicalize()
            .map_err(|_| outside())?;
    }
    if common_dir != git_dir {
        ordinary_or_missing(&common_dir.join("config"), clock, "config")?;
    }
    if !(common_dir.join("objects").is_dir() && common_dir.join("refs").is_dir()) {
        return Err(outside());
    }
    // git falls back to the root as a bare repository when it rejects this one; its HEAD
    // may be a FIFO.
    // A directory of that name (a project's `config/`) cannot block git; only a FIFO,
    // socket, device or link can.
    for name in ["HEAD", "commondir", "config"] {
        if fs::symlink_metadata(root.join(name)).is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        ordinary_or_missing(&root.join(name), clock, name)?;
    }
    // Walk only a directory that is the workspace's own or a worktree entry; the full
    // verification after git decides the rest.
    let own = root.join(".git").canonicalize().ok();
    if own.as_deref() != Some(git_dir.as_path())
        && git_dir.parent() != Some(common_dir.join("worktrees").as_path())
    {
        return Err(outside());
    }
    walk_repository(&git_dir, &common_dir, clock)
}

fn special(name: &str) -> String {
    format!("the workspace's repository has a {name} that is not an ordinary file (a FIFO, socket, device or link); git tools refuse")
}

/// `Ok` for an ordinary file or a missing one, the refusal naming `name` for anything else.
/// Looks at the type only: the file is not opened (it may be large, like a config).
fn ordinary_or_missing(path: &Path, clock: Clock, name: &str) -> Result<(), String> {
    if clock.left().is_zero() {
        return Err(UNCHECKED.to_string());
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) | Err(_) => Err(special(name)),
    }
}

/// Most bytes of a control file git reads here (`gitdir`, `info/alternates`).
const CONTROL_FILE_LIMIT: u64 = 64 << 10;

/// The text of `path` when it is an ordinary file, `None` when it does not exist, and the
/// `refusal` for anything else: a FIFO, device, socket, directory or symbolic link. Reading
/// a FIFO blocks until a writer comes, so nothing is read before the file type is known
/// (issue #359, review 3). The file is opened without following links and without blocking,
/// then its type is checked again on the open file, so a swap between the two checks is
/// refused too. Charged to `clock`: a spent budget is a refusal, never a partial read.
fn read_control_file(path: &Path, clock: Clock, refusal: &str) -> Result<Option<String>, String> {
    if clock.left().is_zero() {
        return Err(UNCHECKED.to_string());
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(_) | Err(_) => return Err(refusal.to_string()),
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| refusal.to_string())?;
    if !file.metadata().map_err(|_| refusal.to_string())?.is_file() {
        return Err(refusal.to_string());
    }
    let mut text = String::new();
    file.take(CONTROL_FILE_LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(|_| refusal.to_string())?;
    if text.len() as u64 > CONTROL_FILE_LIMIT {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        return Err(format!("the workspace's repository has a {name} larger than {CONTROL_FILE_LIMIT} bytes; git tools refuse"));
    }
    Ok(Some(text))
}

/// A git-reported directory, canonical, or the refusal when it is not a directory.
fn canonical_dir(text: &str) -> Result<PathBuf, String> {
    Path::new(text)
        .canonicalize()
        .ok()
        .filter(|path| path.is_dir())
        .ok_or_else(|| OUTSIDE.to_string())
}

/// Point `command` (already `-C root`) at the verified git directory and work tree, so git
/// uses the directory that was checked, not whatever `.git` says when it runs (issue #359).
pub(crate) fn pin_git_dir<'a>(
    command: &'a mut Command,
    root: &Path,
    verified: &VerifiedGit,
) -> &'a mut Command {
    command
        .arg("--git-dir")
        .arg(&verified.git_dir)
        .arg("--work-tree")
        .arg(root)
}

/// Longest one git call may run.
pub(crate) const DEADLINE: Duration = Duration::from_secs(15);

/// Run `command`, reading at most `limit` bytes of stdout. `status` is `None` only when
/// the output was cut at `limit` (git is then stopped). Once the output ends, git is waited
/// for, so its exit status is real. An error when `deadline` passes (git is stopped) or
/// when stdout cannot be read.
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
    let started = Instant::now();
    let stderr = child.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            // Lossy: a non-UTF-8 warning must still be seen by the listing check.
            let mut bytes = Vec::new();
            let _ = stderr.take(4096).read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    });
    let Some(stdout) = child.stdout.take() else {
        stop(&mut child);
        return Err("cannot read git output".into());
    };
    let reader = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut out = Vec::new();
        stdout.take(limit as u64).read_to_end(&mut out)?;
        Ok(out)
    });
    // Reading ends at the end of the output, or once `limit` bytes are in. A command that
    // holds its output open past the deadline, or whose round is cancelled or spent
    // (issue #257), is stopped here and is an error, never a partial result.
    while !reader.is_finished() {
        if let Some(note) = round_stop_note() {
            stop(&mut child);
            return Err(note);
        }
        if started.elapsed() > deadline {
            stop(&mut child);
            return Err(deadline_error(deadline));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let out = match reader.join() {
        Ok(Ok(out)) => out,
        _ => {
            stop(&mut child);
            return Err("cannot read git output".into());
        }
    };
    let status = if out.len() >= limit {
        stop(&mut child);
        None
    } else {
        loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(error) => {
                    stop(&mut child);
                    return Err(error.to_string());
                }
            }
            if let Some(note) = round_stop_note() {
                stop(&mut child);
                return Err(note);
            }
            if started.elapsed() > deadline {
                stop(&mut child);
                return Err(deadline_error(deadline));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    // stderr is only for messages: wait for it within the deadline, never past it.
    let mut error = String::new();
    if let Some(thread) = stderr {
        while !thread.is_finished() && started.elapsed() <= deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if thread.is_finished() {
            error = thread.join().unwrap_or_default();
        }
    }
    Ok((status, out, error))
}

/// The error text when the round this call runs in was cancelled or is spent (issue #257).
fn round_stop_note() -> Option<String> {
    super::budget::current()
        .and_then(|budget| budget.stop_note())
        .map(|note| note.trim_start_matches("error: ").to_string())
}

fn deadline_error(deadline: Duration) -> String {
    format!(
        "git took longer than {} s and was stopped",
        deadline.as_secs()
    )
}

/// Kills `child` and reaps it.
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The owner's global git config for a write: its identity and `safe.directory`, which the
/// read tier keeps off. Applied after [`hardened`].
pub(crate) fn owner_config(command: &mut Command) -> &mut Command {
    command.env_remove("GIT_CONFIG_GLOBAL");
    for name in ["HOME", "XDG_CONFIG_HOME"] {
        match std::env::var_os(name) {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    command
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
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_PAGER", "cat")
        .arg("--no-optional-locks")
        .arg("--no-pager")
        .args(crate::worker::HARDENED_GIT_ARGS)
        .args(crate::worker::LOCAL_ONLY_GIT_ARGS)
        .args(["-c", "diff.external=", "-c", "core.pager=cat"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write, path::PathBuf};

    /// A scratch repository (#308 unit tests); `config` is appended to its `.git/config`.
    fn scratch_repo(name: &str, config: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("alfredo-git-read-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        assert!(Command::new("/usr/bin/git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success());
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(root.join(".git/config"))
            .unwrap();
        file.write_all(config.as_bytes()).unwrap();
        root
    }

    /// Issue #312: a copy search past git's rename limit warns on stderr and exits 0. The
    /// complete-listing helper refuses that output, and the same search with `-l0` passes.
    #[test]
    fn a_copy_search_warning_refuses_the_listing_and_l0_avoids_it() {
        let root = scratch_repo(
            "copy-warning",
            "[user]\n\tname = Owner\n\temail = owner@example.com\n",
        );
        let git_ok = |args: &[&str]| {
            assert!(Command::new("/usr/bin/git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .unwrap()
                .success())
        };
        fs::write(root.join("a.txt"), "alpha\n").unwrap();
        git_ok(&["add", "a.txt"]);
        git_ok(&["commit", "-q", "-m", "a"]);
        for name in ["b.txt", "c.txt", "d.txt"] {
            fs::write(root.join(name), format!("{name}\n")).unwrap();
        }
        git_ok(&["add", "b.txt", "c.txt", "d.txt"]);
        let search = |limit: &str| {
            let mut args = vec!["diff", "--cached"];
            if !limit.is_empty() {
                args.push(limit);
            }
            args.extend([
                "--find-renames",
                "--find-copies",
                "--find-copies-harder",
                "--name-status",
                "-z",
                "--",
            ]);
            git_complete_listing(&root, &args, 1 << 20)
        };
        let warned = search("-l1").unwrap_err();
        assert!(
            warned.contains("cut its copy or rename search short"),
            "{warned}"
        );
        assert!(search("-l0").is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_global_excludes_file_hides_more_and_a_big_listing_is_refused() {
        let root = scratch_repo("global-excludes", "");
        fs::write(root.join("a.txt"), "a").unwrap();
        fs::write(root.join("b.tmp"), "b").unwrap();
        let excludes = root
            .join("..")
            .join(format!("excludes-{}", std::process::id()));
        fs::write(&excludes, "*.tmp\n").unwrap();
        let plain = list_unignored(&root, "", None, 4096).unwrap();
        assert_eq!(plain, ["a.txt", "b.tmp"]);
        let hidden = list_unignored(&root, "", Some(&excludes), 4096).unwrap();
        assert_eq!(hidden, ["a.txt"]);
        let error = list_unignored(&root, "", None, 4).unwrap_err();
        assert!(error.contains("larger than 4 bytes"), "{error}");
        let _ = fs::remove_file(&excludes);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_nested_git_call_draws_on_the_running_calls_budget() {
        call_scope(Duration::from_millis(300), || {
            std::thread::sleep(Duration::from_millis(200));
            // A git call inside the call sees what is left of the call's own budget.
            assert!(call_clock().left() < Duration::from_millis(150));
            call_scope(Duration::from_secs(60), || {
                assert!(call_clock().left() < Duration::from_millis(150));
            });
        });
        // Outside any call a lone git call gets a fresh, full budget.
        assert!(call_clock().left() > Duration::from_secs(14));
    }

    #[test]
    fn a_cut_listing_is_an_error_not_a_prefix() {
        let root = scratch_repo("cut-listing", "[filter \"f\"]\n\tclean = cat\n");
        let error = git_limited(&root, &["config", "--list"], 64).unwrap_err();
        assert!(error.contains("larger than 64 bytes"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cut_filter_listing_refuses_the_gate() {
        let root = scratch_repo("cut-gate", "[filter \"f\"]\n\tclean = cat\n");
        let error = refuse_filters_within(&root, 8, Clock::start(DEADLINE)).unwrap_err();
        assert!(error.contains("too large"), "{error}");
        let error = refuse_filters(&root).unwrap_err();
        assert!(error.contains("filter drivers"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_listing_that_never_finishes_refuses_the_gate() {
        // An include that is a FIFO with no writer: git config blocks reading it.
        let root = scratch_repo("late-listing", "");
        let fifo = root.join("pipe");
        assert!(Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let include = format!("[include]\n\tpath = {}\n", fifo.display());
        fs::OpenOptions::new()
            .append(true)
            .open(root.join(".git/config"))
            .unwrap()
            .write_all(include.as_bytes())
            .unwrap();
        let started = Instant::now();
        let error = refuse_filters_within(
            &root,
            FILTER_KEYS_LIMIT,
            Clock::start(Duration::from_millis(300)),
        )
        .unwrap_err();
        assert!(error.contains("stopped"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = fs::remove_dir_all(&root);
    }

    /// Issue #458: the start-up check refuses a FIFO where git opens a file, without running
    /// git (newer git opens `objects/info/alternates` while it locates the repository), and
    /// leaves an ordinary repository and a large config alone.
    #[test]
    fn a_start_up_fifo_is_refused_without_running_git() {
        let mkfifo = |path: &Path| {
            let _ = fs::remove_file(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            assert!(Command::new("mkfifo").arg(path).status().unwrap().success());
        };
        let root = scratch_repo("start-up", "");
        let clock = || Clock::start(Duration::from_secs(5));
        let big = "# padding\n".repeat(20_000);
        fs::OpenOptions::new()
            .append(true)
            .open(root.join(".git/config"))
            .unwrap()
            .write_all(big.as_bytes())
            .unwrap();
        refuse_special_start_up_files(&root, clock()).unwrap();
        for name in ["HEAD", "config", "commondir", "objects/info/alternates"] {
            let path = root.join(".git").join(name);
            let kept = fs::read(&path).ok();
            mkfifo(&path);
            let started = Instant::now();
            let error = refuse_special_start_up_files(&root, clock()).unwrap_err();
            assert!(
                error.contains(name.rsplit('/').next().unwrap()),
                "{name}: {error}"
            );
            assert!(started.elapsed() < Duration::from_secs(2), "{name}");
            fs::remove_file(&path).unwrap();
            if let Some(kept) = kept {
                fs::write(&path, kept).unwrap();
            }
        }
        // A store reached through alternates has its own alternates file git opens.
        let second = root.join(".git/second/objects");
        mkfifo(&second.join("info/alternates"));
        fs::create_dir_all(root.join(".git/objects/info")).unwrap();
        fs::write(
            root.join(".git/objects/info/alternates"),
            format!("{}\n", second.display()),
        )
        .unwrap();
        let error = refuse_special_start_up_files(&root, clock()).unwrap_err();
        assert!(error.contains("alternates"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }

    /// Issue #461: alternates lines are read as git reads them; a quoted or padded line, a
    /// missing store and one outside the repository are refused, a spent clock is checked
    /// per line, and a store already seen is queued once.
    #[test]
    fn alternates_lines_are_queued_as_git_reads_them() {
        let common = scratch_repo("alternates-461", "").join(".git");
        let common = common.canonicalize().unwrap();
        let store = common.join("objects");
        let other = common.join("other");
        fs::create_dir_all(&other).unwrap();
        let clock = || Clock::start(Duration::from_secs(5));
        let queue = |text: &str, clock: Clock| {
            let mut seen = HashSet::new();
            let mut pending = Vec::new();
            let result = queue_alternates(&store, &common, text, clock, (&mut seen, &mut pending));
            (result, pending)
        };
        let ok = format!("# note\n\n{0}\n{0}\n../other\n", other.display());
        let (result, pending) = queue(&ok, clock());
        result.unwrap();
        assert_eq!(pending, vec![other.clone()]);
        for bad in [
            format!("\"{}\"", other.display()),
            format!(" {}", other.display()),
            format!("{} ", other.display()),
            format!("{}\r", other.display()),
            "../missing".to_string(),
            "/".to_string(),
        ] {
            let (result, pending) = queue(&bad, clock());
            assert_eq!(result.unwrap_err(), ALTERNATES, "{bad:?}");
            assert!(pending.is_empty(), "{bad:?}");
        }
        // Every line is charged to the clock, also a comment or an empty one.
        for line in ["#", "", "../other"] {
            let spent = Clock::start(Duration::ZERO);
            assert_eq!(queue(line, spent).0.unwrap_err(), UNCHECKED, "{line:?}");
        }
        let _ = fs::remove_dir_all(common.parent().unwrap());
    }

    /// Issue #461 (review): `commondir` is read as git reads it, with only a trailing CR/LF
    /// dropped, so a padded value does not resolve.
    #[test]
    fn a_padded_commondir_is_not_trimmed() {
        let root = scratch_repo("commondir-461", "");
        let clock = || Clock::start(Duration::from_secs(5));
        fs::write(root.join(".git/commondir"), ".\r\n").unwrap();
        refuse_special_start_up_files(&root, clock()).unwrap();
        fs::write(root.join(".git/commondir"), ". \n").unwrap();
        assert_eq!(
            refuse_special_start_up_files(&root, clock()).unwrap_err(),
            OUTSIDE
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// Issue #449: a directory that vanishes or cannot be read mid-walk (a background
    /// `git gc` pruning loose objects) refuses with its own reason, not the time limit's;
    /// a spent budget still refuses with the time limit.
    #[test]
    fn an_unreadable_directory_refuses_without_blaming_the_time_limit() {
        let root = std::env::temp_dir().join(format!("alfredo-walk-449-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let file = root.join("not-a-dir");
        fs::write(&file, "x").unwrap();
        let error = walk_without_links(&file, Clock::start(DEADLINE), true).unwrap_err();
        assert_eq!(error, UNREADABLE);
        let error = walk_without_links(&root, Clock::start(Duration::ZERO), true).unwrap_err();
        assert_eq!(error, UNCHECKED);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_command_that_closes_its_output_is_waited_for() {
        // stdout closes at once; git would exit with status 3 after the sleep.
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec >&-; sleep 0.3; exit 3"]);
        let (status, out, _) = run_bounded(command, 4096, DEADLINE).unwrap();
        assert_eq!(status.and_then(|status| status.code()), Some(3));
        assert!(out.is_empty());
    }

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
    fn a_cancelled_or_spent_round_stops_a_running_git_call_at_once() {
        use crate::chat_tools::budget::{within, Budget, Cancel};
        let run = |budget: Budget| {
            let mut command = Command::new("/bin/sleep");
            command.arg("5");
            let started = Instant::now();
            let error = within(&budget, || run_bounded(command, 1024, DEADLINE)).unwrap_err();
            assert!(started.elapsed() < Duration::from_secs(1));
            error
        };
        let cancel = Cancel::default();
        cancel.set();
        assert_eq!(
            run(Budget::new(Duration::from_secs(30), cancel)),
            "cancelled"
        );
        let spent = run(Budget::new(Duration::ZERO, Cancel::default()));
        assert!(spent.contains("round time limit"), "{spent}");
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

    /// Issue #467: a partial clone whose missing blob makes git lazy-fetch through
    /// `remote.origin.uploadpack`, a program the repository chose. The program appends `RAN`
    /// to the returned marker when it runs, then fails. Returns (workspace, marker).
    fn partial_clone_fixture(name: &str) -> (PathBuf, PathBuf) {
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("alfredo-partial-{pid}-{name}"));
        let marker = std::env::temp_dir().join(format!("alfredo-partial-{pid}-{name}.ran"));
        let script = std::env::temp_dir().join(format!("alfredo-partial-{pid}-{name}.sh"));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&marker);
        fs::create_dir_all(&root).unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("/usr/bin/git")
                .arg("-C")
                .arg(&root)
                .args(["-c", "user.name=T", "-c", "user.email=t@e"])
                .args(args)
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q"]);
        fs::write(root.join("a.txt"), "one\n").unwrap();
        git(&["add", "a.txt"]);
        git(&["commit", "-qm", "i"]);
        let blob = Command::new("/usr/bin/git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "HEAD:a.txt"])
            .output()
            .unwrap()
            .stdout;
        let blob = String::from_utf8(blob).unwrap().trim().to_string();
        fs::remove_file(root.join(".git/objects").join(&blob[..2]).join(&blob[2..])).unwrap();
        fs::write(root.join("a.txt"), "two\n").unwrap();
        fs::write(
            &script,
            format!("#!/bin/sh\necho RAN >> '{}'\nexit 1\n", marker.display()),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let url = root.display().to_string();
        let uploadpack = script.display().to_string();
        git(&["config", "core.repositoryformatversion", "1"]);
        git(&["config", "extensions.partialclone", "origin"]);
        git(&["config", "remote.origin.url", &url]);
        git(&["config", "remote.origin.promisor", "true"]);
        git(&["config", "remote.origin.uploadpack", &uploadpack]);
        (root, marker)
    }

    /// The chat git_diff tool, on a partial clone with a missing blob (#467), never runs
    /// the repository's uploadpack program.
    #[test]
    fn git_diff_never_runs_a_partial_clone_uploadpack_program() {
        let (root, marker) = partial_clone_fixture("diff-tool");
        let _ = git_diff(&root, &json!({}));
        assert!(
            !marker.exists(),
            "git_diff ran the repository's uploadpack program"
        );
    }

    /// The hardened git environment alone (no config gate) never lazy-fetches through the
    /// partial clone's uploadpack program (#467).
    #[test]
    fn hardened_git_diff_never_runs_a_partial_clone_uploadpack_program() {
        let (root, marker) = partial_clone_fixture("diff-env");
        let clock = Clock::start(DEADLINE);
        let command =
            git_command_within(&root, &["diff", "--stat", "--patch", "--", "."], clock).unwrap();
        let _ = run_bounded(command, 1 << 20, DEADLINE);
        assert!(
            !marker.exists(),
            "hardened git ran the repository's uploadpack program"
        );
    }

    /// The read-tier config gate refuses a partial clone and names the offending key (#467).
    #[test]
    fn refuse_filters_refuses_and_names_partial_clone_and_transport_keys() {
        let (root, marker) = partial_clone_fixture("refuse-names");
        let error = refuse_filters(&root).unwrap_err();
        assert!(error.contains("remote.origin.uploadpack"), "{error}");
        assert!(!marker.exists());
    }

    /// Review round 2 (#467): a repository's `protocol.<name>.allow` overrides the
    /// command-line `protocol.allow=never`, so the config gate refuses it by name. The
    /// unrelated `protocol.version` stays allowed.
    #[test]
    fn refuse_filters_refuses_and_names_protocol_allow_overrides() {
        let root = scratch_repo("protocol-allow", "[protocol \"file\"]\n\tallow = always\n");
        let error = refuse_filters(&root).unwrap_err();
        assert!(error.contains("protocol.file.allow"), "{error}");
        let version = scratch_repo("protocol-version", "[protocol]\n\tversion = 2\n");
        assert!(refuse_filters(&version).is_ok());
    }
}
