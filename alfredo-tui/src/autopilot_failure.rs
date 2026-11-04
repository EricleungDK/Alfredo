use super::{clean, Family, Saved, Settled};
use crate::{
    task_control::TaskControl,
    tasks::{Snapshot, Task, TaskStatus},
};

/// Planner, worker and repair usage of this run's own tasks: the saved plan's
/// receipt plus the model metrics in each task run's evidence. Anything the
/// server did not report adds nothing.
pub(super) fn run_usage(
    tasks: &TaskControl,
    snapshot: &Snapshot,
    saved: &Saved,
) -> crate::metrics::Usage {
    let mut total = crate::metrics::Usage::default();
    let plan = saved.save_request.as_ref().and_then(|request| {
        snapshot
            .receipts
            .iter()
            .find(|receipt| &receipt.request.correlation == request)
    });
    if let Some(crate::tasks::Action::Plan { plan }) = plan.map(|receipt| &receipt.request.action) {
        if let Some(usage) = &plan.usage {
            total.add(usage);
        }
    }
    let Some(first) = saved.first else {
        return total;
    };
    let roots = saved.roots();
    for task in snapshot.tasks.iter().filter(|task| task.id >= first) {
        let in_run = snapshot
            .repair_root(task.id)
            .is_some_and(|root| roots.contains(&root));
        if !in_run || task.run.is_none() {
            continue;
        }
        let usage = tasks
            .store()
            .evidence(task.id)
            .ok()
            .and_then(|text| serde_json::from_str::<crate::worker::Evidence>(&text).ok())
            .and_then(|evidence| evidence.model_metrics)
            .and_then(|metrics| crate::metrics::Usage::from_metrics(&metrics));
        if let Some(usage) = usage {
            total.add(&usage);
        }
    }
    total
}

/// Root cause when the head failed because the sandbox could not start. The
/// run's evidence carries the typed `failure_code`; repairing cannot fix it.
pub fn sandbox_failure(tasks: &TaskControl, head: &Task) -> Option<String> {
    if head.status != TaskStatus::Failed {
        return None;
    }
    let evidence = tasks.store().evidence(head.id).ok()?;
    let evidence = serde_json::from_str::<crate::worker::Evidence>(&evidence).ok()?;
    (evidence.failure_code.as_deref() == Some(crate::execution::SANDBOX_UNAVAILABLE))
        .then(|| clean(&evidence.detail, 300))
}

/// Failure text when the head's approved files exceed the worker's bounds. The
/// run's evidence carries the typed `failure_code`; repairing cannot fix it.
pub fn too_large_failure(tasks: &TaskControl, head: &Task) -> Option<String> {
    if head.status != TaskStatus::Failed {
        return None;
    }
    let evidence = tasks.store().evidence(head.id).ok()?;
    let evidence = serde_json::from_str::<crate::worker::Evidence>(&evidence).ok()?;
    (evidence.failure_code.as_deref() == Some(crate::worker::FILE_TOO_LARGE))
        .then(|| clean(&evidence.detail, 300))
}

/// Failure text when the head's model request never reached the server. The run's
/// evidence carries the typed `failure_code`; repairing the code cannot fix it.
pub fn unreachable_failure(tasks: &TaskControl, head: &Task) -> Option<String> {
    if head.status != TaskStatus::Failed {
        return None;
    }
    let evidence = tasks.store().evidence(head.id).ok()?;
    let evidence = serde_json::from_str::<crate::worker::Evidence>(&evidence).ok()?;
    (evidence.failure_code.as_deref() == Some(crate::worker::MODEL_UNREACHABLE))
        .then(|| clean(&evidence.detail, 300))
}

pub const INTERRUPTED: &str = "run interrupted";

/// Why an interrupted run cannot be recovered automatically.
pub fn interrupted_reason(tasks: &TaskControl, head: u64) -> String {
    match tasks.run_observations.get(&head) {
        Some(text) if text.contains("another process") => {
            format!("{INTERRUPTED}: its worker is still active in another Alfredo process")
        }
        Some(text) => format!(
            "{INTERRUPTED}: it cannot be recovered safely ({})",
            clean(text, 200)
        ),
        None => format!("{INTERRUPTED}: its worker state is unknown"),
    }
}

pub(super) fn held_or_done(family: &Family, tasks: &TaskControl) -> bool {
    matches!(family.settled, Settled::Success(_)) || tasks.owner.held().contains(&family.root)
}

pub const BUDGET_EXHAUSTED: &str = "repair budget exhausted";

