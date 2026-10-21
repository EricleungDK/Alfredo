//! Git write tools (issue #237): git_commit, git_switch and git_merge (Tier::Mutating)
//! and git_push (Tier::Push) on the owner's real repository, on host. Every call pauses
//! for owner Approval; [`preview`] builds the card and refuses invalid calls first, and
//! [`fingerprint`] (HEAD, branch, refs, index and working-tree status) must be unchanged
//! when the card is approved.
//!
//! Hardening as git_read: model arguments cannot become options (refs starting with `-`
//! are refused, `--end-of-options` and `--` separate them, paths are `:(literal)`),
//! repositories with filter drivers or includes are refused, hooks and fsmonitor are off
//! (`HARDENED_GIT_ARGS`, as worker host git), each call has a deadline. Local writes see
//! the owner's global config (identity) but no other environment; push also keeps the
//! environment credential helpers and the SSH agent need, with prompts off.
//! A merge conflict is never resolved: the conflicted files are listed and the merge is
//! aborted, so the repository is left as it was.
use super::{
    approval::CardKind,
    edit::Change,
    git_read::{self, ref_arg},
    text_arg, Tier, Tool, MAX_OUTPUT,
};
use serde_json::{json, Value};
use std::{path::Path, process::Command, time::Duration};

/// Files git_commit takes by name.
const MAX_FILES: usize = 100;
/// Longest commit message accepted, in bytes.
const MAX_MESSAGE: usize = 4096;
/// Commits listed on a merge or push card.
const MAX_LISTED: usize = 20;
/// Longest a local write (commit, switch, merge) may run.
const LOCAL_DEADLINE: Duration = Duration::from_secs(120);
/// Largest git output read in full (fingerprints, card listings); more is refused.
const FULL: usize = 64 << 20;
/// Lines of a file listing shown on a card.
const MAX_CARD_LINES: usize = 40;
/// Longest a push may run (network, credential helpers).
const PUSH_DEADLINE: Duration = Duration::from_secs(120);
/// Environment a push run must not inherit: it would redirect git elsewhere.
const REDIRECTING_ENV: [&str; 12] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_EXEC_PATH",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_CEILING_DIRECTORIES",
];

pub const GIT_COMMIT: Tool = Tool {
    name: "git_commit",
    tier: Tier::Mutating,
    description: "Commit on the current branch with a message. Without files: commits what is already staged. With files: stages and commits exactly those files (new files too), leaving other changes uncommitted. Needs the owner's approval; the owner sees the files and message. Hooks do not run.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "message": {"type": "string", "description": "Commit message"},
                "files": {"type": "array", "items": {"type": "string"}, "description": "Files to commit, relative to the repository root (default: the staged changes)"}
            },
            "required": ["message"]
        })
    },
    run: commit,
};

pub const GIT_SWITCH: Tool = Tool {
    name: "git_switch",
    tier: Tier::Mutating,
    description: "Switch the working tree to another local branch, or create=true to create a new branch at the current commit and switch to it. Uncommitted changes are carried over; git refuses if they would be overwritten. Needs the owner's approval.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "branch": {"type": "string", "description": "Local branch name, e.g. main or feature/x"},
                "create": {"type": "boolean", "description": "Create the branch first (default false)"}
            },
            "required": ["branch"]
        })
    },
    run: switch,
};

pub const GIT_MERGE: Tool = Tool {
    name: "git_merge",
    tier: Tier::Mutating,
    description: "Merge a branch (e.g. an alfredo/go-<id> branch made by /go) into the current branch. target, if given, must be the current branch (git_switch to it first). Needs the owner's approval; the owner sees the commits and files. On conflict the merge is aborted and the conflicted files are listed; nothing changes.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "source": {"type": "string", "description": "Branch or ref to merge, e.g. alfredo/go-12"},
                "target": {"type": "string", "description": "Branch merged into; must be the current branch (default: current)"}
            },
            "required": ["source"]
        })
    },
    run: merge,
};

pub const GIT_PUSH: Tool = Tool {
    name: "git_push",
    tier: Tier::Push,
    description: "Push a local branch to the same-named branch on a configured remote (no force). Always needs the owner's approval, every time; the owner sees the commits to publish. Rejected (non-fast-forward) pushes are reported, not forced.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "remote": {"type": "string", "description": "Configured remote name (default origin)"},
                "branch": {"type": "string", "description": "Local branch to push (default: current)"}
            }
        })
    },
    run: push,
};

