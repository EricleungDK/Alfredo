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
//! Repository-local config is an allowlist (#309): a key that can run a program or
//! redirect a remote refuses the call by name. Push takes only https, ssh and scp-style
//! remotes, sends one branch with no tags and no submodules, and the approval fingerprint
//! covers the config and the remote URLs, so a change after the card refuses the apply.
//! A merge conflict is never resolved: the conflicted files are listed and the merge is
//! aborted, so the repository is left as it was.
use super::{
    approval::CardKind,
    edit::Change,
    git_read::{self, ref_arg},
    needed_terminal, text_arg, Tier, Tool, MAX_OUTPUT,
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
/// Most lines of a file listing a card shows; a longer one is refused (issue #269).
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
    arguments: &Value,
) -> Option<Result<String, String>> {
    git_read::call_scope(LOCAL_DEADLINE, || fingerprint_state(root, name, arguments))
}

/// [`fingerprint`] for one call's budget (issue #359).
fn fingerprint_state(
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
    let config = config_listing(root)?;
    let remotes = full(root, &["remote", "--verbose"])?;
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
    for part in [&head, &branch, &refs, &status, &config, &remotes] {
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
    // One budget for the whole card: its fingerprint and its preview (issue #359).
    git_read::call_scope(LOCAL_DEADLINE, || preview_card(root, name, arguments))
}

fn preview_card(
    root: &Path,
    name: &str,
    arguments: &Value,
) -> Option<Result<(Change, CardKind), String>> {
    let fingerprint = match fingerprint_state(root, name, arguments)? {
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
            verb(args)
        ));
    }
    Ok(output)
}

/// [`full`] for a listing that decides a secret check: git's stderr warnings refuse it
/// too (see [`git_read::git_complete_listing`]).
fn full_complete(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_read::git_complete_listing(root, args, FULL)?;
    if output.len() >= FULL {
        return Err(format!(
            "git {} output is too large to check safely; refused",
            verb(args)
        ));
    }
    Ok(output)
}

/// The git subcommand of an argument list, for messages (`git` alone when empty).
fn verb<'a>(args: &[&'a str]) -> &'a str {
    args.first().copied().unwrap_or("git")
}

/// Repository-local and worktree config keys a chat git write may run under (#309). None
/// of them can run a program, redirect a remote or change what a card showed.
const ALLOWED_LOCAL_KEYS: [&str; 18] = [
    "core.autocrlf",
    "core.bare",
    "core.filemode",
    "core.ignorecase",
    "core.logallrefupdates",
    "core.precomposeunicode",
    "core.repositoryformatversion",
    "core.sparsecheckout",
    "core.sparsecheckoutcone",
    "index.sparse",
    "core.symlinks",
    "extensions.objectformat",
    "extensions.worktreeconfig",
    "init.defaultbranch",
    "pull.rebase",
    "push.default",
    "user.email",
    "user.name",
];

/// Whether a repository-local `key` (lowercase) with `value` is on the allowlist. Remote
/// and branch keys are per name (`remote.<name>.url`); `commit.gpgsign` only when false.
fn allowed_local_key(key: &str, value: &str) -> bool {
    if key == "commit.gpgsign" {
        return matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "false" | "no" | "off" | "0"
        );
    }
    if ALLOWED_LOCAL_KEYS.contains(&key) {
        return true;
    }
    match (key.split_once('.'), key.rsplit_once('.')) {
        (Some((section, _)), Some((head, variable))) if head.len() > section.len() => match section
        {
            "remote" => matches!(variable, "url" | "pushurl" | "fetch" | "push" | "tagopt"),
            "branch" => matches!(
                variable,
                "remote" | "merge" | "rebase" | "pushremote" | "description"
            ),
            _ => false,
        },
        _ => false,
    }
}

/// A config key or URL for a message: printable characters only, at most 80.
fn shown(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(80)
        .collect()
}

