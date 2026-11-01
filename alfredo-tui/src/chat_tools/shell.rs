//! Mutating tier (issue #236, ADR 0004): run_command runs one shell command after
//! owner Approval. By default it runs in the bubblewrap Sandbox shared with worker
//! checks ([`crate::worker::sandbox_argv`]): Coding Workspace writable, `.git` and
//! system roots read-only, network off. `escalate: true` runs it on the host
//! (network on) behind a visibly different card. Both run in their own process
//! group: a timeout kills the whole tree; output is capped. Secret files and directories
//! in the workspace are masked inside the sandbox (issue #469, [`sandbox_masks`]); a run
//! that cannot mask them all is refused. A sandbox that cannot start is reported with a
//! suggestion to escalate; nothing is retried on the host.
use super::{display, git_read, needed_terminal, resolve, text_arg, Tier, Tool};
use crate::execution::{
    self, ControlSignal, ExecutionAuthority, ExecutionCallbacks, ExecutionLimits, ExecutionRequest,
    ExecutionSandbox, ProcessOutcomeStatus, RustExecutionProvider,
};
use sandbox_masks::EmptyDir;
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// Flag that ends the `run_command` call holding it (issue #302).
#[derive(Clone, Default)]
pub(crate) struct Stop(Arc<AtomicBool>);

impl Stop {
    fn stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Owns a [`Stop`] and sets it on drop, so a turn aborted by Esc, `/stop`, a chat
/// switch, a retry or quit kills the command's process group instead of leaving it to
/// finish (its future, which holds this, is dropped on abort).
#[derive(Default)]
pub(crate) struct StopOnDrop {
    stop: Stop,
    /// Set by the owner once it has the call's result, so a normal drop is not an abort.
    finished: Arc<AtomicBool>,
    /// Set (before the stop) when dropped without `finish`: nobody will read the result.
    abandoned: Arc<AtomicBool>,
}

impl StopOnDrop {
    pub(crate) fn stop(&self) -> Stop {
        self.stop.clone()
    }

    /// Whether the turn was aborted, so the call's result must be reported by the
    /// thread that ran it (issue #333).
    pub(crate) fn abandoned(&self) -> Abandoned {
        Abandoned(self.abandoned.clone())
    }

    /// The call's result was received: dropping this is not an abort.
    pub(crate) fn finish(&self) {
        self.finished.store(true, Ordering::SeqCst);
    }
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if !self.finished.load(Ordering::SeqCst) {
            self.abandoned.store(true, Ordering::SeqCst);
        }
        self.stop.0.store(true, Ordering::SeqCst);
    }
}

/// Whether the turn that owned a call was aborted before it read the result.
#[derive(Clone)]
pub(crate) struct Abandoned(Arc<AtomicBool>);

impl Abandoned {
    pub(crate) fn get(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// First words of the result of a `run_command` that was killed mid-run (issue #333):
/// `cancelled after Ns · killed; it may have partly run`. A command stopped before it
/// started reports `error: cancelled before it started` instead, since nothing ran.
pub const CANCELLED_RESULT: &str = "cancelled after ";

/// Stop flags of the calls in flight, for [`stop_all`].
static ACTIVE: Mutex<Vec<Arc<AtomicBool>>> = Mutex::new(Vec::new());

thread_local! {
    static CURRENT: RefCell<Option<Stop>> = const { RefCell::new(None) };
}

/// Run `body` (an approved call and the logging of its outcome) so that `stop` ends a
/// `run_command` inside it and [`stop_all`] waits for it.
pub(crate) fn scoped<T>(stop: &Stop, body: impl FnOnce() -> T) -> T {
    struct Leave(Arc<AtomicBool>);
    impl Drop for Leave {
        fn drop(&mut self) {
            if let Ok(mut active) = ACTIVE.lock() {
                active.retain(|flag| !Arc::ptr_eq(flag, &self.0));
            }
            CURRENT.with(|current| *current.borrow_mut() = None);
        }
    }
    if let Ok(mut active) = ACTIVE.lock() {
        active.push(stop.0.clone());
    }
    CURRENT.with(|current| *current.borrow_mut() = Some(stop.clone()));
    let _leave = Leave(stop.0.clone());
    body()
}

/// Stop every `run_command` in flight and wait up to `wait` for them to be killed and
/// logged. True when none is left. Called on quit so a host command never outlives
/// Alfredo (issue #302).
pub fn stop_all(wait: Duration) -> bool {
    if let Ok(active) = ACTIVE.lock() {
        for flag in active.iter() {
            flag.store(true, Ordering::SeqCst);
        }
    }
    let deadline = Instant::now() + wait;
    loop {
        if ACTIVE.lock().map_or(true, |active| active.is_empty()) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The stop of the call running on this thread; a fresh one outside [`scoped`].
fn current_stop() -> Stop {
    CURRENT.with(|current| current.borrow().clone().unwrap_or_default())
}

fn cancel_signal(stop: &Stop) -> Result<(), ControlSignal> {
    if stop.stopped() {
        Err(ControlSignal::Cancelled(
            "the owner stopped the command".into(),
        ))
    } else {
        Ok(())
    }
}

/// Default and largest timeout, in seconds.
const DEFAULT_TIMEOUT: u64 = 120;
const MAX_TIMEOUT: u64 = 600;
/// Largest command text, in bytes.
const MAX_COMMAND: usize = 8 * 1024;
/// Output kept from the process before it is stopped (the model sees less; see `cap`).
const OUTPUT_LIMIT: usize = 1024 * 1024;
/// Shown to the model when a host command fails because it wanted a terminal prompt.
const TERMINAL_HINT: &str = "note: host commands run without a terminal, so a command that needs a terminal prompt (sudo password, ssh host key or passphrase, git credentials) cannot run here; use ssh-agent or a credential helper, or ask the owner to run it themselves.";
/// Shown to the model when a sandboxed command fails.
const ESCALATE_HINT: &str = "note: ran sandboxed (network off; only the Coding Workspace is writable; .git and tools outside /usr are not available). If it failed for that reason, call run_command again with escalate: true; the owner must approve running it on the host.";

pub const RUN_COMMAND: Tool = Tool {
    name: "run_command",
    tier: Tier::Mutating,
    description: "Run one shell command (sh -c) in the Coding Workspace, such as tests or a build. Needs the owner's approval each call; the owner sees the exact command and cwd. Runs sandboxed by default: network off, only the working tree writable, .git read-only. Set escalate: true only when the command needs the network or the real host (git push, installing packages); the owner sees a host warning and may deny. Returns the exit code and output.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Shell command, run with sh -c"},
                "cwd": {"type": "string", "description": "Directory relative to the repository root (default: the root)"},
                "escalate": {"type": "boolean", "description": "Run on the host with network on instead of the sandbox (default false)"},
                "timeout_seconds": {"type": "integer", "description": "Kill the command and its children after this many seconds (default 120, at most 600)"}
            },
            "required": ["command"]
        })
    },
    run: run_command,
};