const NAMES: [&str; 4] = ["git_commit", "git_switch", "git_merge", "git_push"];

/// Repository state an approved call must still see: HEAD, current branch, local and
/// remote-tracking refs, index and working-tree status. `None` for other tools.
pub(crate) fn fingerprint(
    root: &Path,
    name: &str,
    _arguments: &Value,
) -> Option<Result<String, String>> {
    if !NAMES.contains(&name) {
        return None;
    }
    Some(state(root))
}

fn state(root: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    git_read::refuse_filters(root)?;
    let head =
        git_read::git(root, &["rev-parse", "--verify", "--quiet", "HEAD"]).unwrap_or_default();
    let branch = git_read::git(root, &["symbolic-ref", "--quiet", "HEAD"]).unwrap_or_default();
    let refs = full(
        root,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    let status = full(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut hash = Sha256::new();
    for part in [&head, &branch, &refs, &status] {
        hash.update(part.as_bytes());
        hash.update([0xff]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Card content for a call of this family, or why it is refused before asking.
/// `None` when `name` is not one of these tools.
pub(crate) fn preview(
    root: &Path,
    name: &str,
    arguments: &Value,
) -> Option<Result<(Change, CardKind), String>> {
    let fingerprint = match fingerprint(root, name, arguments)? {
        Ok(fingerprint) => fingerprint,
        Err(error) => return Some(Err(error)),
    };
    let planned = match name {
        "git_commit" => plan_commit(root, arguments).map(|plan| plan.card()),
        "git_switch" => plan_switch(root, arguments).map(|plan| plan.card()),
        "git_merge" => plan_merge(root, arguments).map(|plan| plan.card()),
        "git_push" => plan_push(root, arguments).map(|plan| plan.card()),
        _ => return None,
    };
    Some(planned.map(|(summary, diff, kind)| {
        (
            Change {
                summary,
                diff,
                fingerprint,
            },
            kind,
        )
    }))
}

type CardText = (String, String, CardKind);

/// Complete stdout of a read-only git call; output past [`FULL`] is refused, never cut.
fn full(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_read::git_limited(root, args, FULL)?;
    if output.len() >= FULL {
        return Err(format!(
            "git {} output is too large to check safely; refused",
            args[0]
        ));
    }
    Ok(output)
}

/// Refuse when repository-local config (not the owner's global or system config) sets
/// keys that make git run a command: merge drivers and signing programs for every write,
/// and for push also SSH, askpass, credential helpers, pack programs, proxies, URL
/// rewrites and protocol allow overrides.
fn refuse_repository_config(root: &Path, push: bool) -> Result<(), String> {
    let listing = full(root, &["config", "--list", "--show-scope"])?;
    for line in listing.lines() {
        let Some((scope, entry)) = line.split_once('\t') else {
            continue;
        };
        if !matches!(scope, "local" | "worktree") {
            continue;
        }
        let key = entry.split('=').next().unwrap_or_default().to_lowercase();
        let ends = |prefix: &str, suffix: &str| key.starts_with(prefix) && key.ends_with(suffix);
        let local = ends("merge.", ".driver") || ends("gpg.", ".program");
        let remote = push
            && (matches!(
                key.as_str(),
                "core.sshcommand"
                    | "core.askpass"
                    | "http.proxy"
                    | "https.proxy"
                    | "protocol.allow"
            ) || ends("credential.", ".helper")
                || ends("remote.", ".receivepack")
                || ends("remote.", ".uploadpack")
                || ends("http.", ".proxy")
                || ends("url.", ".insteadof")
                || ends("url.", ".pushinsteadof")
                || ends("protocol.", ".allow"));
        if local || remote {
            return Err(format!(
                "repository config sets {key}, which can make git run a command; refused (the owner can run this git command by hand)"
            ));
        }
    }
    Ok(())
}

/// What a stopped or failed write may have left behind, for an honest report.
fn leftovers(root: &Path) -> String {
    let lock = git_read::git(root, &["rev-parse", "--git-path", "index.lock"])
        .ok()
        .map(|path| root.join(path.trim()));
    if lock.is_some_and(|path| path.exists()) {
        "; index.lock remains (a stopped or concurrent git write), ask the owner to check the repository".into()
    } else {
        String::new()
    }
}

fn required<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    text_arg(arguments, name)?.ok_or_else(|| format!("argument {name} is required"))
}

fn flag(arguments: &Value, name: &str) -> Result<bool, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("argument {name} must be true or false")),
    }
}

/// The current branch name; a detached HEAD is refused.
fn current_branch(root: &Path) -> Result<String, String> {
    git_read::git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|name| name.trim().to_string())
        .ok()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "HEAD is detached; git_switch to a branch first".to_string())
}

