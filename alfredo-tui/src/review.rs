//! Readable projection of evidence already verified by `TaskStore`. No review authority.
use crate::worker::Evidence;
use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
};

pub struct View {
    pub task: u64,
    lines: Vec<Line<'static>>,
    review_prefix: usize,
    layout: std::cell::RefCell<Option<Layout>>,
    #[cfg(test)]
    measured_lines: std::cell::Cell<usize>,
}

struct Layout {
    width: u16,
    // One starting row per logical line, followed by the total row count.
    starts: Vec<usize>,
}

/// Owned visible slice with its wrapped-row offset and complete scroll boundary.
/// This is presentation data only; it conveys no evidence verification authority.
pub struct Page {
    pub lines: Vec<Line<'static>>,
    pub row: u16,
    pub maximum: usize,
}

fn safe(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

fn heading(text: impl Into<String>) -> Line<'static> {
    Line::styled(
        text.into(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

const RECORDED: &str = "; recorded at run time: ";

fn detail_line(task: u64, stored: &str, current: Option<&crate::tasks::Task>) -> String {
    match current.filter(|t| !t.can_review()) {
        Some(t) => format!(
            "Task #{task} is {}{RECORDED}{stored}",
            crate::tasks::status_label(&t.status)
        ),
        None => stored.to_owned(),
    }
}

fn hint_line(task: u64, current: Option<&crate::tasks::Task>) -> String {
    match current.filter(|t| !t.can_review()) {
        Some(t) if t.can_repair() => format!("/repair {task} reason"),
        Some(t) => format!(
            "No review commands apply; the task is {}.",
            crate::tasks::status_label(&t.status)
        ),
        None => {
            format!("/review {task} JSON · /accept {task} · /reject {task} · /repair {task} reason")
        }
    }
}

impl View {
    pub fn lines(&self) -> &[Line<'static>] {
        &self.lines
    }

    /// Cache width-dependent geometry of immutable evidence. A page clones at
    /// most one screen of logical lines, rather than the retained history.
    pub fn page(&self, width: u16, height: u16, offset: usize) -> Page {
        use ratatui::widgets::{Paragraph, Wrap};
        if width == 0 || height == 0 {
            return Page {
                lines: Vec::new(),
                row: 0,
                maximum: 0,
            };
        }
        let mut cache = self.layout.borrow_mut();
        if cache.as_ref().is_none_or(|layout| layout.width != width) {
            let mut starts = Vec::with_capacity(self.lines.len() + 1);
            starts.push(0);
            for line in &self.lines {
                let rows = if line.width() <= usize::from(width) {
                    1
                } else {
                    Paragraph::new(line.clone())
                        .wrap(Wrap { trim: false })
                        .line_count(width)
                        .max(1)
                };
                starts.push(starts.last().copied().unwrap_or(0) + rows);
            }
            #[cfg(test)]
            self.measured_lines
                .set(self.measured_lines.get() + self.lines.len());
            *cache = Some(Layout { width, starts });
        }
        let layout = cache.as_ref().expect("layout initialized");
        let maximum = layout
            .starts
            .last()
            .copied()
            .unwrap_or(0)
            .saturating_sub(usize::from(height));
        let top = offset.min(maximum);
        let first = layout
            .starts
            .partition_point(|row| *row <= top)
            .saturating_sub(1)
            .min(self.lines.len());
        let last = layout
            .starts
            .partition_point(|row| *row < top.saturating_add(usize::from(height)))
            .min(self.lines.len());
        Page {
            lines: self.lines[first..last].to_vec(),
            row: top
                .saturating_sub(layout.starts[first])
                .min(u16::MAX as usize) as u16,
            maximum,
        }
    }

    /// Re-derive the status and command lines for the task's current state, so
    /// an open view follows `/accept`, `/reject` and other status changes.
    pub fn with_current(mut self, current: &crate::tasks::Task) -> Self {
        let task = self.task;
        let Some(run) = self
            .lines
            .iter()
            .position(|line| line.to_string().starts_with("Run: "))
        else {
            return self;
        };
        let shown = self.lines[run - 1].to_string();
        let prefix = format!("Task #{task} is ");
        let stored = match shown
            .strip_prefix(&prefix)
            .and_then(|rest| rest.split_once(RECORDED))
        {
            Some((_, stored)) => stored.to_owned(),
            None => shown,
        };
        let detail = detail_line(task, &stored, Some(current));
        let mut changed = self.lines[run - 1].to_string() != detail;
        self.lines[run - 1] = Line::from(detail);
        let hint = self.lines.iter().position(|line| {
            let text = line.to_string();
            text.starts_with(&format!("/review {task} JSON"))
                || text == format!("/repair {task} reason")
                || text.starts_with("No review commands apply")
        });
        if let Some(hint) = hint {
            let text = hint_line(task, Some(current));
            changed |= self.lines[hint].to_string() != text;
            self.lines[hint] = Line::from(text);
        }
        // An unchanged view keeps its measured layout; only a rewrite invalidates it.
        if changed {
            *self.layout.get_mut() = None;
        }
        self
    }

    pub fn with_assessment(self, assessment: Option<&crate::assessment::Assessment>) -> Self {
        self.with_review(
            assessment
                .map(super::assessment::Assessment::summary)
                .as_deref(),
        )
    }
    pub fn with_review(mut self, summary: Option<&str>) -> Self {
        *self.layout.get_mut() = None;
        let replacement: Vec<_> = summary
            .map(|text| {
                std::iter::once(heading("Recorded reviewer assessment"))
                    .chain(text.lines().map(|line| Line::from(safe(line))))
                    .collect()
            })
            .unwrap_or_default();
        let count = replacement.len();
        self.lines.splice(1..1 + self.review_prefix, replacement);
        self.review_prefix = count;
        self
    }

    pub fn with_acceptance(mut self, criteria: &[String]) -> Self {
        *self.layout.get_mut() = None;
        let mut contract = vec![heading(
            "Acceptance criteria · check results are supporting evidence",
        )];
        if criteria.is_empty() {
            contract.push(Line::from(
                "Not recorded for this task; no criteria inferred.",
            ));
        } else {
            contract.extend(
                criteria
                    .iter()
                    .enumerate()
                    .map(|(i, s)| Line::from(format!("{}. {}", i + 1, safe(s)))),
            );
        }
        self.lines
            .splice(1 + self.review_prefix..1 + self.review_prefix, contract);
        self
    }

    pub fn with_inputs(mut self, inputs: &[crate::tasks::DependencyInput]) -> Self {
        if !inputs.is_empty() {
            *self.layout.get_mut() = None;
            self.lines.insert(
                1 + self.review_prefix,
                Line::from(format!(
                    "Accepted dependency inputs: {}",
                    inputs
                        .iter()
                        .map(|input| input
                            .source_task
                            .map(|source| format!("#{} via repair #{source}", input.task))
                            .unwrap_or_else(|| format!("#{}", input.task)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
        }
        self
    }

    /// Call only after the store validates the run binding and evidence digest.
    pub fn from_verified(task: u64, raw: &str) -> Result<Self, String> {
        Self::build(task, raw, None)
    }

    /// Like `from_verified`, but the status line and command hints follow the
    /// task's current state rather than the review prompt stored at run time.
    pub fn from_verified_for(task: &crate::tasks::Task, raw: &str) -> Result<Self, String> {
        Self::build(task.id, raw, Some(task))
    }

    fn build(task: u64, raw: &str, current: Option<&crate::tasks::Task>) -> Result<Self, String> {
        let evidence: Evidence =
            serde_json::from_str(raw).map_err(|_| "Cannot display malformed run evidence")?;
        let mut lines = vec![
            heading(format!(
                "Task #{task} · worker result {:?}",
                evidence.status
            )),
            Line::from(detail_line(task, &safe(&evidence.detail), current)),
            Line::from(format!("Run: {}", safe(&evidence.run))),
            Line::from(format!("Baseline: {}", safe(&evidence.baseline))),
            Line::from(hint_line(task, current)),
            Line::from("Accepted changes remain in the isolated worktree."),
            Line::default(),
            heading("Check result"),
        ];
        lines.insert(
            4,
            Line::from(
                evidence
                    .generation
                    .as_ref()
                    .map(super::provider::Generation::summary)
                    .unwrap_or_else(|| "Requested generation: unrecorded".into()),
            ),
        );
        lines.insert(
            4,
            Line::from(safe(&evidence.agent.as_ref().map_or_else(
                || "Local Agent conversation unrecorded".into(),
                super::agent::Record::summary,
            ))),
        );
        if let Some(metrics) = &evidence.model_metrics {
            lines.insert(4, Line::from(metrics.summary()));
        }
        if let Some(candidate) = &evidence.candidate_commit {
            lines.insert(4, Line::from(format!("Candidate: {}", safe(candidate))));
        }
        if let Some(check) = &evidence.check {
            let exit = check
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unavailable".into());
            lines.push(Line::from(format!(
                "{} · exit {exit} · provider {}",
                safe(&check.status),
                safe(&check.provider)
            )));
            if check.reconciliation_required {
                lines.push(Line::styled(
                    "Outcome unknown · reconciliation required",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            if !check.error_message.is_empty() {
                lines.push(Line::from(safe(&check.error_message)));
            }
            lines.push(Line::from(format!("Receipt: {}", safe(&check.receipt_id))));
        } else {
            lines.push(Line::from(
                "No check receipt retained; no check success established.",
            ));
        }
        lines.push(Line::default());
        lines.push(heading("Changes · unified diff"));
        if evidence.patch.is_empty() {
            lines.push(Line::from("No patch captured."));
        } else {
            for line in safe(&evidence.patch).lines() {
                let color = if line.starts_with("diff --git ")
                    || line.starts_with("+++ ")
                    || line.starts_with("--- ")
                {
                    Color::Cyan
                } else if line.starts_with("@@") {
                    Color::Yellow
                } else if line.starts_with('+') {
                    Color::Green
                } else if line.starts_with('-') {
                    Color::Red
                } else {
                    Color::White
                };
                lines.push(Line::styled(line.to_owned(), Style::default().fg(color)));
            }
        }
        if let Some(check) = &evidence.check {
            for (label, output) in [
                ("Check stdout", &check.stdout),
                ("Check stderr", &check.stderr),
            ] {
                lines.push(Line::default());
                lines.push(heading(label));
                if output.is_empty() {
                    lines.push(Line::from("(empty)"));
                } else {
                    lines.extend(safe(output).lines().map(|line| Line::from(line.to_owned())));
                }
            }
        }
        Ok(Self {
            task,
            lines,
            review_prefix: 0,
            layout: Default::default(),
            #[cfg(test)]
            measured_lines: Default::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_evidence_pages_do_not_remeasure_history_and_resize_invalidates() {
        let view = View {
            task: 1,
            review_prefix: 0,
            lines: (0..66_000)
                .map(|_| Line::from("界 wide text".repeat(10)))
                .collect(),
            layout: Default::default(),
            measured_lines: Default::default(),
        };
        let page = view.page(30, 12, usize::MAX);
        assert!(page.lines.len() <= 12);
        assert_eq!(view.measured_lines.get(), 66_000);
        for offset in [0, 10, 50_000, usize::MAX] {
            let page = view.page(30, 12, offset);
            assert!(page.lines.len() <= 12);
        }
        assert_eq!(view.measured_lines.get(), 66_000);
        view.page(60, 12, 0);
        assert_eq!(view.measured_lines.get(), 132_000);
        view.page(60, 24, 0);
        assert_eq!(view.measured_lines.get(), 132_000);
        let view = view.with_inputs(&[crate::tasks::DependencyInput {
            task: 2,
            source_task: None,
            run: "test".into(),
            candidate: "a".repeat(40),
            evidence_sha256: "b".repeat(64),
        }]);
        assert!(view.layout.borrow().is_none());
        view.page(60, 24, 0);
        assert_eq!(view.measured_lines.get(), 198_001);
    }

    fn task(status: crate::tasks::TaskStatus) -> crate::tasks::Task {
        serde_json::from_value(serde_json::json!({
            "id": 7, "title": "t", "model": "m", "dependencies": [], "status": status,
        }))
        .unwrap()
    }

    #[test]
    fn refresh_keeps_the_measured_layout_unless_a_line_changed() {
        use crate::tasks::TaskStatus;
        let raw = serde_json::to_string(&Evidence {
            agent: None,
            candidate_commit: None,
            model_metrics: None,
            failure_code: None,
            generation: None,
            project_notes: None,
            run: "r".into(),
            baseline: "b".into(),
            status: TaskStatus::ReviewReady,
            detail: "stored".into(),
            patch: String::new(),
            check: None,
        })
        .unwrap();
        let view = View::from_verified_for(&task(TaskStatus::Accepted), &raw).unwrap();
        view.page(40, 10, 0);
        let measured = view.measured_lines.get();
        assert!(measured > 0);
        let view = view.with_current(&task(TaskStatus::Accepted));
        view.page(40, 10, 0);
        assert_eq!(
            view.measured_lines.get(),
            measured,
            "unchanged view re-measured"
        );
        let view = view.with_current(&task(TaskStatus::Failed));
        view.page(40, 10, 0);
        assert!(
            view.measured_lines.get() > measured,
            "changed view kept stale layout"
        );
    }
}