/// Refuse when repository-local config (not the owner's global or system config) sets a
/// key outside [`ALLOWED_LOCAL_KEYS`]: a signing key or proxy command, a merge driver, a
/// pager, an editor, a helper, a URL rewrite or a protocol override would make git run a
/// program or send data elsewhere. Refused by name, for every chat git write.
fn refuse_repository_config(root: &Path) -> Result<(), String> {
    let listing = config_listing(root)?;
    // Records are `scope NUL key LF value NUL` (a bare key has no LF). A subsection name
    // may hold `=`, so the key is split from the value at the newline, never at `=`.
    let mut fields = listing.split('\0');
    while let (Some(scope), Some(entry)) = (fields.next(), fields.next()) {
        if !matches!(scope, "local" | "worktree") {
            continue;
        }
        let (key, value) = entry.split_once('\n').unwrap_or((entry, "true"));
        let key = key.to_lowercase();
        if !allowed_local_key(&key, value) {
            return Err(format!(
                "repository config sets {}, which chat git writes do not allow (they can run a program or redirect a remote); refused (the owner can run this git command by hand)",
                shown(&key)
            ));
        }
    }
    Ok(())
}

/// The repository config as a write reads it (`--show-scope`), read whole: local, worktree
/// and the owner's global config, never a cut listing.
fn config_listing(root: &Path) -> Result<String, String> {
    let clock = git_read::call_clock();
    let mut command = local_command(root, clock)?;
    command.args(["config", "--null", "--list", "--show-scope"]);
    let (status, out, error) = git_read::run_bounded(command, FULL, clock.left())?;
    match status {
        None => Err("repository config is too large to check; refused".into()),
        Some(status) if status.success() => Ok(String::from_utf8_lossy(&out).into_owned()),
        Some(_) => Err(format!(
            "repository config cannot be read ({})",
            error
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("git config failed")
                .trim()
                .trim_start_matches("fatal: ")
        )),
    }
}

/// `git -C root` for a local write: hardened, with the owner's HOME (global identity) and
/// no other environment; discovery stops above the parent. Pinned to the verified git
/// directory, so a write never lands in another repository (issue #359).
fn local_command(root: &Path, clock: git_read::Clock) -> Result<Command, String> {
    let verified = git_read::verified_git_dir_within(root, clock, git_read::Config::Owner)?;
    let ceiling = ceiling(root)?;
    let mut command = Command::new("/usr/bin/git");
    git_read::owner_config(git_read::hardened(&mut command));
    command
        .env("GIT_CEILING_DIRECTORIES", ceiling)
        .arg("-C")
        .arg(root);
    git_read::pin_git_dir(&mut command, root, &verified);
    Ok(command)
}

/// The directory above the repository, which git discovery may not climb past.
fn ceiling(root: &Path) -> Result<std::path::PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    Ok(root.parent().unwrap_or(&root).to_path_buf())
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
                indent(&self.listing)
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
    refuse_repository_config(root)?;
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
    refuse_long_list(
        "commit",
        listing.lines().count(),
        "commit fewer files at a time (pass files, or git add fewer)",
    )?;
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
    // Status shows a rename only when both names are in the pathspec, and the staged
    // name listing above shows a copy as a plain add, so a secret staged under a new
    // name (`git mv .npmrc docs/notes.txt`, `cp .npmrc config.txt`) is found here: the
    // sources of every staged rename or copy in the whole index, in either mode.
    let index = full_complete(
        root,
        &[
            "diff",
            "--cached",
            "--no-ext-diff",
            "--find-renames",
            "--find-copies",
            "--find-copies-harder",
            "-l0",
            "--name-status",
            "-z",
            "--",
        ],
    )?;
    // Read lossily: a replacement character may hide the bytes that made a name secret.
    if index.contains('\u{FFFD}') {
        return Err("a staged file name is not UTF-8; the commit is refused".into());
    }
    let mut fields = index.split('\0').filter(|f| !f.is_empty());
    while let Some(status) = fields.next() {
        let first = fields.next();
        if status.starts_with(['R', 'C']) {
            names.extend(first.map(String::from));
            fields.next();
        }
    }
    match names
        .iter()
        .find(|name| crate::planning_context::secret(name))
    {
        Some(name) => Err(format!(
            "refused: {name} looks like a secret or credential file (see the secret-file list in docs/reference.md); commit it by hand if intended"
        )),
        None => Ok(()),
    }
}