fn local_branch_exists(root: &Path, branch: &str) -> bool {
    let full = format!("refs/heads/{branch}");
    git_read::git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &full,
        ],
    )
    .is_ok_and(|sha| !sha.trim().is_empty())
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

struct CommitPlan {
    branch: String,
    message: String,
    /// `:(literal)` pathspecs; empty commits the staged changes.
    pathspecs: Vec<String>,
    listing: String,
}

impl CommitPlan {
    fn card(self) -> CardText {
        let count = self.listing.lines().count();
        let first = self.message.lines().next().unwrap_or_default();
        (
            format!(
                "commit {count} file{} on {}: {first}",
                if count == 1 { "" } else { "s" },
                self.branch
            ),
            format!(
                "branch: {}\nmessage:\n{}\nfiles{}:\n{}",
                self.branch,
                indent(&self.message),
                if self.pathspecs.is_empty() {
                    " (staged)"
                } else {
                    ""
                },
                indent(&cut(&self.listing, MAX_CARD_LINES))
            ),
            CardKind::Git,
        )
    }
}

fn plan_commit(root: &Path, arguments: &Value) -> Result<CommitPlan, String> {
    let message = required(arguments, "message")?.trim().to_string();
    if message.is_empty() {
        return Err("argument message must not be empty".into());
    }
    if message.len() > MAX_MESSAGE || message.contains('\0') {
        return Err(format!("message must be under {MAX_MESSAGE} bytes of text"));
    }
    let files = match arguments.get("files") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(files)) if files.len() <= MAX_FILES => files.clone(),
        Some(Value::Array(_)) => return Err(format!("at most {MAX_FILES} files per commit")),
        Some(_) => return Err("argument files must be a list of paths".into()),
    };
    let mut pathspecs = Vec::new();
    for file in &files {
        let Value::String(file) = file else {
            return Err("argument files must be a list of paths".into());
        };
        let path = git_read::path_arg(&json!({ "path": file }))?
            .ok_or("files entries must name files, not the whole repository")?;
        pathspecs.push(format!(":(literal){path}"));
    }
    refuse_repository_config(root, false)?;
    let branch = current_branch(root)?;
    refuse_secrets(root, &pathspecs)?;
    let listing = if pathspecs.is_empty() {
        full(
            root,
            &["diff", "--cached", "--no-ext-diff", "--name-status", "--"],
        )?
    } else {
        let mut args = vec!["status", "--porcelain=v1", "--untracked-files=all", "--"];
        args.extend(pathspecs.iter().map(String::as_str));
        full(root, &args)?
    };
    if listing.trim().is_empty() {
        return Err(if pathspecs.is_empty() {
            "nothing to commit: nothing is staged (pass files to commit them)".into()
        } else {
            "nothing to commit: the named files have no changes".into()
        });
    }
    Ok(CommitPlan {
        branch,
        message,
        pathspecs,
        listing: listing.trim_end().to_string(),
    })
}

/// Refuse when any file the commit would take (the staged names, or git's expansion of
/// the pathspecs) is a secret or `.git` path. Generated trees stay committable. Runs in
/// `plan_commit`, so it applies at preview and again at apply.
fn refuse_secrets(root: &Path, pathspecs: &[String]) -> Result<(), String> {
    let mut names = Vec::new();
    if pathspecs.is_empty() {
        let out = full(
            root,
            &[
                "diff",
                "--cached",
                "--no-ext-diff",
                "--no-renames",
                "--name-only",
                "-z",
                "--",
            ],
        )?;
        names.extend(out.split('\0').filter(|n| !n.is_empty()).map(String::from));
    } else {
        let mut args = vec![
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
        ];
        args.extend(pathspecs.iter().map(String::as_str));
        let out = full(root, &args)?;
        let mut entries = out.split('\0').filter(|e| !e.is_empty());
        while let Some(entry) = entries.next() {
            let Some((code, name)) = entry.split_at_checked(3) else {
                continue;
            };
            names.push(name.to_string());
            if code.contains(['R', 'C']) {
                // A rename or copy is followed by its source name.
                names.extend(entries.next().map(String::from));
            }
        }
    }
    match names
        .iter()
        .find(|name| crate::planning_context::secret(&name.to_lowercase()))
    {
        Some(name) => Err(format!(
            "refused: {name} looks like a secret (.env*, *.pem, *.key, *.p12, *.pfx or .git); commit it by hand if intended"
        )),
        None => Ok(()),
    }
}

