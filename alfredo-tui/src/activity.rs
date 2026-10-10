//! Read-only projection of acknowledged task receipts; no inferred actor or timestamp.
use crate::tasks::{Action, Snapshot};

#[derive(Debug)]
pub struct Entry {
    pub revision: u64,
    pub task: u64,
    pub correlation: String,
    pub summary: String,
    pub detail: String,
}

pub fn entries(snapshot: &Snapshot, query: &str) -> Vec<Entry> {
    iter_entries(snapshot, query).collect()
}

/// Projects newest matching receipts on demand. Exact task filters reject
/// unrelated receipts before formatting; text search still inspects their text.
pub fn iter_entries<'a>(snapshot: &'a Snapshot, query: &str) -> impl Iterator<Item = Entry> + 'a {
    let exact_task = query
        .strip_prefix('#')
        .and_then(|id| id.parse::<u64>().ok());
    let query = query.to_lowercase();
    snapshot
        .receipts
        .iter()
        .rev()
        .filter_map(move |receipt| {
            if exact_task.is_some_and(|id| id != receipt.task && !matches!(receipt.request.action, Action::ResolveRepair { task } if snapshot.resolved_by(id) == Some(task)) && !matches!(&receipt.request.action, Action::ReviewAndRepair { task, .. } | Action::ReviewArchitecture { task, .. } if *task == id) && !matches!(&receipt.request.action, Action::Plan { plan } if id >= receipt.task && id < receipt.task + plan.tasks.len() as u64)) {
                return None;
            }
            let (summary, detail) = match &receipt.request.action {
                Action::ReviewArchitecture { task, decision } => (if receipt.task == *task { format!("Architect revision required · #{task}") } else { format!("Architecture review · repair #{} proposed", receipt.task) }, decision.summary()),
                Action::ResolveRepair { task } => ("Repair resolution recorded".into(), format!("Accepted repair #{task} supplies its unsuccessful ancestors for future dependencies")),
                Action::Assign { model, .. } => ("Worker assigned".into(), format!("Local Agent model {model} · fresh approval required")),
                Action::Plan { plan } => (format!("Plan proposed · {} tasks{}", plan.tasks.len(), plan.scope.as_ref().map(|scope| format!(" · scope revision {}", scope.revision)).unwrap_or_default()), format!("Frontier Architect {} · {} · tasks require approval{}", plan.planner, plan.prompt, plan.context.as_ref().map(|context| format!(" · committed context {} · {} sources", context.baseline, context.sources.len())).unwrap_or_default())),
                Action::Branch { name, commit, .. } => (
                    "Review branch recorded".into(),
                    format!("{name} · commit {commit}"),
                ),
                Action::Propose {
                    title,
                    model,
                    dependencies,
                } => (
                    "Task proposed".into(),
                    format!("{title} · model {model} · depends on {dependencies:?}"),
                ),
                Action::ReviewAndRepair { task, decision } => (
                    format!("Review #{task}: {} · repair #{} proposed", decision.outcome.label(), receipt.task),
                    format!("{}\nRepair #{} inherits policy; fresh approval required", decision.summary(), receipt.task),
                ),
                Action::Repair { task, reason } => (
                    "Repair proposed · approval required".into(),
                    format!("Parent #{task} · {reason}"),
                ),
                Action::Permit { policy, .. } => (
                    "Policy set · approval required".into(),
                    format!("Files {:?} · check {:?}", policy.files, policy.check),
                ),
                Action::Approve { .. } => (
                    "Task approved".into(),
                    "Approval does not mean execution has started".into(),
                ),
                Action::Cancel { .. } => (
                    "Task cancelled".into(),
                    "Unstarted task cancellation acknowledged".into(),
                ),
                Action::Requeue { .. } => (
                    "Task requeued".into(),
                    "Cancelled run returns to approved; a new run starts when dispatched".into(),
                ),
                Action::Start {
                    baseline, inputs, ..
                } => (
                    "Worker run claimed".into(),
                    format!(
                        "Committed baseline {baseline} · {} accepted dependency inputs",
                        inputs.len()
                    ),
                ),
                Action::Finish {
                    run,
                    status,
                    evidence_sha256,
                    detail,
                    ..
                } => (
                    format!("Worker result: {status:?}"),
                    format!("{run} · {detail} · evidence SHA-256 {evidence_sha256}"),
                ),
                Action::Decide { decision, .. } => (
                    decision.risk.map_or_else(|| format!("Review: {}", decision.outcome.label()), |risk| format!("Human review required: {}", risk.label())),
                    decision.summary(),
                ),
                Action::Assess { assessment, .. } => (
                    if assessment.accept { "Review accepted with criterion assessment".into() } else { "Review rejected with criterion assessment".into() },
                    assessment.summary(),
                ),
                Action::Review { accept, .. } => (
                    if *accept {
                        "Review accepted".into()
                    } else {
                        "Review rejected".into()
                    },
                    "Decision recorded; changes remain in isolated worktree".into(),
                ),
            };
            let title = snapshot
                .tasks
                .iter()
                .find(|task| task.id == receipt.task)
                .map(|task| task.title.as_str())
                .unwrap_or("");
            if exact_task.is_none()
                && !format!(
                    "{} {} {} {} {}",
                    receipt.task, title, summary, detail, receipt.request.correlation
                )
                .to_lowercase()
                .contains(&query)
            {
                return None;
            }
            Some(Entry {
                revision: receipt.revision,
                task: receipt.task,
                correlation: receipt.request.correlation.clone(),
                summary,
                detail,
            })
        })
}

