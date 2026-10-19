//! `/clean`: remove the retained worktrees of finished runs, and only those.
//!
//! Every run checks out the whole repository into `<state>/rust-tasks-v1/<id>/runs/<run>/worktree`
//! and registers it in the repository's own `.git/worktrees`. Nothing a review needs lives
//! there: the saved evidence (`evidence.json`, with the diff and the check receipt) sits beside
//! it in the run directory, and the candidate commit is held by `refs/alfredo/candidates/<sha>`.
//! A worktree is therefore removable once its run has a finish receipt, its evidence still
//! matches that receipt, and, when the run saved a candidate, the candidate ref still
//! resolves. A failed or cancelled run has no candidate by design; its worktree is scratch
//! and its diff is in the evidence. Everything else is kept, and the reason is shown.
use crate::tasks::{Action, Snapshot, TaskStatus, TaskStore};
use crate::worker::{git, Evidence};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub struct Entry {
    pub task: u64,
    pub run: String,
    pub path: PathBuf,
    pub bytes: u64,
    /// accepted, rejected, failed or cancelled
    pub outcome: String,
}
#[derive(Debug)]
pub struct Kept {
    pub task: u64,
    pub run: String,
    pub reason: String,
}
#[derive(Default)]
pub struct Plan {
    pub remove: Vec<Entry>,
    pub keep: Vec<Kept>,
}
impl Plan {
    pub fn bytes(&self) -> u64 {
        self.remove.iter().map(|entry| entry.bytes).sum()
    }
}
#[derive(Default)]
pub struct Outcome {
    pub removed: Vec<Entry>,
    pub failed: Vec<(String, String)>,
}
impl Outcome {
    pub fn bytes(&self) -> u64 {
        self.removed.iter().map(|entry| entry.bytes).sum()
    }
}

struct Finish {
    status: TaskStatus,
    digest: String,
}

pub fn size_label(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in UNITS {
        if value < 1000.0 {
            break;
        }
        value /= 1000.0;
        unit = next;
    }
    format!("{value:.1} {unit}")
}

/// Bytes under `path`, never following symlinks.
fn tree_bytes(path: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(meta) = fs::symlink_metadata(&current) else {
            continue;
        };
        total += meta.len();
        if meta.is_dir() {
            if let Ok(entries) = fs::read_dir(&current) {
                pending.extend(entries.flatten().map(|entry| entry.path()));
            }
        }
    }
    total
}

fn finish_receipts(snapshot: &Snapshot) -> BTreeMap<String, Finish> {
    let mut finishes = BTreeMap::new();
    for receipt in &snapshot.receipts {
        if let Action::Finish {
            run,
            status,
            evidence_sha256,
            ..
        } = &receipt.request.action
        {
            finishes.insert(
                run.clone(),
                Finish {
                    status: status.clone(),
                    digest: evidence_sha256.clone(),
                },
            );
        }
    }
    finishes
}