fn commit(root: &Path, arguments: &Value) -> Result<String, String> {
    let plan = plan_commit(root, arguments)?;
    let mut args = vec!["commit", "--quiet", "--no-verify", "-m", &plan.message];
    if !plan.pathspecs.is_empty() {
        let mut add = vec!["add", "--"];
        add.extend(plan.pathspecs.iter().map(String::as_str));
        let index = write(root, &["write-tree"], Env::Local)?;
        let index = index.trim();
        write(root, &add, Env::Local).map_err(|error| error + &leftovers(root))?;
        args.push("--");
        args.extend(plan.pathspecs.iter().map(String::as_str));
        if let Err(error) = write(root, &args, Env::Local) {
            let restored = match write(root, &["read-tree", "--end-of-options", index], Env::Local)
            {
                Ok(_) => "; the index was restored".to_string(),
                Err(reset) => format!("; restoring the index failed ({reset})"),
            };
            return Err(error + &restored + &leftovers(root));
        }
    } else {
        write(root, &args, Env::Local).map_err(|error| error + &leftovers(root))?;
    }
    let made = git_read::git(root, &["log", "-1", "--no-color", "--format=%h %s"])?;
    Ok(format!("committed on {}: {}", plan.branch, made.trim()))
}

struct SwitchPlan {
    from: String,
    branch: String,
    create: bool,
}

impl SwitchPlan {
    fn card(self) -> CardText {
        (
            if self.create {
                format!(
                    "create branch {} from {} and switch to it",
                    self.branch, self.from
                )
            } else {
                format!("switch {} → {}", self.from, self.branch)
            },
            format!(
                "from: {}\nto: {}{}\nUncommitted changes are carried over; git refuses if they would be overwritten.",
                self.from,
                self.branch,
                if self.create { " (new, at the current commit)" } else { "" }
            ),
            CardKind::Git,
        )
    }
}

fn plan_switch(root: &Path, arguments: &Value) -> Result<SwitchPlan, String> {
    let branch = ref_arg(arguments, "branch")?
        .ok_or("argument branch is required")?
        .to_string();
    let create = flag(arguments, "create")?;
    refuse_repository_config(root, false)?;
    let exists = local_branch_exists(root, &branch);
    if create {
        if exists {
            return Err(format!(
                "branch {branch} already exists; switch without create"
            ));
        }
        git_read::git(root, &["check-ref-format", "--branch", &branch])
            .map_err(|_| format!("{branch:?} is not a valid branch name"))?;
    } else if !exists {
        return Err(format!(
            "no local branch {branch} (create=true makes it at the current commit)"
        ));
    }
    let from = match current_branch(root) {
        Ok(name) => name,
        Err(_) => format!(
            "detached HEAD {}",
            git_read::git(root, &["rev-parse", "--short", "HEAD"])?.trim()
        ),
    };
    if from == branch {
        return Err(format!("already on {branch}"));
    }
    Ok(SwitchPlan {
        from,
        branch,
        create,
    })
}

fn switch(root: &Path, arguments: &Value) -> Result<String, String> {
    let plan = plan_switch(root, arguments)?;
    if plan.create {
        // Validated above: a plain branch name that cannot start with `-`.
        write(
            root,
            &["switch", "--no-guess", "-c", &plan.branch],
            Env::Local,
        )
        .map_err(|error| error + &leftovers(root))?;
        Ok(format!(
            "created {} from {} and switched to it",
            plan.branch, plan.from
        ))
    } else {
        write(
            root,
            &["switch", "--no-guess", "--end-of-options", &plan.branch],
            Env::Local,
        )
        .map_err(|error| error + &leftovers(root))?;
        Ok(format!("switched {} → {}", plan.from, plan.branch))
    }
}

