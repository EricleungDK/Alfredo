//! Agent view: the transcript of one agent (a task's repair lineage or the
//! architect), newest at the bottom. `project` is pure: turns come from the
//! retained Local Agent conversation, saved check evidence, live worker state and
//! owner notes. `gather` reads verified files; rendering lives in `ui`. Nothing
//! here records state or authorizes work, and nothing is invented when a record
//! is missing.
use crate::tasks::TaskStatus;

/// Whose transcript the right pane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    /// A task family, by its repair root.
    Task(u64),
    Architect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Dim,
    /// `▸ path` headings of answer code.
    Path,
    Pass,
    Fail,
    Warn,
    /// Dim, cut to one row (the instruction's files/check line).
    Summary,
    /// Check output: one row per line, cut rather than wrapped.
    Output,
}

/// One speaker's contribution: a dim label and its lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub label: String,
    pub lines: Vec<(String, Tone)>,
}

/// Who wrote an attempt's instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    You,
    Autopilot,
}

/// Saved check outcome of an attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub passed: bool,
    pub exit: Option<i32>,
    /// Bounded, sanitized output tail.
    pub tail: String,
}

/// What verified evidence and the retained conversation hold for one run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    pub prompt: Option<String>,
    pub answer: Option<String>,
    pub check: Option<Check>,
    pub detail: String,
    /// One line on why the retained conversation is not shown.
    pub note: Option<String>,
    /// The retained answer is what streamed before the run was cut short.
    pub cut: Option<crate::agent::Cut>,
}

/// Live, process-local observation of a running worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Live {
    /// Short stage word (`generating`, `check`, `queued`, …).
    pub stage: String,
    pub prompt: Option<String>,
    /// Streamed model text (bounded tail).
    pub output: String,
    pub check_output: String,
    pub checking: bool,
    pub cancelling: bool,
}

/// One attempt of a task family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub id: u64,
    pub title: String,
    pub status: TaskStatus,
    pub repair_of: Option<u64>,
    pub origin: Origin,
    pub files: Vec<String>,
    pub check: Vec<String>,
    /// None until the attempt has recorded evidence; Err when it is unreadable.
    pub recorded: Option<Result<Recorded, String>>,
    pub live: Option<Live>,
}

/// An owner instruction given to an attempt, with its current effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub task: u64,
    pub text: String,
    pub status: String,
}

fn worker_label(attempt: &Attempt) -> String {
    let speaker = match attempt.origin {
        Origin::You => "You",
        Origin::Autopilot => "Autopilot",
    };
    match attempt.repair_of {
        Some(_) => format!("{speaker} → repair #{}", attempt.id),
        None => format!("{speaker} → worker #{}", attempt.id),
    }
}

/// Names of read-only references in a worker request.
pub fn references(prompt: &str) -> Vec<String> {
    prompt
        .lines()
        .filter_map(|line| {
            line.strip_prefix("READ-ONLY FILE ")
                .or_else(|| line.strip_prefix("READ-ONLY REFERENCE OMITTED "))
        })
        .map(|name| crate::dashboard::single_line(name.trim()))
        .filter(|name| !name.is_empty())
        .collect()
}

fn code(text: &str) -> Vec<(String, Tone)> {
    crate::worker::display_output(&crate::dashboard::safe(text))
        .lines()
        .map(|line| match line.strip_prefix("▸ ") {
            Some(_) => (line.to_owned(), Tone::Path),
            None => (line.to_owned(), Tone::Normal),
        })
        .collect()
}

fn plain(text: &str, tone: Tone) -> Vec<(String, Tone)> {
    crate::dashboard::safe(text)
        .lines()
        .map(|line| (line.trim_end().to_owned(), tone))
        .collect()
}

/// A failure detail without the compact output tail the Check turn already shows.
fn short_detail(detail: &str) -> String {
    let mut detail = crate::dashboard::single_line(detail);
    for marker in [": stderr: ", ": stdout: ", " · stderr: ", " · stdout: "] {
        if let Some(index) = detail.find(marker) {
            detail.truncate(index);
        }
    }
    crate::dashboard::truncate(detail.trim_end_matches(':').trim(), 200)
}