/// The `commit` tool: one budget for the whole call (issue #359).
fn commit(root: &Path, arguments: &Value) -> Result<String, String> {
    git_read::call_scope(LOCAL_DEADLINE, || commit_run(root, arguments))
}

fn commit_run(root: &Path, arguments: &Value) -> Result<String, String> {
    let plan = plan_commit(root, arguments)?;
    let mut args = vec!["commit", "--quiet", "--no-verify", "-m", &plan.message];
    if !plan.pathspecs.is_empty() {
        let mut add = vec!["add", "--"];
        add.extend(plan.pathspecs.iter().map(String::as_str));
        let intent = intent_to_add(root)?;
        let index = write(root, &["write-tree"], Env::Local)?;
        let index = index.trim();
        write(root, &add, Env::Local).map_err(|error| error + &leftovers(root))?;
        // Re-check the staged set the commit will take: a plain-named file holding a copy
        // of a secret is only visible once it is staged (`plan_commit` saw it untracked).
        if let Err(error) = refuse_secrets(root, &plan.pathspecs) {
            return Err(error + &restore_index(root, index, &intent) + &leftovers(root));
        }
        args.push("--");
        args.extend(plan.pathspecs.iter().map(String::as_str));
        if let Err(error) = write(root, &args, Env::Local) {
            return Err(error + &restore_index(root, index, &intent) + &leftovers(root));
        }
    } else {
        write(root, &args, Env::Local).map_err(|error| error + &leftovers(root))?;
    }
    let made = git_read::git(root, &["log", "-1", "--no-color", "--format=%h %s"])?;
    Ok(format!("committed on {}: {}", plan.branch, made.trim()))
}

/// Put the index back to the tree `write-tree` recorded before `git add`; the message
/// says whether that worked.
/// `read-tree` drops intent-to-add entries (`git add -N`), so they are added again.
fn restore_index(root: &Path, index: &str, intent: &[String]) -> String {
    let restored =
        write(root, &["read-tree", "--end-of-options", index], Env::Local).and_then(|_| {
            if intent.is_empty() {
                return Ok(String::new());
            }
            let mut args = vec!["add", "--intent-to-add", "--"];
            args.extend(intent.iter().map(String::as_str));
            write(root, &args, Env::Local)
        });
    match restored {
        Ok(_) => "; the index was restored".to_string(),
        Err(reset) => format!("; restoring the index failed ({reset})"),
    }
}

/// Paths that are intent-to-add entries in the index (new to the index, not to the
/// worktree; the worktree diff lists them as added).
fn intent_to_add(root: &Path) -> Result<Vec<String>, String> {
    let out = full(
        root,
        &["diff", "--name-only", "--diff-filter=A", "-z", "--"],
    )?;
    Ok(out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect())
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
    refuse_repository_config(root)?;
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

/// The `switch` tool: one budget for the whole call (issue #359).
fn switch(root: &Path, arguments: &Value) -> Result<String, String> {
    git_read::call_scope(LOCAL_DEADLINE, || switch_run(root, arguments))
}

fn switch_run(root: &Path, arguments: &Value) -> Result<String, String> {
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
    refuse_repository_config(root)?;
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
    // `--numstat -z` names every path whole (`--stat` shortens a long or control-byte
    // name to `.../`), and `full` refuses a listing git printed past the read limit
    // rather than trusting its first bytes (issue #269).
    let raw = full(
        root,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--numstat",
            "-z",
            "--end-of-options",
            &three,
            "--",
        ],
    )?;
    let files = merge_file_list(&raw);
    refuse_long_list(
        "merge file list",
        files.lines().count(),
        "merge a smaller branch, or land the work in smaller pieces",
    )?;
    Ok(MergePlan {
        source,
        target: current,
        count,
        commits: commits.trim_end().to_string(),
        files,
    })
}