struct MergePlan {
    source: String,
    target: String,
    count: usize,
    commits: String,
    files: String,
}

impl MergePlan {
    fn card(self) -> CardText {
        (
            format!(
                "merge {} → {} ({} commit{})",
                self.source,
                self.target,
                self.count,
                if self.count == 1 { "" } else { "s" }
            ),
            format!(
                "source: {}\ntarget: {} (current branch)\ncommits:\n{}{}\nfiles:\n{}\nOn conflict the merge is aborted and nothing changes.",
                self.source,
                self.target,
                indent(&self.commits),
                if self.count > MAX_LISTED {
                    format!("\n  [{} more]", self.count - MAX_LISTED)
                } else {
                    String::new()
                },
                indent(&self.files)
            ),
            CardKind::Git,
        )
    }
}

fn plan_merge(root: &Path, arguments: &Value) -> Result<MergePlan, String> {
    let source = ref_arg(arguments, "source")?
        .ok_or("argument source is required")?
        .to_string();
    let target = ref_arg(arguments, "target")?;
    refuse_repository_config(root, false)?;
    let current = current_branch(root)?;
    if let Some(target) = target {
        if target != current {
            return Err(format!(
                "merge target {target} is not the current branch {current}; git_switch to {target} first"
            ));
        }
    }
    let dirty = git_read::git(root, &["status", "--porcelain=v1", "--untracked-files=no"])?;
    if !dirty.trim().is_empty() {
        return Err(
            "tracked files have uncommitted changes; commit or stash them before merging".into(),
        );
    }
    let commit = format!("{source}^{{commit}}");
    git_read::git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &commit,
        ],
    )
    .map_err(|_| format!("no branch or commit named {source}"))?;
    let range = format!("HEAD..{source}");
    let count: usize = git_read::git(
        root,
        &["rev-list", "--count", "--end-of-options", &range, "--"],
    )?
    .trim()
    .parse()
    .unwrap_or(0);
    if count == 0 {
        return Err(format!("{current} is already up to date with {source}"));
    }
    let listed = format!("--max-count={MAX_LISTED}");
    let commits = git_read::git(
        root,
        &[
            "log",
            "--no-color",
            "--no-show-signature",
            "--format=%h %s",
            &listed,
            "--end-of-options",
            &range,
            "--",
        ],
    )?;
    let three = format!("HEAD...{source}");
    let files = git_read::git(
        root,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--stat",
            "--end-of-options",
            &three,
            "--",
        ],
    )?;
    Ok(MergePlan {
        source,
        target: current,
        count,
        commits: commits.trim_end().to_string(),
        files: cut(files.trim_end(), 40),
    })
}

/// At most `lines` lines of `text`, with a marker.
fn cut(text: &str, lines: usize) -> String {
    let total = text.lines().count();
    let mut kept: Vec<&str> = text.lines().take(lines).collect();
    let more = format!("[{} more lines]", total.saturating_sub(lines));
    if total > lines {
        kept.push(&more);
    }
    kept.join("\n")
}