/// A validated call.
struct Plan {
    command: String,
    cwd: PathBuf,
    shown_cwd: String,
    escalate: bool,
    timeout: u64,
}

/// What the approval card shows for a run_command call.
pub(crate) struct Shown {
    pub summary: String,
    pub preview: String,
    pub escalated: bool,
    /// Canonical cwd at preview; an approved call refuses if it resolves elsewhere.
    pub cwd: String,
}

/// Canonical cwd of a run_command call (see [`Shown::cwd`]).
pub(crate) fn cwd(root: &Path, arguments: &Value) -> Result<String, String> {
    plan(root, arguments).map(|plan| plan.cwd.to_string_lossy().into_owned())
}

/// Card content, or why the call is refused before asking. `None` for other tools.
pub(crate) fn preview(root: &Path, name: &str, arguments: &Value) -> Option<Result<Shown, String>> {
    if name != RUN_COMMAND.name {
        return None;
    }
    Some(plan(root, arguments).map(|plan| {
        let place = if plan.escalate {
            "HOST · network on · your environment and credentials (SSH agent, tokens) · outside the sandbox"
        } else {
            "sandboxed · network off · only the Coding Workspace writable"
        };
        let mut preview = format!(
            "cwd {} · {place} · timeout {}s",
            plan.shown_cwd, plan.timeout
        );
        for line in plan.command.lines() {
            preview.push_str(&format!("\n$ {line}"));
        }
        Shown {
            summary: format!(
                "{} {}",
                if plan.escalate { "run on HOST" } else { "run" },
                plan.command
            ),
            preview,
            escalated: plan.escalate,
            cwd: plan.cwd.to_string_lossy().into_owned(),
        }
    }))
}

fn plan(root: &Path, arguments: &Value) -> Result<Plan, String> {
    let command = text_arg(arguments, "command")?
        .filter(|c| !c.trim().is_empty())
        .ok_or("argument command is required")?;
    if command.len() > MAX_COMMAND || command.contains('\0') {
        return Err(format!(
            "command must be at most {MAX_COMMAND} bytes of text"
        ));
    }
    let escalate = match arguments.get("escalate") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err("argument escalate must be true or false".into()),
    };
    let timeout = match arguments.get("timeout_seconds") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT,
        Some(value) => value
            .as_u64()
            .filter(|t| (1..=MAX_TIMEOUT).contains(t))
            .ok_or(format!(
                "argument timeout_seconds must be a whole number from 1 to {MAX_TIMEOUT}"
            ))?,
    };
    let cwd = match text_arg(arguments, "cwd")?.filter(|c| !c.trim().is_empty()) {
        Some(cwd) => resolve(root, cwd)?,
        None => root
            .canonicalize()
            .map_err(|_| "Coding Workspace is not available".to_string())?,
    };
    if !cwd.is_dir() {
        return Err("cwd is not a directory".into());
    }
    let shown_cwd = match display(root, &cwd) {
        shown if shown.is_empty() => ".".to_string(),
        shown => shown,
    };
    Ok(Plan {
        command: command.to_string(),
        cwd,
        shown_cwd,
        escalate,
        timeout,
    })
}

fn run_command(root: &Path, arguments: &Value) -> Result<String, String> {
    let plan = plan(root, arguments)?;
    let stop = current_stop();
    if stop.stopped() {
        return Err("cancelled before it started; nothing ran".into());
    }
    if plan.escalate {
        on_host(&plan, &stop)
    } else {
        sandboxed(root, &plan, crate::doctor::BWRAP, &stop)
    }
}