fn outcome(status: &TaskStatus) -> Option<(&'static str, Tone)> {
    Some(match status {
        TaskStatus::Accepted => ("✓ Accepted", Tone::Pass),
        TaskStatus::ReviewReady => ("◐ Check passed · awaiting review", Tone::Pass),
        TaskStatus::Failed => ("✗ Failed", Tone::Fail),
        TaskStatus::Rejected => ("✗ Rejected", Tone::Fail),
        TaskStatus::Cancelled => ("– Cancelled", Tone::Warn),
        TaskStatus::NeedsHumanReview => ("‖ Held for human review", Tone::Warn),
        TaskStatus::Proposed | TaskStatus::Approved | TaskStatus::Running => return None,
    })
}

/// Transcript turns of a task family, oldest attempt first. Owner notes follow
/// the attempt they were given to. `expanded` shows full instruction text.
pub fn project(attempts: &[Attempt], notes: &[Note], expanded: bool) -> Vec<Turn> {
    let mut turns = Vec::new();
    for attempt in attempts {
        let recorded = attempt.recorded.as_ref().and_then(|r| r.as_ref().ok());
        let prompt = attempt
            .live
            .as_ref()
            .and_then(|live| live.prompt.as_deref())
            .or_else(|| recorded.and_then(|r| r.prompt.as_deref()));
        // A repair made from an owner note shown just above names it, not repeats it.
        let from_note = attempt.repair_of.is_some_and(|parent| {
            let prefix = format!("Repair #{parent}: {}", crate::instruct::OWNER);
            attempt.title.strip_prefix(&prefix).is_some_and(|rest| {
                notes
                    .iter()
                    .any(|note| note.task == parent && rest.starts_with(note.text.as_str()))
            })
        });
        let title = match attempt.repair_of {
            Some(parent) if from_note => format!("Repair #{parent} with your note"),
            _ => crate::dashboard::single_line(&attempt.title),
        };
        let mut instruction = vec![(title, Tone::Normal)];
        if !attempt.files.is_empty() || !attempt.check.is_empty() {
            instruction.push((
                format!(
                    "files {} · check {}",
                    attempt.files.join(", "),
                    attempt.check.join(" ")
                ),
                Tone::Summary,
            ));
        }
        if expanded {
            match prompt {
                Some(prompt) => {
                    instruction.push((String::new(), Tone::Normal));
                    instruction.extend(plain(prompt, Tone::Dim));
                }
                None => instruction.push(("Request text not retained".into(), Tone::Dim)),
            }
        }
        turns.push(Turn {
            label: worker_label(attempt),
            lines: instruction,
        });
        if let Some(names) = prompt.map(references).filter(|names| !names.is_empty()) {
            turns.push(Turn {
                label: "References".into(),
                lines: vec![(names.join(", "), Tone::Normal)],
            });
        }
        let command = || (attempt.check.join(" "), Tone::Dim);
        if let Some(live) = &attempt.live {
            let mut lines = code(&live.output);
            if lines.is_empty() && !live.checking {
                let mut status = format!("… {}", live.stage);
                if live.cancelling {
                    status.push_str(" · cancelling");
                }
                lines.push((status, Tone::Warn));
            }
            if !lines.is_empty() {
                turns.push(Turn {
                    label: "Worker".into(),
                    lines,
                });
            }
            if live.checking {
                let mut lines = vec![command()];
                lines.extend(plain(&bounded(&live.check_output), Tone::Output));
                lines.push((
                    if live.cancelling {
                        "… running · cancelling"
                    } else {
                        "… running"
                    }
                    .into(),
                    Tone::Warn,
                ));
                turns.push(Turn {
                    label: "Check".into(),
                    lines,
                });
            }
        } else if let Some(recorded) = recorded {
            if let Some(answer) = &recorded.answer {
                let mut lines = code(answer);
                if let (false, Some(cut)) = (lines.is_empty(), &recorded.cut) {
                    lines.push((
                        match cut.elapsed_secs {
                            Some(secs) => format!("— steered at {secs}s · output cut"),
                            None => "— steered · output cut".into(),
                        },
                        Tone::Dim,
                    ));
                }
                if !lines.is_empty() {
                    turns.push(Turn {
                        label: "Worker".into(),
                        lines,
                    });
                }
            }
            if let Some(check) = &recorded.check {
                let mut lines = vec![command()];
                lines.extend(plain(&check.tail, Tone::Output));
                let exit = check
                    .exit
                    .map_or_else(|| "exit unknown".into(), |code| format!("exit {code}"));
                lines.push(if check.passed {
                    (format!("✓ passed · {exit}"), Tone::Pass)
                } else {
                    (format!("✗ failed · {exit}"), Tone::Fail)
                });
                turns.push(Turn {
                    label: "Check".into(),
                    lines,
                });
            }
        }
        if attempt.live.is_none() {
            if let Some((word, tone)) = outcome(&attempt.status) {
                let mut lines = Vec::new();
                match &attempt.recorded {
                    Some(Ok(recorded)) => {
                        let detail = short_detail(&recorded.detail);
                        let show = tone == Tone::Fail || attempt.status == TaskStatus::Cancelled;
                        lines.push(if show && !detail.is_empty() {
                            (format!("{word} · {detail}"), tone)
                        } else {
                            (word.into(), tone)
                        });
                        if let Some(note) = &recorded.note {
                            lines.push((crate::dashboard::single_line(note), Tone::Dim));
                        }
                    }
                    Some(Err(reason)) => {
                        lines.push((word.into(), tone));
                        lines.push((
                            format!(
                                "Evidence unavailable: {}",
                                crate::dashboard::single_line(reason)
                            ),
                            Tone::Dim,
                        ));
                    }
                    None => lines.push((word.into(), tone)),
                }
                turns.push(Turn {
                    label: "Outcome".into(),
                    lines,
                });
            }
        }
        for note in notes.iter().filter(|note| note.task == attempt.id) {
            turns.push(note_turn(note));
        }
    }
    turns
}

