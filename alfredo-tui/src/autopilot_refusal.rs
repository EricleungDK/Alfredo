use super::{clean, text, Saved, MAX_STATE};
use crate::{task_control::TaskControl, understanding};
use std::path::Path;

/// Autopilot approves without a human reading each policy, so it also refuses
/// plans a reader would reject: checks that can only fail and unordered writers.
pub(super) fn unattended_lint(plan: &crate::planner::Plan) -> Result<(), String> {
    let findings = crate::plan_lint::findings(plan);
    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings.join("; "))
    }
}

/// Start of every [`lint_refusal`].
pub(super) const LINT_REFUSAL_PREFIX: &str = "The plan cannot run unattended and nothing changed:";

/// What follows the findings in every [`lint_refusal`].
pub(super) const LINT_REFUSAL_FIX: &str =
    "Fix it with /plan-revise (draft) or run it by hand with /approve";

/// Reviewing a plan is the approval of its files and checks, but autopilot
/// still refuses what it would refuse from the planner.
pub(super) fn lint_refusal(plan: &crate::planner::Plan) -> Result<(), String> {
    unattended_lint(plan).map_err(|error| {
        // Every finding ends with a period of its own (#650).
        format!(
            "{LINT_REFUSAL_PREFIX} {}. {LINT_REFUSAL_FIX}",
            clean(&error, 600).trim_end_matches('.')
        )
    })
}

/// How the interactive TUI shows a [`lint_refusal`] that the one-line footer would cut
/// before its fix: the footer names the fix, and the chat rows carry the whole sentence
/// (#650). `None` for any other message.
pub fn lint_refusal_view(message: &str, command: &str) -> Option<(String, Vec<String>)> {
    if !message.starts_with(LINT_REFUSAL_PREFIX) || !message.ends_with(LINT_REFUSAL_FIX) {
        return None;
    }
    Some((
        format!("{command} refused: the plan fails the unattended lint (see the chat rows above)"),
        vec![format!("✗ Not started · {message}")],
    ))
}

/// What opens the refusal of a `/go` while a run is active; the goal follows.
pub(super) const ACTIVE_REFUSAL_PREFIX: &str = "Autopilot is active for “";

/// What follows the goal in that refusal.
pub(super) const ACTIVE_REFUSAL_FIX: &str =
    "”; /pause or /stop and let workers finish before a new /go";

/// How the interactive TUI shows the refusal of a `/go` while a run is active, which the
/// one-line footer would cut before its fix: the footer says why, and the chat rows carry
/// the whole sentence (#672). `None` for any other message.
#[must_use]
pub fn active_refusal_view(message: &str, command: &str) -> Option<(String, Vec<String>)> {
    if !message.starts_with(ACTIVE_REFUSAL_PREFIX) || !message.ends_with(ACTIVE_REFUSAL_FIX) {
        return None;
    }
    Some((
        format!("{command} refused: autopilot is active (see the chat rows above)"),
        vec![format!("✗ Not started · {message}")],
    ))
}

/// The goal autopilot shows for a plan it adopts: the `/plan` prompt.
pub(super) fn plan_goal(plan: &crate::planner::Plan) -> Result<String, String> {
    let goal = clean(plan.prompt.trim(), 4096);
    if text(&goal, 4096) {
        Ok(goal)
    } else {
        Err("The plan has no usable prompt to show as the goal".into())
    }
}

pub(super) fn draft_matches(tasks: &TaskControl, request: &str) -> bool {
    tasks.planner.checkpoint().is_some_and(|draft| {
        draft
            .origin
            .as_ref()
            .is_some_and(|origin| origin.correlation == request)
            && draft.validate().is_ok()
    })
}

pub(super) fn goal_brief(goal: &str) -> understanding::Brief {
    understanding::Brief {
        destination: format!("Autopilot goal: {}", clean(goal, 1900)),
        scope: "Only the tasks autopilot plans for this goal, each limited to its approved files and check.".into(),
        constraints: "Local isolated worktrees and approved file/check policies; no push; the checked-out branch, index and working files stay untouched.".into(),
        uncertainty: "Plan and code are model-generated; autopilot acceptance relies on the approved check passing, not independent inspection.".into(),
    }
}

/// A user-authored pending draft is never confirmed on the user's behalf. Only
/// an untouched Wayfinder entry placeholder or autopilot's own draft may proceed.
pub(super) fn scope_blocker(scope: &understanding::Snapshot, goal: &str) -> Result<(), String> {
    if scope.brief.is_none() || scope.confirmed {
        return Ok(());
    }
    let placeholder = scope.flow.is_some() && scope.draft_revision == 1;
    if placeholder || scope.brief.as_ref() == Some(&goal_brief(goal)) {
        return Ok(());
    }
    Err(format!(
        "Scope draft {} awaits your review; inspect /scope and /scope-confirm {} (or revise it) before /go",
        scope.draft_revision, scope.draft_revision
    ))
}

/// Name the state file and the ways out of a refused one; the file is never touched.
/// `io`: the file could not be read at all, so fixing its permissions comes first.
pub(super) fn unreadable(path: &Path, problem: &str, io: bool) -> String {
    let path = path.display();
    let fix = if io {
        "Fix its permissions or move it aside"
    } else {
        "Move it aside"
    };
    format!(
        "{problem} ({path}); original preserved. {fix} (mv \"{path}\" \"{path}.bad\") or start with --new-mission NAME"
    )
}

/// The refusal for a state file that exists but cannot be read.
pub(super) fn read_failure(path: &Path, error: &std::io::Error) -> String {
    unreadable(path, &format!("Cannot read autopilot state: {error}"), true)
}

/// Parse and validate saved state bytes; every refusal names `path` and the way out.
pub(super) fn parse(path: &Path, bytes: &[u8]) -> Result<Saved, String> {
    if bytes.len() > MAX_STATE {
        return Err(unreadable(
            path,
            "Autopilot state exceeds its size bound",
            false,
        ));
    }
    let saved: Saved = serde_json::from_slice(bytes).map_err(|error| {
        unreadable(
            path,
            &format!("Autopilot state is invalid ({error})"),
            false,
        )
    })?;
    saved
        .validate()
        .map_err(|problem| unreadable(path, &problem, false))?;
    Ok(saved)
}