/// How a run ended, from either launcher.
struct Ran {
    status: ProcessOutcomeStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn limits(timeout: u64) -> ExecutionLimits {
    ExecutionLimits {
        timeout_seconds: timeout as f64,
        output_limit_bytes: OUTPUT_LIMIT,
        address_space_bytes: 32 * 1024 * 1024 * 1024,
        file_size_bytes: 1024 * 1024 * 1024,
        open_file_limit: 4096,
        process_count_limit: 1024,
        descendant_grace_seconds: 1.0,
    }
}

fn request(
    argv: Vec<String>,
    cwd: String,
    plan: &Plan,
    sandbox: ExecutionSandbox,
) -> ExecutionRequest {
    ExecutionRequest {
        schema_version: execution::EXECUTION_SCHEMA_VERSION,
        request_id: "chat-run-command".into(),
        effect: "local-agent".into(),
        argv,
        working_directory: cwd,
        authority: ExecutionAuthority::LocalAgent {
            mission_id: "chat".into(),
            session_id: "chat".into(),
            session_revision: 1,
            runner_operation_id: "chat:run_command".into(),
            worktree_identity: "chat:workspace".into(),
            allowed_paths: Vec::new(),
        },
        limits: limits(plan.timeout),
        sandbox,
        environment: BTreeMap::new(),
        input_text: None,
        input_sha256: None,
        shell: false,
    }
}

fn sandbox_failure(reason: &str) -> String {
    format!(
        "the sandbox could not run the command ({}); nothing ran on the host. If the command must run, call run_command again with escalate: true and the owner will be asked to approve running it on the host.",
        reason.trim()
    )
}

/// Text of the placeholder that holds `root/.git` while a sandboxed command runs in a
/// Coding Workspace with no Git repository (issue #522). The exact text tells the host the
/// file is the placeholder and not the owner's file.
const PLACEHOLDER_TEXT: &str = "alfredo: this Coding Workspace has no Git repository\n";

/// Placeholders this process has written, with the number of runs holding each one.
static PLACEHOLDERS: Mutex<Vec<Placeholder>> = Mutex::new(Vec::new());

struct Placeholder {
    path: PathBuf,
    /// Device and inode of the file this process wrote.
    identity: (u64, u64),
    holders: usize,
}

/// One sandboxed run's hold on a placeholder; the last hold removes the file.
struct Hold(PathBuf);

/// Keeps `git` (the workspace's `root/.git`) occupied for one sandboxed run, so that
/// bubblewrap can bind it read-only and `git init`, `mkdir .git` or `ln -s` inside the
/// sandbox fail with the file in place (issue #522). When `git` is absent, this writes the
/// placeholder; when it is already a placeholder held by a concurrent run, this joins that
/// hold. `None` when the owner's own `.git` is there, which is bound as before.
fn hold_placeholder(git: &Path) -> Result<Option<Hold>, String> {
    let mut held = PLACEHOLDERS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(entry) = held.iter_mut().find(|entry| entry.path == git) {
        entry.holders += 1;
        return Ok(Some(Hold(git.to_path_buf())));
    }
    match fs::symlink_metadata(git) {
        Ok(_) if is_placeholder(git) => {
            // Left by an earlier process that died during a run: replace it.
            fs::remove_file(git).map_err(|error| error.to_string())?;
        }
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let mut file = match OpenOptions::new().write(true).create_new(true).open(git) {
        Ok(file) => file,
        // Something appeared since the check: bind that as the owner's entry.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    file.write_all(PLACEHOLDER_TEXT.as_bytes())
        .map_err(|error| error.to_string())?;
    let meta = file.metadata().map_err(|error| error.to_string())?;
    held.push(Placeholder {
        path: git.to_path_buf(),
        identity: (meta.dev(), meta.ino()),
        holders: 1,
    });
    Ok(Some(Hold(git.to_path_buf())))
}

/// Whether `git` is a regular file holding exactly the placeholder text.
fn is_placeholder(git: &Path) -> bool {
    fs::symlink_metadata(git).is_ok_and(|meta| meta.is_file())
        && fs::read_to_string(git).is_ok_and(|text| text == PLACEHOLDER_TEXT)
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut held = PLACEHOLDERS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(index) = held.iter().position(|entry| entry.path == self.0) else {
            return;
        };
        held[index].holders -= 1;
        if held[index].holders > 0 {
            return;
        }
        let entry = held.remove(index);
        // Removed only while it is still the file this process wrote: an owner's file put in
        // its place, or a placeholder another process wrote, is left alone.
        let same = fs::symlink_metadata(&entry.path)
            .is_ok_and(|meta| (meta.dev(), meta.ino()) == entry.identity);
        if same && is_placeholder(&entry.path) {
            let _ = fs::remove_file(&entry.path);
        }
    }
}

mod sandbox_masks {
    //! Secret masks for the chat sandbox (issue #469). The sandbox binds the Coding Workspace
    //! read-write, so a rule-approved or carded `run_command` could read a secret file that
    //! the read tools refuse. Before each sandboxed run, every workspace entry the secret
    //! predicate ([`crate::planning_context::secret`]) matches is hidden: a secret file is
    //! bound over with `/dev/null`, and a secret directory with an empty read-only directory.
    //!
    //! The walk is bounded and does not follow symbolic links. It includes git-ignored files,
    //! which are exactly the ones a `.gitignore` leaves out of Git. It skips `.git` at every
    //! depth: Git needs its own directory, the root `.git` is already read-only in the sandbox,
    //! and its contents are not masked one by one. When the walk cannot finish (an unreadable
    //! directory, the visit or depth budget, the mask cap, a secret-named symbolic link or a
    //! name that is not UTF-8), it refuses, so the caller runs nothing rather than run unmasked.
    use crate::planning_context;
    use std::{
        fs,
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
    };

    /// Most masks (files and directories together) one run may carry. The masks are read-only
    /// binds declared with the sandbox's other read-only bindings, which the execution
    /// validator caps at 256 entries; the rest of the run takes a handful of those.
    pub(crate) const LIMIT: usize = 128;
    /// Entries one walk may visit before it refuses, so a huge tree cannot stall a run.
    const VISIT_LIMIT: usize = 250_000;
    /// Directory depth one walk may descend before it refuses.
    const DEPTH_LIMIT: usize = 64;

    /// The workspace entries the sandbox must not see. Paths are absolute and canonical, so
    /// they match the mount destinations the sandbox is built with.
    #[derive(Debug, Default, PartialEq, Eq)]
    pub(crate) struct Masks {
        /// Non-directory entries (regular files, and anything else not a directory) that are
        /// bound over with `/dev/null`.
        pub(crate) files: Vec<String>,
        /// Secret directories, bound over with an empty read-only directory (see [`EmptyDir`]).
        pub(crate) dirs: Vec<String>,
    }

    impl Masks {
        fn len(&self) -> usize {
            self.files.len() + self.dirs.len()
        }
    }

    /// Walk `root` (canonical) and collect its masks, or say why they cannot be collected.
    pub(crate) fn scan(root: &Path) -> Result<Masks, String> {
        scan_with(root, VISIT_LIMIT)
    }

    fn scan_with(root: &Path, visit_limit: usize) -> Result<Masks, String> {
        let mut scan = Scan {
            masks: Masks::default(),
            visits: 0,
            visit_limit,
        };
        scan.directory(root, "", 0)?;
        Ok(scan.masks)
    }

    struct Scan {
        masks: Masks,
        visits: usize,
        visit_limit: usize,
    }

    impl Scan {
        /// One directory: `path` on disk, `relative` its `/`-separated path in the workspace
        /// ("" for the root), `depth` how many directories down it is.
        fn directory(&mut self, path: &Path, relative: &str, depth: usize) -> Result<(), String> {
            if depth > DEPTH_LIMIT {
                return Err(format!(
                    "the tree is nested deeper than {DEPTH_LIMIT} directories"
                ));
            }
            let entries = fs::read_dir(path)
                .map_err(|error| format!("{} could not be listed: {error}", shown(relative)))?;
            for entry in entries {
                let entry = entry
                    .map_err(|error| format!("{} could not be listed: {error}", shown(relative)))?;
                self.visits += 1;
                if self.visits > self.visit_limit {
                    return Err(format!(
                        "the workspace has more than {} entries to scan",
                        self.visit_limit
                    ));
                }
                let name = entry.file_name();
                // Git's own directory, at any depth: see the module docs.
                if name.to_str() == Some(".git") {
                    continue;
                }
                let name = name.to_string_lossy().into_owned();
                let child = if relative.is_empty() {
                    name
                } else {
                    format!("{relative}/{name}")
                };
                let file_type = entry
                    .file_type()
                    .map_err(|error| format!("{child} could not be read: {error}"))?;
                let secret = planning_context::secret(&child);
                if file_type.is_symlink() {
                    // A symlink is never followed. A secret-named one cannot be masked: bwrap
                    // would mount over whatever it points at, so the run is refused instead.
                    if secret {
                        return Err(format!(
                            "{child} is a symbolic link named like a secret file, and a link cannot be masked safely; remove it or rename it"
                        ));
                    }
                    continue;
                }
                if secret {
                    let text = entry.path().into_os_string().into_string().map_err(|_| {
                        format!("{child} is not valid UTF-8 and matches a secret file name")
                    })?;
                    if file_type.is_dir() {
                        self.masks.dirs.push(text);
                    } else {
                        self.masks.files.push(text);
                    }
                    if self.masks.len() > LIMIT {
                        return Err(format!(
                            "the workspace has more than {LIMIT} secret files and directories to hide"
                        ));
                    }
                    continue;
                }
                if file_type.is_dir() {
                    self.directory(&entry.path(), &child, depth + 1)?;
                }
            }
            Ok(())
        }
    }

    fn shown(relative: &str) -> &str {
        if relative.is_empty() {
            "the Coding Workspace"
        } else {
            relative
        }
    }

    /// An empty directory on the host, made for one sandboxed run and removed when the run
    /// ends. Each secret directory is bound over with it read-only, so the sandbox sees the
    /// directory with nothing in it.
    #[derive(Debug)]
    pub(crate) struct EmptyDir(String);

    static EMPTY_DIRS: AtomicU64 = AtomicU64::new(0);

    impl EmptyDir {
        pub(crate) fn create() -> Result<Self, String> {
            let path = std::env::temp_dir().join(format!(
                "alfredo-mask-{}-{}",
                std::process::id(),
                EMPTY_DIRS.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path)
                .map_err(|error| format!("no empty directory for masks: {error}"))?;
            // Canonical, as the sandbox validator requires of every bound path.
            let canonical = path
                .canonicalize()
                .and_then(|canonical| {
                    canonical
                        .into_os_string()
                        .into_string()
                        .map_err(|_| std::io::Error::other("not UTF-8"))
                })
                .map_err(|error| {
                    let _ = fs::remove_dir(&path);
                    format!("no empty directory for masks: {error}")
                })?;
            Ok(Self(canonical))
        }

        pub(crate) fn path(&self) -> &str {
            &self.0
        }
    }

    impl Drop for EmptyDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir(&self.0);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{os::unix::ffi::OsStrExt, path::PathBuf};

        fn scratch(tag: &str) -> PathBuf {
            let root = std::env::temp_dir().join(format!(
                "alfredo-masks-{tag}-{}-{}",
                std::process::id(),
                EMPTY_DIRS.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            root.canonicalize().unwrap()
        }

        fn text(path: &Path) -> String {
            path.to_string_lossy().into_owned()
        }

        #[test]
        fn secret_files_and_directories_are_masked_and_nothing_else() {
            let root = scratch("walk");
            fs::create_dir_all(root.join(".ssh")).unwrap();
            fs::write(root.join(".ssh/id_rsa"), "k").unwrap();
            fs::create_dir_all(root.join("sub/deeper")).unwrap();
            fs::write(root.join("sub/.env"), "x").unwrap();
            fs::write(root.join("sub/deeper/token.pem"), "x").unwrap();
            fs::write(root.join("README.md"), "hi").unwrap();
            fs::write(root.join(".gitignore"), ".env\n").unwrap();
            fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
            fs::write(root.join("node_modules/pkg/.env"), "x").unwrap();
            let masks = scan(&root).unwrap();
            let mut files = masks.files.clone();
            files.sort();
            assert_eq!(
                files,
                vec![
                    text(&root.join("node_modules/pkg/.env")),
                    text(&root.join("sub/.env")),
                    text(&root.join("sub/deeper/token.pem")),
                ]
            );
            assert_eq!(masks.dirs, vec![text(&root.join(".ssh"))]);
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn git_directories_are_skipped_at_every_depth() {
            let root = scratch("git");
            fs::create_dir_all(root.join(".git/objects")).unwrap();
            fs::write(root.join(".git/config"), "[remote]\n").unwrap();
            fs::create_dir_all(root.join("nested/.git")).unwrap();
            fs::write(root.join("nested/.git/.env"), "x").unwrap();
            assert_eq!(scan(&root).unwrap(), Masks::default());
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn a_symlink_is_never_followed_and_a_secret_named_one_is_refused() {
            let root = scratch("links");
            fs::create_dir_all(root.join("outside")).unwrap();
            fs::write(root.join("outside/.env"), "x").unwrap();
            std::os::unix::fs::symlink("outside", root.join("alias")).unwrap();
            std::os::unix::fs::symlink("README.md", root.join("link.txt")).unwrap();
            assert_eq!(
                scan(&root).unwrap().files,
                vec![text(&root.join("outside/.env"))]
            );
            std::os::unix::fs::symlink("outside", root.join(".env")).unwrap();
            let error = scan(&root).unwrap_err();
            assert!(error.contains(".env is a symbolic link"), "{error}");
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn a_visit_budget_refuses_instead_of_stopping_early() {
            let root = scratch("budget");
            for index in 0..10 {
                fs::write(root.join(format!("file{index}")), "x").unwrap();
            }
            let error = scan_with(&root, 5).unwrap_err();
            assert!(error.contains("more than 5 entries"), "{error}");
            assert!(scan_with(&root, 10).is_ok());
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn a_tree_nested_past_the_depth_limit_refuses() {
            let root = scratch("depth");
            let mut deep = root.clone();
            for _ in 0..=DEPTH_LIMIT {
                deep = deep.join("d");
            }
            fs::create_dir_all(&deep).unwrap();
            let error = scan(&root).unwrap_err();
            assert!(error.contains("nested deeper"), "{error}");
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn a_secret_name_that_is_not_utf8_refuses_and_a_plain_one_is_ignored() {
            let root = scratch("utf8");
            let plain = root.join(std::ffi::OsStr::from_bytes(b"\xffplain.txt"));
            fs::write(plain, "x").unwrap();
            assert_eq!(scan(&root).unwrap(), Masks::default());
            let secret = root.join(std::ffi::OsStr::from_bytes(b".env\xff"));
            fs::write(secret, "x").unwrap();
            let error = scan(&root).unwrap_err();
            assert!(error.contains("not valid UTF-8"), "{error}");
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn an_empty_directory_is_made_for_a_run_and_removed_after_it() {
            let path = {
                let empty = EmptyDir::create().unwrap();
                let path = PathBuf::from(empty.path());
                assert!(path.is_dir());
                assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
                path
            };
            assert!(!path.exists(), "the empty directory outlived its run");
        }
    }
}

/// Run in the bubblewrap Sandbox at `bwrap`: the worker check boundary rooted at the
/// Coding Workspace, then `cd` to the cwd inside it.
fn sandboxed(root: &Path, plan: &Plan, bwrap: &str, stop: &Stop) -> Result<String, String> {
    if !Path::new(bwrap).is_file() {
        return Err(sandbox_failure(&format!(
            "bubblewrap is not installed at {bwrap}"
        )));
    }
    let root = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    let root_text = root
        .to_str()
        .ok_or("Coding Workspace path is not UTF-8")?
        .to_string();
    // Secret files and directories are masked before anything is reserved or written, so a
    // refused run leaves no placeholder behind. A refusal never offers escalation: a host
    // run has no masks.
    let masks = sandbox_masks::scan(&root).map_err(|reason| {
        format!("the sandbox cannot hide every secret file in the Coding Workspace ({reason}); nothing ran")
    })?;
    let empty_dir = if masks.dirs.is_empty() {
        None
    } else {
        Some(EmptyDir::create().map_err(|reason| format!("{reason}; nothing ran"))?)
    };
    let git = root.join(".git");
    // Held until this function returns, on every path, so the placeholder is removed after
    // the run (issue #522).
    let hold = hold_placeholder(&git).map_err(|error| {
        format!("the Coding Workspace's .git could not be reserved for the sandbox ({error}); nothing ran")
    })?;
    let gitfile = match hold {
        Some(_) => Some(git.to_string_lossy().into_owned()),
        None => git.exists().then(|| git.to_string_lossy().into_owned()),
    };
    let mut argv =
        crate::worker::sandbox_argv(&root_text, gitfile.as_deref(), &limits(plan.timeout));
    argv[0] = bwrap.to_string();
    // Read-only extras outside the workspace: a linked worktree's Git directory, and
    // the owner's offline toolchain (only the entries it needs, see `toolchain_binds`).
    let mut extras: Vec<String> = Vec::new();
    if hold.is_none() && git.is_file() {
        extras.extend(git_common_dir(&root));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let toolchain = toolchain_binds(home.as_deref());
    extras.extend(toolchain.binds.iter().cloned());
    let cargo_home = toolchain.cargo_home;
    let rustup_home = toolchain.rustup_home;
    extras.retain(|path| !Path::new(path).starts_with(&root));
    extras.dedup();
    let at = argv
        .iter()
        .position(|a| a == "--chdir")
        .unwrap_or(argv.len());
    let mut binds: Vec<String> = extras
        .iter()
        .flat_map(|p| ["--ro-bind".to_string(), p.clone(), p.clone()])
        .collect();
    // Secret masks shadow the writable workspace bind above: a file is bound over with
    // /dev/null, a secret directory with the empty read-only directory of this run.
    let mut mask_readonly: Vec<(String, String)> = Vec::new();
    for file in &masks.files {
        binds.extend(["--ro-bind".into(), "/dev/null".into(), file.clone()]);
        mask_readonly.push(("/dev/null".into(), file.clone()));
    }
    if let Some(empty) = &empty_dir {
        for dir in &masks.dirs {
            binds.extend(["--ro-bind".into(), empty.path().into(), dir.clone()]);
            mask_readonly.push((empty.path().into(), dir.clone()));
        }
    }
    argv.splice(at..at, binds);
    argv.extend([
        "/bin/sh".into(),
        "-c".into(),
        "if [ \"$3\" != - ]; then export CARGO_HOME=\"$3\" PATH=\"$3/bin:$PATH\" CARGO_NET_OFFLINE=true; fi; if [ \"$4\" != - ]; then export RUSTUP_HOME=\"$4\"; fi; cd -- \"$1\" && exec /bin/sh -c \"$2\"".into(),
        "alfredo".into(),
        plan.cwd.to_string_lossy().into_owned(),
        plan.command.clone(),
        cargo_home.clone().unwrap_or_else(|| "-".into()),
        rustup_home.clone().unwrap_or_else(|| "-".into()),
    ]);
    let readonly: Vec<(String, String)> = gitfile
        .iter()
        .chain(&extras)
        .map(|p| (p.clone(), p.clone()))
        .chain(mask_readonly)
        .collect();
    let mut request = request(
        argv,
        root_text.clone(),
        plan,
        ExecutionSandbox {
            mode: "bubblewrap".into(),
            readable_roots: vec![root_text.clone()],
            writable_roots: vec![root_text],
            readonly_bindings: readonly,
        },
    );
    request.environment = BTreeMap::from([
        ("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()),
        ("HOME".into(), "/tmp".into()),
        ("CI".into(), "1".into()),
    ]);
    let mut poll = || cancel_signal(stop);
    let receipt = RustExecutionProvider::new()
        .execute_with_callbacks(
            &request,
            &mut ExecutionCallbacks {
                output: None,
                process_started: None,
                poll: Some(&mut poll),
            },
        )
        .map_err(|failure| sandbox_failure(&failure.message))?;
    if execution::sandbox_unavailable_receipt(&receipt) {
        let reason = if receipt.stderr.trim().is_empty() {
            receipt.error_message.clone()
        } else {
            receipt.stderr.clone()
        };
        return Err(sandbox_failure(&reason));
    }
    let status = match receipt.status.as_str() {
        "completed" | "failed" => ProcessOutcomeStatus::Completed,
        "timed-out" => ProcessOutcomeStatus::TimedOut,
        "output-limit" => ProcessOutcomeStatus::OutputLimit,
        "cancelled" => ProcessOutcomeStatus::Cancelled,
        _ => ProcessOutcomeStatus::Unknown,
    };
    if status == ProcessOutcomeStatus::Unknown {
        return Err(format!(
            "the sandboxed command did not finish cleanly: {}",
            receipt.error_message
        ));
    }
    report(
        plan,
        Ran {
            status,
            exit_code: receipt.exit_code,
            stdout: receipt.stdout,
            stderr: receipt.stderr,
        },
    )
}

/// The owner's offline toolchain as the sandbox sees it (issue #310).
#[derive(Debug, Default)]
struct Toolchain {
    /// Canonical `~/.cargo`, exported as `CARGO_HOME`; None when there is none.
    cargo_home: Option<String>,
    /// Canonical `~/.rustup`, exported as `RUSTUP_HOME`; None when there is none.
    rustup_home: Option<String>,
    /// Read-only binds: the entries listed in [`CARGO_ENTRIES`] and [`RUSTUP_ENTRIES`]
    /// only. Never the whole homes: `~/.cargo` holds `credentials.toml` and
    /// `config.toml` (registry tokens), which a model must not read.
    binds: Vec<String>,
}

/// Entries of `~/.cargo` an offline build needs: the rustup proxies in `bin`, the
/// registry index and cache, and git dependency checkouts.
const CARGO_ENTRIES: [&str; 3] = ["bin", "registry", "git"];
/// Entries of `~/.rustup` an offline toolchain needs: the installed toolchains and the
/// settings that name the default one.
const RUSTUP_ENTRIES: [&str; 2] = ["toolchains", "settings.toml"];

/// Build the [`Toolchain`] for `home`. Only the listed entries are bound, and only when
/// each is a real directory or file: a symlink is skipped, so it cannot point a bind at
/// a secret elsewhere, and a missing entry is skipped, so bwrap does not fail on it.
/// Everything else in those homes (credentials, config, any other file) stays outside.
fn toolchain_binds(home: Option<&Path>) -> Toolchain {
    let Some(home) = home else {
        return Toolchain::default();
    };
    let canonical_dir = |name: &str| home.join(name).canonicalize().ok().filter(|p| p.is_dir());
    let real_entry = |dir: &Path, name: &str| {
        // symlink_metadata does not follow links, so a symlink is neither dir nor file.
        let meta = std::fs::symlink_metadata(dir.join(name)).ok()?;
        (meta.is_dir() || meta.is_file())
            .then(|| dir.join(name).to_str().map(str::to_string))
            .flatten()
    };
    let mut toolchain = Toolchain::default();
    if let Some(cargo) = canonical_dir(".cargo") {
        toolchain
            .binds
            .extend(CARGO_ENTRIES.iter().filter_map(|e| real_entry(&cargo, e)));
        toolchain.cargo_home = cargo.to_str().map(str::to_string);
    }
    if let Some(rustup) = canonical_dir(".rustup") {
        toolchain
            .binds
            .extend(RUSTUP_ENTRIES.iter().filter_map(|e| real_entry(&rustup, e)));
        toolchain.rustup_home = rustup.to_str().map(str::to_string);
    }
    toolchain
}

/// The shared Git directory of a linked worktree (holds its per-worktree gitdir too), only
/// when the workspace's repository verifies as one of its own (issue #359). Nothing when it
/// does not: the sandbox then has no extra bind and git inside it cannot reach another
/// repository.
fn git_common_dir(root: &Path) -> Option<String> {
    let verified = git_read::verified_git_dir(root, git_read::Config::Hardened).ok()?;
    verified.common_dir.to_str().map(str::to_string)
}

/// Run on the host (owner-approved escalation): the owner's environment, network on.
fn on_host(plan: &Plan, stop: &Stop) -> Result<String, String> {
    let cwd = plan.cwd.to_string_lossy().into_owned();
    let mut request = request(
        vec!["/bin/sh".into(), "-c".into(), plan.command.clone()],
        cwd,
        plan,
        ExecutionSandbox {
            mode: "none".into(),
            readable_roots: Vec::new(),
            writable_roots: Vec::new(),
            readonly_bindings: Vec::new(),
        },
    );
    request.environment = std::env::vars().collect();
    // The child has no terminal (issue #304): fail git prompts instead of hanging.
    request
        .environment
        .insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    let outcome = execution::run_on_host(&request, &mut || cancel_signal(stop))
        .map_err(|error| format!("could not start the command: {}", error.failure.message))?;
    if outcome.status == ProcessOutcomeStatus::Unknown {
        return Err(format!(
            "the command did not finish cleanly: {}",
            outcome.error_message
        ));
    }
    report(
        plan,
        Ran {
            status: outcome.status,
            exit_code: outcome.exit_code,
            stdout: String::from_utf8_lossy(&outcome.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&outcome.stderr).into_owned(),
        },
    )
}

/// Bytes kept from the end of each stream; together they stay under `MAX_OUTPUT`, so
/// stderr is never pushed out by long stdout. Failures print last, so tails are kept.
const STDOUT_BUDGET: usize = 10 * 1024;
const STDERR_BUDGET: usize = 5 * 1024;

/// The last `budget` bytes of `text` (on a character boundary), with a marker when cut.
fn tail(text: &str, budget: usize) -> String {
    if text.len() <= budget {
        return text.to_string();
    }
    let mut start = text.len() - budget;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!(
        "[cut: first {start} of {} bytes omitted]\n{}",
        text.len(),
        &text[start..]
    )
}

/// Exit code first, then output; a timeout is an error (the tree was killed).
fn report(plan: &Plan, ran: Ran) -> Result<String, String> {
    let place = if plan.escalate { "host" } else { "sandboxed" };
    let output = format!(
        "stdout:\n{}\nstderr:\n{}",
        tail(ran.stdout.trim_end(), STDOUT_BUDGET),
        tail(ran.stderr.trim_end(), STDERR_BUDGET)
    );
    match ran.status {
        ProcessOutcomeStatus::TimedOut => Err(format!(
            "timed out after {}s ({place}); the command and its child processes were killed\n{output}",
            plan.timeout
        )),
        ProcessOutcomeStatus::Cancelled => Err(format!(
            "cancelled ({place}); the command and its child processes were killed\n{output}"
        )),
        status => {
            let code = ran
                .exit_code
                .map_or("killed by a signal".to_string(), |c| format!("exit code {c}"));
            let mut text = format!("{code} · {place} · cwd {}", plan.shown_cwd);
            if status == ProcessOutcomeStatus::OutputLimit {
                text.push_str(&format!(
                    "\n[stopped: output passed {OUTPUT_LIMIT} bytes; the process tree was killed]"
                ));
            }
            if !plan.escalate && ran.exit_code != Some(0) {
                text.push('\n');
                text.push_str(ESCALATE_HINT);
            }
            if plan.escalate && ran.exit_code != Some(0) && needed_terminal(&ran.stderr) {
                text.push('\n');
                text.push_str(TERMINAL_HINT);
            }
            Ok(format!("{text}\n{output}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_bubblewrap_suggests_escalation_and_runs_nothing() {
        let root =
            std::env::temp_dir().join(format!("alfredo-shell-nobwrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let plan = plan(&root, &json!({"command": "touch ran"})).unwrap();
        let error = sandboxed(&root, &plan, "/nonexistent/bwrap", &Stop::default()).unwrap_err();
        assert!(error.contains("escalate: true"), "{error}");
        assert!(error.contains("nothing ran on the host"), "{error}");
        assert!(!root.join("ran").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "alfredo-shell-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_placeholder_stays_until_the_last_hold_is_released() {
        let root = scratch("hold-last");
        let git = root.join(".git");
        let first = hold_placeholder(&git).unwrap().expect("held");
        assert_eq!(std::fs::read_to_string(&git).unwrap(), PLACEHOLDER_TEXT);
        let second = hold_placeholder(&git).unwrap().expect("held");
        drop(first);
        assert!(git.is_file(), "a concurrent run lost its placeholder");
        drop(second);
        assert!(
            std::fs::symlink_metadata(&git).is_err(),
            "placeholder left behind"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_existing_git_entry_is_never_taken_over_or_removed() {
        let root = scratch("hold-existing");
        let git = root.join(".git");
        std::fs::create_dir(&git).unwrap();
        assert!(hold_placeholder(&git).unwrap().is_none());
        assert!(git.is_dir());
        std::fs::remove_dir(&git).unwrap();
        std::fs::write(&git, "gitdir: elsewhere\n").unwrap();
        assert!(hold_placeholder(&git).unwrap().is_none());
        assert_eq!(
            std::fs::read_to_string(&git).unwrap(),
            "gitdir: elsewhere\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_file_put_in_place_of_the_placeholder_is_not_removed() {
        let root = scratch("hold-replaced");
        let git = root.join(".git");
        let hold = hold_placeholder(&git).unwrap().expect("held");
        std::fs::remove_file(&git).unwrap();
        std::fs::write(&git, "owner's own file\n").unwrap();
        drop(hold);
        assert_eq!(std::fs::read_to_string(&git).unwrap(), "owner's own file\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_placeholder_left_by_an_earlier_process_is_replaced_and_then_removed() {
        let root = scratch("hold-stale");
        let git = root.join(".git");
        std::fs::write(&git, PLACEHOLDER_TEXT).unwrap();
        let hold = hold_placeholder(&git).unwrap().expect("held");
        assert_eq!(std::fs::read_to_string(&git).unwrap(), PLACEHOLDER_TEXT);
        drop(hold);
        assert!(std::fs::symlink_metadata(&git).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_needs_are_recognised_from_every_needle() {
        for stderr in [
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
            "fatal: could not read Username for 'https://github.com'",
            "fatal: could not read Password for 'https://u@github.com'",
            "fatal: terminal prompts disabled",
            "sudo: a terminal is required to read the password",
            "sudo: no tty present",
            "could not open /dev/tty: No such device",
            "Host key verification failed.",
        ] {
            assert!(needed_terminal(stderr), "{stderr}");
        }
        for stderr in [
            "Permission denied (publickey).",
            "fatal: repository not found",
            "",
        ] {
            assert!(!needed_terminal(stderr), "{stderr}");
        }
    }
}