fn note_turn(note: &Note) -> Turn {
    let mut lines = plain(&note.text, Tone::Normal);
    if !note.status.is_empty() {
        lines.push((crate::dashboard::single_line(&note.status), Tone::Dim));
    }
    Turn {
        label: "You".into(),
        lines,
    }
}

/// Last 40 lines of live check output.
fn bounded(text: &str) -> String {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    lines[lines.len().saturating_sub(40)..].join("\n")
}

/// `Agent · worker #2 · running`, `Agent · repair #3 of #2 · failed`.
pub fn title(attempts: &[Attempt]) -> String {
    let Some(head) = attempts.last() else {
        return "Agent".into();
    };
    let state = crate::dashboard::state_word(&crate::tasks::Task {
        id: head.id,
        title: String::new(),
        model: String::new(),
        dependencies: vec![],
        status: head.status.clone(),
        policy: None,
        run: None,
        repair_of: head.repair_of,
    });
    match head.repair_of {
        Some(parent) => format!("Agent · repair #{} of #{parent} · {state}", head.id),
        None => format!("Agent · worker #{} · {state}", head.id),
    }
}

/// Cached verified record of one attempt.
pub type Record = std::sync::Arc<Result<Recorded, String>>;

/// Right-pane state of an open agent view. View state only; never persisted.
#[derive(Debug)]
pub struct View {
    pub target: Target,
    /// Instruction turns show their full request text.
    pub expanded: bool,
    pub(crate) offset: std::cell::Cell<u16>,
    pub(crate) reading: std::cell::Cell<crate::reading::Viewport>,
    /// The chat draft set aside while this view owns the prompt.
    pub chat_draft: String,
    /// Right pane to restore on Esc.
    pub previous: Previous,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Previous {
    pub tasks_visible: bool,
    pub planner_visible: bool,
}

impl View {
    pub fn new(target: Target, previous: Previous) -> Self {
        Self {
            target,
            expanded: false,
            offset: Default::default(),
            reading: Default::default(),
            chat_draft: String::new(),
            previous,
        }
    }
    /// Move the reading position like the chat transcript.
    pub fn scroll_rows(&self, rows: i32) {
        let mut reading = self.reading.get();
        reading.move_rows(rows);
        self.reading.set(reading);
    }
    pub fn position(
        &self,
        heights: &[usize],
        height: u16,
        blocks: &[crate::reading::Block],
    ) -> crate::reading::Position {
        let mut reading = self.reading.get();
        let position = reading.position_blocks(&self.offset, heights, height, blocks);
        self.reading.set(reading);
        position
    }
}

/// Prompt title while an agent owns the prompt.
pub fn prompt_title(tasks: &crate::task_control::TaskControl, target: Target) -> String {
    let name = match target {
        Target::Architect => "architect".to_string(),
        Target::Task(root) => match tasks
            .snapshot
            .as_ref()
            .and_then(|snapshot| crate::instruct::head(snapshot, root))
        {
            Some(head) if head.repair_of.is_some() => format!("repair #{}", head.id),
            Some(head) => format!("worker #{}", head.id),
            None => format!("worker #{root}"),
        },
    };
    format!(" To {name} · Enter send · Esc back ")
}

fn origin(
    snapshot: &crate::tasks::Snapshot,
    task: &crate::tasks::Task,
    tasks: &crate::task_control::TaskControl,
) -> Origin {
    if tasks
        .owner
        .instructions()
        .iter()
        .any(|item| item.child == Some(task.id))
        || crate::instruct::owner_note(snapshot, task.id).is_some()
    {
        return Origin::You;
    }
    let automatic = snapshot.receipts.iter().any(|receipt| {
        receipt.task == task.id
            && matches!(&receipt.request.action,
                crate::tasks::Action::Repair { reason, .. } if reason.starts_with("autopilot:"))
    });
    if automatic || (task.repair_of.is_none() && tasks.autopilot_roots.contains(&task.id)) {
        Origin::Autopilot
    } else {
        Origin::You
    }
}

/// Attempts of the family rooted at `root`, reading verified records once per
/// evidence hash.
pub fn gather(tasks: &crate::task_control::TaskControl, root: u64) -> Vec<Attempt> {
    let Some(snapshot) = &tasks.snapshot else {
        return vec![];
    };
    let mut family: Vec<&crate::tasks::Task> = snapshot
        .tasks
        .iter()
        .filter(|task| snapshot.repair_root(task.id) == Some(root))
        .collect();
    family.sort_by_key(|task| task.id);
    family
        .into_iter()
        .map(|task| Attempt {
            id: task.id,
            title: task.title.clone(),
            status: task.status.clone(),
            repair_of: task.repair_of,
            origin: origin(snapshot, task, tasks),
            files: task
                .policy
                .as_ref()
                .map(|policy| policy.files.clone())
                .unwrap_or_default(),
            check: task
                .policy
                .as_ref()
                .map(|policy| policy.check.clone())
                .unwrap_or_default(),
            recorded: task
                .run
                .as_ref()
                .and_then(|run| run.evidence_sha256.clone())
                .map(|hash| (*tasks.agent_record(task.id, &hash)).clone()),
            live: tasks.agent_live(task.id),
        })
        .collect()
}

/// Read and verify one attempt's evidence and retained conversation.
pub fn read_record(store: &crate::tasks::TaskStore, task: u64) -> Result<Recorded, String> {
    let raw = store.evidence(task)?;
    let evidence: crate::worker::Evidence =
        serde_json::from_str(&raw).map_err(|_| "Malformed run evidence".to_string())?;
    let check = evidence.check.as_ref().map(|check| Check {
        passed: crate::worker::check_passed(check),
        exit: check.exit_code,
        tail: crate::worker::output_tail(check)
            .map(|(_, tail)| tail)
            .unwrap_or_default(),
    });
    let (prompt, answer, note) = match crate::agent::retained_exchange(store, &evidence) {
        Ok((prompt, answer)) => (Some(prompt), Some(answer), None),
        Err(reason) => (None, None, Some(reason)),
    };
    Ok(Recorded {
        prompt,
        answer,
        check,
        detail: evidence.detail,
        note,
        cut: None,
    })
}

/// Architect transcript: the planning request, the streamed or saved draft and
/// owner notes.
pub fn project_architect(
    request: Option<&str>,
    origin: Origin,
    draft: &str,
    notes: &[Note],
    expanded: bool,
) -> Vec<Turn> {
    let mut turns = Vec::new();
    if let Some(request) = request.filter(|text| !text.trim().is_empty()) {
        let speaker = match origin {
            Origin::You => "You → architect",
            Origin::Autopilot => "Autopilot → architect",
        };
        let text = crate::dashboard::single_line(request);
        let lines = if expanded {
            plain(&text, Tone::Normal)
        } else {
            vec![(crate::dashboard::truncate(&text, 160), Tone::Normal)]
        };
        turns.push(Turn {
            label: speaker.into(),
            lines,
        });
    }
    let lines = plain(draft, Tone::Normal);
    if !lines.is_empty() {
        turns.push(Turn {
            label: "Architect".into(),
            lines,
        });
    }
    turns.extend(notes.iter().map(note_turn));
    turns
}

/// Open the agent view for `target`: it takes the right pane and the prompt.
/// The chat draft is set aside and the target's own unsent draft restored.
pub fn open(
    app: &mut crate::model::App,
    tasks: &mut crate::task_control::TaskControl,
    target: Target,
) {
    let session = &mut app.sessions[app.selected];
    let (chat_draft, previous) = match tasks.agent.take() {
        Some(view) => {
            tasks
                .agent_drafts
                .insert(view.target, session.draft.clone());
            (view.chat_draft, view.previous)
        }
        None => (
            session.draft.clone(),
            Previous {
                tasks_visible: tasks.visible,
                planner_visible: tasks.planner.visible,
            },
        ),
    };
    session.clear_draft();
    session.insert(&tasks.agent_drafts.remove(&target).unwrap_or_default());
    let mut view = View::new(target, previous);
    view.chat_draft = chat_draft;
    app.models_visible = false;
    tasks.set_visible(true);
    tasks.planner.visible = false;
    tasks.evidence = None;
    tasks.activity = None;
    tasks.autopilot_report = None;
    tasks.scope_view = None;
    tasks.agent = Some(view);
}

/// Leave the agent view, keeping its unsent draft for next time and restoring
/// the chat draft. `restore` also returns the right pane to what it showed
/// before; otherwise the view that replaced it stays.
pub fn close(
    app: &mut crate::model::App,
    tasks: &mut crate::task_control::TaskControl,
    restore: bool,
) {
    let Some(view) = tasks.agent.take() else {
        return;
    };
    let session = &mut app.sessions[app.selected];
    if session.draft.is_empty() {
        tasks.agent_drafts.remove(&view.target);
    } else {
        tasks
            .agent_drafts
            .insert(view.target, session.draft.clone());
    }
    session.clear_draft();
    session.insert(&view.chat_draft);
    if restore {
        tasks.set_visible(view.previous.tasks_visible);
        tasks.planner.visible = view.previous.planner_visible;
    }
}

fn parse_target(tasks: &crate::task_control::TaskControl, word: &str) -> Result<Target, String> {
    if word == "architect" {
        return Ok(Target::Architect);
    }
    let id: u64 = word
        .trim_start_matches('#')
        .parse()
        .map_err(|_| "Use a task number or architect".to_string())?;
    tasks
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.repair_root(id))
        .map(Target::Task)
        .ok_or(format!("Task #{id} not found"))
}