fn read_evidence(directory: &Path, run: &str, digest: &str) -> Result<Evidence, String> {
    let path = directory.join("evidence.json");
    let meta = fs::symlink_metadata(&path).map_err(|_| "evidence file is missing".to_string())?;
    if !meta.is_file() {
        return Err("evidence is not a regular file".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|file| file.take(1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|_| "evidence cannot be read".to_string())?;
    if bytes.len() > 1024 * 1024 || format!("{:x}", Sha256::digest(&bytes)) != digest {
        return Err("evidence does not match its receipt".into());
    }
    let evidence: Evidence =
        serde_json::from_slice(&bytes).map_err(|_| "evidence is malformed".to_string())?;
    if evidence.run != run {
        return Err("evidence belongs to another run".into());
    }
    Ok(evidence)
}

/// Registered worktree paths of the repository, canonicalised where they exist.
async fn registered(workspace: &Path) -> Result<Vec<PathBuf>, String> {
    let listing = git(workspace, &["worktree", "list", "--porcelain"]).await?;
    Ok(listing
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect())
}

fn outcome_label(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Accepted => "accepted",
        TaskStatus::Rejected => "rejected",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
        _ => "unfinished",
    }
}

/// Decide, per run that still has a worktree directory, whether it may be removed.
pub async fn plan(store: &TaskStore) -> Result<Plan, String> {
    let snapshot = store.snapshot()?;
    let workspace = snapshot.workspace.clone();
    let finishes = finish_receipts(&snapshot);
    let registered = registered(&workspace).await?;
    // Every run the store knows: each task's current run plus every finished one.
    let mut runs: BTreeMap<String, u64> = BTreeMap::new();
    for receipt in &snapshot.receipts {
        if let Action::Finish { run, task, .. } = &receipt.request.action {
            runs.insert(run.clone(), *task);
        }
    }
    for task in &snapshot.tasks {
        if let Some(run) = &task.run {
            runs.insert(run.id.clone(), task.id);
        }
    }
    let mut plan = Plan::default();
    for (run, task_id) in runs {
        let Ok(directory) = store.run_directory(&run) else {
            continue;
        };
        let worktree = directory.join("worktree");
        let Ok(meta) = fs::symlink_metadata(&worktree) else {
            continue;
        };
        let mut keep = |reason: &str| {
            plan.keep.push(Kept {
                task: task_id,
                run: run.clone(),
                reason: reason.into(),
            })
        };
        let Some(task) = snapshot.tasks.iter().find(|task| task.id == task_id) else {
            keep("task is unknown");
            continue;
        };
        let Some(finish) = finishes.get(&run) else {
            keep("run is not finished");
            continue;
        };
        let current = task.run.as_ref().is_some_and(|bound| bound.id == run);
        let outcome = if current {
            match task.status {
                TaskStatus::Accepted
                | TaskStatus::Rejected
                | TaskStatus::Failed
                | TaskStatus::Cancelled => outcome_label(&task.status),
                TaskStatus::ReviewReady | TaskStatus::NeedsHumanReview => {
                    keep("awaiting review");
                    continue;
                }
                _ => {
                    keep("task is not finished");
                    continue;
                }
            }
        } else if matches!(finish.status, TaskStatus::Failed | TaskStatus::Cancelled) {
            outcome_label(&finish.status)
        } else {
            keep("superseded run kept for its review result");
            continue;
        };
        if !meta.is_dir() {
            keep("worktree is not a plain directory");
            continue;
        }
        let evidence = match read_evidence(&directory, &run, &finish.digest) {
            Ok(evidence) => evidence,
            Err(reason) => {
                keep(&reason);
                continue;
            }
        };
        match &evidence.candidate_commit {
            Some(commit) => {
                let name = format!("refs/alfredo/candidates/{commit}");
                let valid = commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit());
                let resolved = if valid {
                    git(
                        &workspace,
                        &[
                            "rev-parse",
                            "--verify",
                            "--quiet",
                            &format!("{name}^{{commit}}"),
                        ],
                    )
                    .await
                    .ok()
                } else {
                    None
                };
                if resolved.as_deref().map(str::trim) != Some(commit.as_str()) {
                    keep("candidate ref is missing");
                    continue;
                }
            }
            None => {
                // Failed and cancelled runs save no candidate; their diff is in the evidence.
                if matches!(
                    evidence.status,
                    TaskStatus::ReviewReady | TaskStatus::Accepted
                ) || outcome == "accepted"
                {
                    keep("candidate commit was never saved");
                    continue;
                }
            }
        }
        let canonical = fs::canonicalize(&worktree).ok();
        let inside = canonical.as_ref().is_some_and(|path| {
            fs::canonicalize(&directory).is_ok_and(|dir| *path == dir.join("worktree"))
        });
        if !inside || !canonical.as_ref().is_some_and(|p| registered.contains(p)) {
            keep("not a registered Alfredo worktree");
            continue;
        }
        plan.remove.push(Entry {
            task: task_id,
            run,
            path: worktree.clone(),
            bytes: tree_bytes(&worktree),
            outcome: outcome.into(),
        });
    }
    Ok(plan)
}