fn merge(root: &Path, arguments: &Value) -> Result<String, String> {
    let plan = plan_merge(root, arguments)?;
    let before = git_read::git(root, &["rev-parse", "HEAD"])?;
    if let Err(error) = write(
        root,
        &["merge", "--no-edit", "--end-of-options", &plan.source],
        Env::Local,
    ) {
        let merging = git_read::git(root, &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"])
            .is_ok_and(|sha| !sha.trim().is_empty());
        if !merging {
            return Err(error + &leftovers(root));
        }
        let conflicted = git_read::git(
            root,
            &["diff", "--name-only", "--diff-filter=U", "--no-ext-diff"],
        )
        .unwrap_or_default();
        let aborted = write(root, &["merge", "--abort"], Env::Local);
        let files: Vec<&str> = conflicted.lines().filter(|l| !l.is_empty()).collect();
        let restored = git_read::git(root, &["rev-parse", "HEAD"]).is_ok_and(|head| head == before)
            && git_read::git(root, &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"]).is_err()
            && leftovers(root).is_empty();
        return Err(match aborted {
            Ok(_) if !restored => format!(
                "merge conflict in: {}; merge aborted, but the repository may not be as it was{}; ask the owner to check",
                files.join(", "),
                leftovers(root)
            ),
            Ok(_) => format!(
                "merge conflict in: {}; merge aborted, the repository is as it was. Ask the owner to resolve it",
                files.join(", ")
            ),
            Err(abort) => format!(
                "merge conflict in: {}; aborting failed ({abort}); the repository is mid-merge, ask the owner",
                files.join(", ")
            ),
        });
    }
    let head = git_read::git(root, &["log", "-1", "--no-color", "--format=%h %s"])?;
    Ok(format!(
        "merged {} into {} ({} commits): {}",
        plan.source,
        plan.target,
        plan.count,
        head.trim()
    ))
}

struct PushPlan {
    remote: String,
    branch: String,
    url: String,
    count: Option<usize>,
    commits: String,
}

impl PushPlan {
    fn card(self) -> CardText {
        (
            format!(
                "push {} → {}/{}{}",
                self.branch,
                self.remote,
                self.branch,
                match self.count {
                    Some(count) => format!(" ({count} commit{})", if count == 1 { "" } else { "s" }),
                    None => " (new remote branch)".into(),
                }
            ),
            format!(
                "remote: {} ({})\nbranch: {} → {}/{}\ncommits to publish{}:\n{}\nPublishes to a remote; this cannot be undone from here.",
                self.remote,
                self.url,
                self.branch,
                self.remote,
                self.branch,
                if self.count.is_none() { " (latest; the branch is new there)" } else { "" },
                indent(&self.commits)
            ),
            CardKind::Push,
        )
    }
}

/// A configured remote name: plain characters only (never a URL or path).
fn remote_arg(root: &Path, arguments: &Value) -> Result<String, String> {
    let remote = text_arg(arguments, "remote")?
        .map(str::trim)
        .unwrap_or("origin");
    let plain = !remote.is_empty()
        && remote.len() <= 64
        && !remote.starts_with(['-', '.'])
        && remote
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    let configured = plain
        && git_read::git(root, &["remote"])?
            .lines()
            .any(|line| line.trim() == remote);
    if !configured {
        return Err(format!(
            "remote {:?} is not a configured remote name (see git remote)",
            remote.chars().take(64).collect::<String>()
        ));
    }
    Ok(remote.to_string())
}

/// `url` without user info (tokens) in it.
fn without_credentials(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let host_end = rest.find('/').unwrap_or(rest.len());
            match rest[..host_end].rfind('@') {
                Some(at) => format!("{scheme}://{}", &rest[at + 1..]),
                None => url.to_string(),
            }
        }
        None => url.to_string(),
    }
}

fn plan_push(root: &Path, arguments: &Value) -> Result<PushPlan, String> {
    let remote = remote_arg(root, arguments)?;
    refuse_repository_config(root, true)?;
    let branch = match ref_arg(arguments, "branch")? {
        Some(branch) => branch.to_string(),
        None => current_branch(root)?,
    };
    if !local_branch_exists(root, &branch) {
        return Err(format!("no local branch {branch}"));
    }
    let url = git_read::git(
        root,
        &["remote", "get-url", "--push", "--all", "--", &remote],
    )
    .map(|urls| {
        urls.lines()
            .map(|url| without_credentials(url.trim()))
            .collect::<Vec<_>>()
            .join(", ")
    })
    .unwrap_or_default();
    let tracking = format!("refs/remotes/{remote}/{branch}");
    let local = format!("refs/heads/{branch}");
    let tracked = git_read::git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &tracking,
        ],
    )
    .is_ok_and(|sha| !sha.trim().is_empty());
    let listed = format!("--max-count={MAX_LISTED}");
    let (count, range) = if tracked {
        let range = format!("{tracking}..{local}");
        let count: usize = git_read::git(
            root,
            &["rev-list", "--count", "--end-of-options", &range, "--"],
        )?
        .trim()
        .parse()
        .unwrap_or(0);
        if count == 0 {
            return Err(format!(
                "{remote}/{branch} is already up to date with {branch}"
            ));
        }
        (Some(count), range)
    } else {
        (None, local.clone())
    };
    let commits = git_read::git(
        root,
        &[
            "log",
            "--no-color",
            "--no-show-signature",
            "--format=%h %s",
            &listed,
            "--end-of-options",
            &range,
            "--",
        ],
    )?;
    Ok(PushPlan {
        remote,
        branch,
        url,
        count,
        commits: commits.trim_end().to_string(),
    })
}