/// One row per changed file, `path | +added -deleted`, and a count row. Names are shown
/// whole with control and invisible characters escaped, so a card row is what the path is.
fn merge_file_list(raw: &str) -> String {
    let mut rows = Vec::new();
    for entry in raw.split('\0').filter(|entry| !entry.is_empty()) {
        let mut parts = entry.splitn(3, '\t');
        let (added, deleted, path) = (
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
        );
        let counts = if added == "-" {
            "binary".to_string()
        } else {
            format!("+{added} -{deleted}")
        };
        rows.push(format!("{} | {counts}", visible_name(path)));
    }
    let count = rows.len();
    rows.push(format!(
        "{count} file{} changed",
        if count == 1 { "" } else { "s" }
    ));
    rows.join("\n")
}

/// A path with control characters written as `\xNN` and invisible ones as `<U+XXXX>`.
fn visible_name(path: &str) -> String {
    path.chars()
        .map(|c| {
            if c.is_control() {
                format!("\\x{:02x}", c as u32)
            } else {
                crate::dashboard::safe_visible(&c.to_string())
            }
        })
        .collect()
}

/// Refuses a card list over [`MAX_CARD_LINES`]: the owner would approve lines the card
/// cannot show. Runs while planning, so before any card and again at apply.
fn refuse_long_list(what: &str, lines: usize, advice: &str) -> Result<(), String> {
    if lines > MAX_CARD_LINES {
        return Err(format!(
            "the {what} is {lines} lines, over the {MAX_CARD_LINES}-line approval card limit; nothing was changed. {advice}"
        ));
    }
    Ok(())
}

/// The `merge` tool: one budget for the whole call (issue #359).
fn merge(root: &Path, arguments: &Value) -> Result<String, String> {
    git_read::call_scope(LOCAL_DEADLINE, || merge_run(root, arguments))
}