/// `/watch ID|architect` opens an agent view; `/tell ID|architect TEXT` sends it
/// an instruction from anywhere. None for other input.
pub fn console(
    app: &mut crate::model::App,
    tasks: &mut crate::task_control::TaskControl,
    text: &str,
) -> Option<Result<String, String>> {
    let text = text.trim();
    let (verb, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let rest = rest.trim();
    match verb {
        "/watch" => Some((|| {
            let target = parse_target(tasks, rest)?;
            if target == Target::Architect
                && !tasks.planner.active()
                && tasks.planner.checkpoint().is_none()
            {
                return Err("No plan is being drafted; /plan REQUEST starts one".into());
            }
            open(app, tasks, target);
            Ok(match target {
                Target::Architect => "Watching the architect".into(),
                Target::Task(_) => format!(
                    "Watching {}",
                    prompt_title(tasks, target)
                        .trim()
                        .trim_start_matches("To ")
                        .split(" · ")
                        .next()
                        .unwrap_or_default()
                ),
            })
        })()),
        "/tell" => Some((|| {
            let (word, note) = rest
                .split_once(char::is_whitespace)
                .ok_or("Usage: /tell ID TEXT")?;
            let target = parse_target(tasks, word)?;
            crate::instruct::Instructions::give_to(tasks, target, note)
        })()),
        _ => None,
    }
}