fn push(root: &Path, arguments: &Value) -> Result<String, String> {
    let plan = plan_push(root, arguments)?;
    let refspec = format!("refs/heads/{0}:refs/heads/{0}", plan.branch);
    write(
        root,
        &[
            "push",
            "--no-verify",
            "--end-of-options",
            &plan.remote,
            &refspec,
        ],
        Env::Remote,
    )?;
    Ok(format!(
        "pushed {} to {}/{}",
        plan.branch, plan.remote, plan.branch
    ))
}

#[derive(Clone, Copy)]
enum Env {
    /// Hardened read environment plus the owner's HOME (global config: identity).
    Local,
    /// The owner's environment (credential helpers, SSH agent) minus git redirections;
    /// prompts off.
    Remote,
}

const TERMINAL_HINT: &str = "ssh needed a terminal prompt (host key or passphrase); add the key to ssh-agent or run git push yourself";

/// Start the child in its own session so it has no controlling terminal: ssh and some
/// credential helpers open `/dev/tty` directly, which `GIT_TERMINAL_PROMPT=0` and a null
/// stdin do not stop, and would draw over the TUI and compete for its keystrokes.
fn detach_from_terminal(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and touches no memory.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    let _ = command;
}

/// Whether git's stderr shows ssh wanted a prompt it could not have.
fn needed_terminal(error: &str) -> bool {
    let error = error.to_lowercase();
    ["/dev/tty", "no tty", "host key verification failed"]
        .iter()
        .any(|needle| error.contains(needle))
}

/// Run a writing git command in `root`. `Err` is git's message.
fn write(root: &Path, args: &[&str], env: Env) -> Result<String, String> {
    let ceiling = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    let ceiling = ceiling.parent().unwrap_or(&ceiling).to_path_buf();
    let mut command = Command::new("/usr/bin/git");
    let deadline = match env {
        Env::Local => {
            git_read::hardened(&mut command).env_remove("GIT_CONFIG_GLOBAL");
            for name in ["HOME", "XDG_CONFIG_HOME"] {
                match std::env::var_os(name) {
                    Some(value) => command.env(name, value),
                    None => command.env_remove(name),
                };
            }
            LOCAL_DEADLINE
        }
        Env::Remote => {
            for name in REDIRECTING_ENV {
                command.env_remove(name);
            }
            command
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("LC_ALL", "C")
                .env("GIT_PAGER", "cat")
                .arg("--no-pager")
                .args(crate::worker::HARDENED_GIT_ARGS)
                .args(["-c", "protocol.ext.allow=never"]);
            PUSH_DEADLINE
        }
    };
    command
        .env("GIT_CEILING_DIRECTORIES", ceiling)
        .arg("-C")
        .arg(root)
        .args(args);
    detach_from_terminal(&mut command);
    let (status, out, error) = git_read::run_bounded(command, MAX_OUTPUT * 2, deadline)?;
    match status {
        Some(status) if !status.success() => {
            if matches!(env, Env::Remote) && needed_terminal(&error) {
                return Err(TERMINAL_HINT.to_string());
            }
            let lines: Vec<&str> = error
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with("hint:"))
                .map(|line| {
                    line.trim_start_matches("fatal: ")
                        .trim_start_matches("error: ")
                })
                .take(4)
                .collect();
            Err(if lines.is_empty() {
                format!("git {} failed", args[0])
            } else {
                lines.join("; ")
            })
        }
        _ => Ok(String::from_utf8_lossy(&out).into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_lose_their_credentials() {
        assert_eq!(
            without_credentials("https://user:tok@github.com/o/r.git"),
            "https://github.com/o/r.git"
        );
        assert_eq!(
            without_credentials("git@github.com:o/r.git"),
            "git@github.com:o/r.git"
        );
        assert_eq!(without_credentials("/tmp/r.git"), "/tmp/r.git");
    }

    #[test]
    fn ssh_terminal_failures_are_recognised() {
        assert!(needed_terminal("Host key verification failed.\n"));
        assert!(needed_terminal("could not open /dev/tty: No such device"));
        assert!(!needed_terminal("Permission denied (publickey)."));
    }
}