/// Remove the planned worktrees with Git, then prune stale registrations.
pub async fn clean(store: &TaskStore) -> Result<Outcome, String> {
    let workspace = store.snapshot()?.workspace;
    let plan = plan(store).await?;
    let mut outcome = Outcome::default();
    for entry in plan.remove {
        let path = entry.path.to_str().ok_or("Non-UTF-8 worktree path")?;
        match git(&workspace, &["worktree", "remove", "--force", path]).await {
            Ok(_) => outcome.removed.push(entry),
            Err(error) => outcome.failed.push((entry.run, error)),
        }
    }
    git(&workspace, &["worktree", "prune"]).await?;
    Ok(outcome)
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn report(plan: &Plan, done: Option<&Outcome>) -> String {
    let mut lines = vec!["Clean finished runs' worktrees".to_owned(), String::new()];
    match done {
        Some(done) => {
            lines.push(format!(
                "Removed {} worktree{} · freed {}",
                done.removed.len(),
                plural(done.removed.len()),
                size_label(done.bytes())
            ));
            for entry in &done.removed {
                lines.push(format!(
                    "  #{} {} {} · {}",
                    entry.task,
                    entry.run,
                    entry.outcome,
                    size_label(entry.bytes)
                ));
            }
            for (run, error) in &done.failed {
                lines.push(format!("  kept {run}: {error}"));
            }
        }
        None => {
            lines.push(format!(
                "Would remove {} worktree{} · {}",
                plan.remove.len(),
                plural(plan.remove.len()),
                size_label(plan.bytes())
            ));
            for entry in &plan.remove {
                lines.push(format!(
                    "  #{} {} {} · {}",
                    entry.task,
                    entry.run,
                    entry.outcome,
                    size_label(entry.bytes)
                ));
            }
        }
    }
    if !plan.keep.is_empty() {
        lines.push(String::new());
        lines.push(format!("Kept {}", plan.keep.len()));
        for kept in &plan.keep {
            lines.push(format!("  #{} {} · {}", kept.task, kept.run, kept.reason));
        }
    }
    lines.push(String::new());
    lines.push(
        "Evidence (F3), refs/alfredo/* candidates, /branch and alfredo/go-* branches stay.".into(),
    );
    if done.is_none() && !plan.remove.is_empty() {
        lines.push("/clean yes removes the listed worktrees.".into());
    }
    lines.join("\n")
}

/// Terminal entry for `/clean` and `/clean yes`.
pub fn command(
    runtime: &tokio::runtime::Runtime,
    tasks: &mut crate::task_control::TaskControl,
    autopilot_running: bool,
    argument: &str,
) -> Result<String, String> {
    if autopilot_running {
        return Err("Pause autopilot (/pause or F5) before /clean".into());
    }
    if !tasks.workers.is_empty() || tasks.dispatch.enabled {
        return Err("Finish or cancel coding workers (and /dispatch off) before /clean".into());
    }
    let confirm = match argument.trim() {
        "" => false,
        "yes" => true,
        _ => return Err("Usage: /clean (list) or /clean yes (remove)".into()),
    };
    let store = tasks.store().clone();
    let plan = runtime.block_on(plan(&store))?;
    if plan.remove.is_empty() && !confirm {
        tasks.set_visible(true);
        tasks.planner.visible = false;
        tasks.autopilot_report = Some(report(&plan, None));
        return Ok("Nothing to clean: no finished run has a removable worktree".into());
    }
    let (text, notice) = if confirm {
        let done = runtime.block_on(clean(&store))?;
        let notice = format!(
            "Removed {} worktree{} · freed {}{}",
            done.removed.len(),
            plural(done.removed.len()),
            size_label(done.bytes()),
            if done.failed.is_empty() {
                String::new()
            } else {
                format!(" · {} kept after an error", done.failed.len())
            }
        );
        (report(&plan, Some(&done)), notice)
    } else {
        (
            report(&plan, None),
            format!(
                "{} worktree{} ({}) removable · /clean yes removes them",
                plan.remove.len(),
                plural(plan.remove.len()),
                size_label(plan.bytes())
            ),
        )
    };
    tasks.set_visible(true);
    tasks.planner.visible = false;
    tasks.autopilot_report = Some(text);
    Ok(notice)
}