/// Cached F4 projection. Entries are rebuilt only when the snapshot revision,
/// receipt tail or query changes; wrapped row geometry only when the width
/// changes; and each frame turns only the visible window into styled lines.
#[derive(Default)]
pub struct View {
    key: Option<(String, std::path::PathBuf, u64, usize, String, String)>,
    entries: Vec<Entry>,
    width: u16,
    // One starting row per entry, followed by the total row count.
    starts: Vec<usize>,
    builds: usize,
}

/// Owned visible slice with the wrapped-row offset into it and the scroll maximum.
pub struct Window {
    pub lines: Vec<ratatui::text::Line<'static>>,
    pub row: u16,
    pub maximum: usize,
}

fn entry_lines(entry: &Entry) -> Vec<ratatui::text::Line<'static>> {
    use crate::dashboard::safe;
    use ratatui::{
        style::{Color, Style},
        text::Line,
    };
    let mut lines = vec![Line::styled(
        format!(
            "r{} · task #{} · {}",
            entry.revision,
            entry.task,
            safe(&entry.summary)
        ),
        Style::default().fg(Color::Cyan),
    )];
    lines.extend(
        safe(&entry.detail)
            .lines()
            .map(|line| Line::from(line.to_owned())),
    );
    lines.push(Line::from(format!("Receipt: {}", safe(&entry.correlation))));
    lines.push(Line::default());
    lines
}

fn empty_lines() -> Vec<ratatui::text::Line<'static>> {
    vec![ratatui::text::Line::from(
        "No saved task activity matches this query",
    )]
}

fn rows(line: &ratatui::text::Line<'static>, width: u16) -> usize {
    use ratatui::widgets::{Paragraph, Wrap};
    if line.width() <= usize::from(width) {
        1
    } else {
        Paragraph::new(line.clone())
            .wrap(Wrap { trim: false })
            .line_count(width)
            .max(1)
    }
}

impl View {
    /// How many times the entry list was projected from the snapshot.
    pub fn builds(&self) -> usize {
        self.builds
    }

    /// Visible window for a viewport; `offset` is clamped like the other
    /// scrolling panels.
    pub fn window(
        &mut self,
        snapshot: &Snapshot,
        query: &str,
        width: u16,
        height: u16,
        offset: usize,
    ) -> Window {
        if width == 0 || height == 0 {
            return Window {
                lines: Vec::new(),
                row: 0,
                maximum: 0,
            };
        }
        let key = (
            snapshot.mission.clone(),
            snapshot.workspace.clone(),
            snapshot.revision,
            snapshot.receipts.len(),
            snapshot
                .receipts
                .last()
                .map(|receipt| receipt.request.correlation.clone())
                .unwrap_or_default(),
            query.to_owned(),
        );
        if self.key.as_ref() != Some(&key) {
            self.entries = entries(snapshot, query);
            self.builds += 1;
            self.key = Some(key);
            self.width = 0;
        }
        if self.width != width {
            self.starts.clear();
            self.starts.push(0);
            if self.entries.is_empty() {
                self.starts.push(1);
            }
            for entry in &self.entries {
                let height = entry_lines(entry)
                    .iter()
                    .map(|line| rows(line, width))
                    .sum::<usize>();
                self.starts
                    .push(self.starts.last().copied().unwrap_or(0) + height);
            }
            self.width = width;
        }
        let total = self.starts.last().copied().unwrap_or(0);
        let maximum = total.saturating_sub(usize::from(height));
        let top = offset.min(maximum);
        if self.entries.is_empty() {
            return Window {
                lines: empty_lines(),
                row: top.min(u16::MAX as usize) as u16,
                maximum,
            };
        }
        let first = self
            .starts
            .partition_point(|start| *start <= top)
            .saturating_sub(1)
            .min(self.entries.len() - 1);
        let last = self
            .starts
            .partition_point(|start| *start < top.saturating_add(usize::from(height)))
            .min(self.entries.len());
        Window {
            lines: self.entries[first..last.max(first + 1)]
                .iter()
                .flat_map(entry_lines)
                .collect(),
            row: (top - self.starts[first]).min(u16::MAX as usize) as u16,
            maximum,
        }
    }
}