/// One line of a recorded failure for the summary, shortened on a character
/// boundary. A failed check keeps its `Check failed (exit N): stderr:` head and
/// ends with the output's key error line (as `worker::failing_lines` picks it,
/// else the last output line); the head is cut first so the error stays visible.
pub fn last_failure_line(detail: &str) -> String {
    const LIMIT: usize = 160;
    const KEY_LIMIT: usize = 120;
    let first = detail
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no detail recorded");
    let first = clean(first, 1780);
    if let Some((head, key)) = check_head_and_key(&first) {
        let key = shorten(&clean(&key, 400), KEY_LIMIT);
        let room = LIMIT.saturating_sub(key.chars().count() + 1);
        return format!("{} {key}", shorten(head.trim_end(), room));
    }
    shorten(&first, LIMIT)
}

/// The summary line of an autopilot repair's title (`Repair #N: autopilot: <failure>`), as
/// `Last failure` shows it, so a title cut for width keeps the check's key error line.
pub fn title_failure_line(title: &str) -> Option<String> {
    let (_, detail) = title.split_once(": autopilot: ")?;
    Some(last_failure_line(detail))
}

/// Split a `Check ...: <label>: a | b | c` summary into its head and the most
/// specific output line.
fn check_head_and_key(summary: &str) -> Option<(&str, String)> {
    let check = summary.find("Check ")?;
    let end = [": stderr: ", ": stdout: ", " · stderr: ", " · stdout: "]
        .iter()
        .filter_map(|label| {
            summary[check..]
                .find(label)
                .map(|at| (at, check + at + label.len()))
        })
        .min()?
        .1;
    let (head, body) = summary.split_at(end);
    let output = body
        .trim_start_matches('…')
        .split(" | ")
        .collect::<Vec<_>>()
        .join("\n");
    let picked = crate::worker::failing_lines(&output);
    let last = |text: &str, skip_note: bool| {
        text.lines()
            .rev()
            .map(str::trim)
            .find(|line| {
                !(line.is_empty() || skip_note && line.ends_with("more failing lines omitted)"))
            })
            .map(str::to_string)
    };
    let key = last(&picked, true).or_else(|| last(&output, false))?;
    Some((head, key))
}

fn shorten(line: &str, limit: usize) -> String {
    if line.chars().count() > limit {
        let cut: String = line.chars().take(limit.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

/// Repair reason detail: a failed check's bounded output tail from verified
/// evidence (naming a no-progress attempt), else the recorded run detail. Never empty.
pub fn failure_detail(tasks: &TaskControl, head: &Task) -> String {
    let failure = tasks
        .store()
        .evidence(head.id)
        .ok()
        .and_then(|raw| serde_json::from_str::<crate::worker::Evidence>(&raw).ok())
        .and_then(|evidence| crate::worker::check_failure(&evidence, 1780));
    if let Some(failure) = failure {
        return failure;
    }
    head.run
        .as_ref()
        .map(|run| clean(&run.detail, 1780))
        .filter(|detail| !detail.trim_end_matches(':').trim().is_empty())
        .unwrap_or_else(|| "worker failed; no detail recorded".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskStore;

    fn control(name: &str, held_root: Option<u64>) -> TaskControl {
        let root = std::env::temp_dir().join(format!("alfredo-held-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("workspace")).unwrap();
        let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "test").unwrap();
        let mut tasks = TaskControl::new(store);
        if let Some(held) = held_root {
            let item = serde_json::json!({
                "id": 1, "root": held, "task": held, "note": "n", "effect": "repair",
                "status": "queued", "done": false,
            });
            let state = serde_json::json!({"version": 1, "next": 2, "instructions": [item]});
            std::fs::write(
                crate::instruct::state_path(&root, "c"),
                serde_json::to_vec(&state).unwrap(),
            )
            .unwrap();
            tasks.owner = crate::instruct::Instructions::open(&root, "c").unwrap();
        }
        tasks
    }

    fn family(root: u64, settled: Settled) -> Family {
        Family {
            root,
            head: root,
            settled,
            repairs: 0,
        }
    }

    #[test]
    fn a_family_is_held_or_done_when_it_succeeded_or_the_owner_holds_it() {
        let free = control("free", None);
        let held = control("held", Some(4));
        assert!(held_or_done(&family(4, Settled::Active), &held));
        assert!(!held_or_done(&family(5, Settled::Active), &held));
        assert!(held_or_done(&family(4, Settled::Success(9)), &free));
        assert!(!held_or_done(&family(4, Settled::Active), &free));
        assert!(!held_or_done(&family(4, Settled::Stuck("x".into())), &free));
        assert!(!held_or_done(&family(4, Settled::Cancelled), &free));
    }
}