fn merge_run(root: &Path, arguments: &Value) -> Result<String, String> {
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

/// Whether git_push may send to `url`: `https://`, `ssh://` or scp-style `[user@]host:path`
/// (#309). The URL is parsed as one fixed form: the scheme is exactly lowercase, the
/// authority ends at the first `/`, it holds at most one `@`, and the host is a plain name
/// that cannot read as an option. Characters that libcurl or git read differently from a
/// plain host (`?`, `#`, `\`, `%`, whitespace, control characters) and every other remote
/// helper (`ext::`, `file://`, `git://`, `http://`, local paths) are refused.
fn push_url_allowed(url: &str) -> bool {
    let odd = |c: char| c.is_whitespace() || c.is_control() || "?#\\%".contains(c);
    if url.chars().any(odd) || url.matches('@').count() > 1 || url.contains("::") {
        return false;
    }
    let plain = |text: &str| {
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    let word = |text: &str| plain(text) && text.starts_with(|c: char| c.is_ascii_alphanumeric());
    let user_ok = |user: &str| {
        !user.is_empty()
            && !user.starts_with('-')
            && user
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-:".contains(c))
    };
    if let Some((scheme, rest)) = url.split_once("://") {
        if scheme != "https" && scheme != "ssh" {
            return false;
        }
        let authority = rest.split('/').next().unwrap_or_default();
        let (user, hostport) = match authority.split_once('@') {
            Some((user, hostport)) => (Some(user), hostport),
            None => (None, authority),
        };
        let (host, port_ok) = match hostport.split_once(':') {
            Some((host, port)) => (
                host,
                !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()),
            ),
            None => (hostport, true),
        };
        return word(host) && port_ok && user.is_none_or(user_ok);
    }
    // scp-style: git reads the host before the first `:` and the path after it, so the
    // path holds no `:` or `@`, and the host is a name, not a URL scheme word (#309).
    let Some((left, path)) = url.split_once(':') else {
        return false;
    };
    let (user, host) = match left.split_once('@') {
        Some((user, host)) => (Some(user), host),
        None => (None, left),
    };
    let scheme_word = ["https", "http", "ssh", "git", "file", "ext", "fd"]
        .iter()
        .any(|scheme| host.eq_ignore_ascii_case(scheme));
    let named = host.chars().any(|c| c.is_ascii_alphabetic() || c == '.');
    !path.is_empty()
        && !path.contains([':', '@'])
        && word(host)
        && named
        && !scheme_word
        && user.is_none_or(word)
}

/// A URL for a refusal message: everything before the last `@` (user info, which may be a
/// token) is replaced by `***`, the scheme is kept, control characters are escaped.
fn redacted(url: &str) -> String {
    let scheme_end = url.find("://").map_or(0, |i| i + 3);
    let shown = match url.rfind('@') {
        Some(at) => {
            let keep = if at < scheme_end {
                ""
            } else {
                &url[..scheme_end]
            };
            format!("{keep}***{}", &url[at..])
        }
        None => url.to_string(),
    };
    shown
        .chars()
        .map(|c| {
            if c.is_control() {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect::<String>()
        .chars()
        .take(120)
        .collect()
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
    refuse_repository_config(root)?;
    let branch = match ref_arg(arguments, "branch")? {
        Some(branch) => branch.to_string(),
        None => current_branch(root)?,
    };
    if !local_branch_exists(root, &branch) {
        return Err(format!("no local branch {branch}"));
    }
    let urls = full(
        root,
        &["remote", "get-url", "--push", "--all", "--", &remote],
    )?;
    // The raw values are checked (no trimming): git uses them as they are.
    let urls: Vec<&str> = urls.split('\n').filter(|url| !url.is_empty()).collect();
    if urls.is_empty() {
        return Err(format!("remote {remote} has no push URL"));
    }
    if let Some(url) = urls.iter().find(|url| !push_url_allowed(url)) {
        return Err(format!(
            "remote {remote} URL {} is not https://, ssh:// or user@host:path; chat git_push refuses it (the owner can push by hand)",
            redacted(url)
        ));
    }
    let url = urls
        .iter()
        .map(|url| without_credentials(url))
        .collect::<Vec<_>>()
        .join(", ");
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

/// The `push` tool: one budget for the whole call (issue #359).
fn push(root: &Path, arguments: &Value) -> Result<String, String> {
    git_read::call_scope(PUSH_DEADLINE, || push_run(root, arguments))
}

fn push_run(root: &Path, arguments: &Value) -> Result<String, String> {
    let plan = plan_push(root, arguments)?;
    let refspec = format!("refs/heads/{0}:refs/heads/{0}", plan.branch);
    write(
        root,
        &[
            "push",
            "--no-verify",
            "--no-follow-tags",
            "--no-recurse-submodules",
            "--no-signed",
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

/// Git's own error lines, with the terminal hint appended when a remote command needed a
/// prompt, so the real failure (for example an HTTPS credential error) is never lost.
fn failure_message(args: &[&str], error: &str, remote: bool) -> String {
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
    let message = if lines.is_empty() {
        format!("git {} failed", verb(args))
    } else {
        lines.join("; ")
    };
    if remote && needed_terminal(error) {
        format!("{message}\n{TERMINAL_HINT}")
    } else {
        message
    }
}

/// Run a writing git command in `root`. `Err` is git's message.
fn write(root: &Path, args: &[&str], env: Env) -> Result<String, String> {
    let (mut command, clock) = match env {
        Env::Local => {
            let clock = git_read::call_clock();
            (local_command(root, clock)?, clock)
        }
        Env::Remote => {
            let clock = git_read::call_clock();
            let verified = git_read::verified_git_dir_within(root, clock, git_read::Config::Owner)?;
            let mut command = Command::new("/usr/bin/git");
            for name in REDIRECTING_ENV {
                command.env_remove(name);
            }
            // After the removals: GIT_CEILING_DIRECTORIES is in REDIRECTING_ENV too.
            command
                .env("GIT_CEILING_DIRECTORIES", ceiling(root)?)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("LC_ALL", "C")
                .env("GIT_PAGER", "cat")
                .arg("-C")
                .arg(root);
            // Pinned like the local writes: a push cannot be redirected either (issue #359).
            git_read::pin_git_dir(&mut command, root, &verified)
                .arg("--no-pager")
                .args(crate::worker::HARDENED_GIT_ARGS)
                .args(["-c", "protocol.ext.allow=never"]);
            (command, clock)
        }
    };
    command.args(args);
    detach_from_terminal(&mut command);
    let (status, out, error) = git_read::run_bounded(command, MAX_OUTPUT * 2, clock.left())?;
    match status {
        // Cut output: git was stopped part-way, so the result is not known. Never success.
        None => Err(format!(
            "git {} printed too much output and was stopped; check git status",
            verb(args)
        )),
        Some(status) if !status.success() => {
            Err(failure_message(args, &error, matches!(env, Env::Remote)))
        }
        Some(_) => Ok(String::from_utf8_lossy(&out).into_owned()),
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
    fn push_urls_are_https_ssh_or_scp_style_only() {
        for url in [
            "https://github.com/o/r.git",
            "https://user:tok@github.com:443/o/r.git",
            "ssh://git@github.com/o/r.git",
            "ssh://git@github.com:22/o/r.git",
            "git@github.com:o/r.git",
            "github.com:o/r.git",
            "user@host:~/x",
            "host:path/x.git",
            "gitlab.example.com:group/r.git",
        ] {
            assert!(push_url_allowed(url), "{url}");
        }
        for url in [
            "/tmp/r.git",
            "../r.git",
            "./r.git",
            "file:///tmp/r.git",
            "git://127.0.0.1:9/x.git",
            "http://example.com/x.git",
            "ftp://example.com/x.git",
            "ext::sh -c touch MARK",
            "fd::3",
            "ssh://-oProxyCommand=x/r.git",
            "https://evil-probe.invalid?@github.com/o/r.git",
            "https://evil-probe.invalid#@github.com/o/r.git",
            "https://a@b@github.com/x",
            "https://github.com\\@evil/x",
            "ssh://a@b@github.com/o/r.git",
            "git@github.com@evil:o/r.git",
            "HTTPS://github.com/o/r.git",
            // scp-style: a URL scheme word, a second `:` or `@` in the path, no name.
            "evil.invalid:x@github.com:o/r.git",
            "https:github.com/o/r.git",
            "ssh:github.com/o/r.git",
            "evil:tok@github.com:o/r.git",
            "git@github.com:o/r.git:x",
            "git@github.com:o/r@x",
            "1234:o/r.git",
            " https://github.com/o/r.git",
            "https://github.com/o/r.git ",
            "git@-oProxy:o/r.git",
            "-oProxyCommand=x:o/r.git",
            "https://github.com:port/o/r.git",
            "https:///o/r.git",
            "host:",
        ] {
            assert!(!push_url_allowed(url), "{url}");
        }
    }

    #[test]
    fn refused_urls_are_shown_without_user_info() {
        assert_eq!(
            redacted("https://a@b@github.com/x"),
            "https://***@github.com/x"
        );
        assert_eq!(
            redacted("evil:tok@github.com:o/r.git"),
            "***@github.com:o/r.git"
        );
        assert_eq!(redacted("/tmp/r\u{1b}[2J.git"), "/tmp/r\\u{1b}[2J.git");
    }

    #[test]
    fn terminal_failures_keep_git_error_and_add_the_hint() {
        let https =
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled\n";
        let message = failure_message(&["push"], https, true);
        assert!(message.contains("could not read Username"), "{message}");
        assert!(message.ends_with(TERMINAL_HINT), "{message}");
        let ssh = failure_message(&["push"], "Host key verification failed.\n", true);
        assert!(ssh.contains("Host key verification failed"), "{ssh}");
        assert!(ssh.ends_with(TERMINAL_HINT), "{ssh}");
        let local = failure_message(&["commit"], "fatal: could not read Username\n", false);
        assert!(!local.contains(TERMINAL_HINT), "{local}");
    }

    #[test]
    fn ssh_terminal_failures_are_recognised() {
        assert!(needed_terminal("Host key verification failed.\n"));
        assert!(needed_terminal("could not open /dev/tty: No such device"));
        assert!(!needed_terminal("Permission denied (publickey)."));
    }
}
