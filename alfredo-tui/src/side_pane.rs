//! Left side pane: missions and work rows projected from the task store, live
//! worker/planner state and chat sessions. Projection is pure; rendering lives
//! in `ui`. Nothing here records state or authorizes work.
use crate::{
    mission_work::NodeId,
    model::App,
    task_control::TaskControl,
    tasks::{Task, TaskStatus},
    theme::{Record, RowStatus, Theme},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Missions,
    Work,
}

/// Stable identity of a work row across redraws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKey {
    Architect,
    Node(NodeId),
    Chat(usize),
}

/// What Enter on a row opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Architect,
    Node(NodeId),
    Chat(usize),
    Mission(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Group,
    Record(Record),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkRow {
    pub key: RowKey,
    pub kind: RowKind,
    pub status: Option<RowStatus>,
    pub depth: usize,
    pub label: String,
    /// Right-aligned short fact: group size or agent state.
    pub right: String,
    /// Dim second line for running rows: stage, model, elapsed.
    pub second: Option<String>,
    pub expanded: bool,
    /// The row shown in the right pane.
    pub current: bool,
    pub target: OpenTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionRow {
    pub name: String,
    pub current: bool,
    pub progress: String,
    pub target: OpenTarget,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub done: usize,
    pub total: usize,
    pub repairs: usize,
    pub working: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Projection {
    pub missions: Vec<MissionRow>,
    pub work: Vec<WorkRow>,
    pub summary: Summary,
    /// Direct task matches while a filter is active.
    pub filter: Option<(String, usize, usize)>,
}

impl Projection {
    pub fn working(&self) -> bool {
        self.work
            .iter()
            .any(|row| row.status == Some(RowStatus::Working))
    }
}

/// Another mission of this workspace, refreshed off the render path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionEntry {
    pub name: String,
    pub progress: MissionProgress,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MissionProgress {
    /// No autopilot state saved.
    Idle,
    /// Saved autopilot phase word.
    State(String),
    /// State file unreadable.
    Unknown,
}

impl MissionProgress {
    pub fn label(&self) -> &str {
        match self {
            Self::Idle => "idle",
            Self::State(word) => word,
            Self::Unknown => "?",
        }
    }
}

/// View state of the pane; never persisted.
#[derive(Clone, Debug, Default)]
pub struct PaneUi {
    pub theme: Theme,
    pub focus: Option<Section>,
    /// Narrow terminals show the pane as an overlay while focused.
    pub overlay: bool,
    pub mission_cursor: usize,
    pub work_cursor: Option<RowKey>,
    pub missions: Vec<MissionEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneKey {
    Up,
    Down,
    Tab,
    Enter,
    Esc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneAction {
    None,
    /// The work cursor moved to this row.
    Moved(RowKey),
    Open(OpenTarget),
    Exit,
}

impl PaneUi {
    /// F6: focus the pane (as an overlay when narrow), or return to the prompt.
    pub fn toggle_focus(&mut self, narrow: bool) {
        if self.focus.is_some() {
            self.focus = None;
            self.overlay = false;
        } else {
            self.focus = Some(Section::Work);
            self.overlay = narrow;
            self.work_cursor = None;
        }
    }

    fn leave(&mut self) {
        self.focus = None;
        self.overlay = false;
    }

    /// The highlighted work row: the cursor if still present, else the row
    /// shown in the right pane, else the first row.
    pub fn work_index(&self, projection: &Projection) -> Option<usize> {
        self.work_cursor
            .and_then(|key| projection.work.iter().position(|row| row.key == key))
            .or_else(|| projection.work.iter().position(|row| row.current))
            .or((!projection.work.is_empty()).then_some(0))
    }

    pub fn mission_index(&self, projection: &Projection) -> Option<usize> {
        (!projection.missions.is_empty())
            .then(|| self.mission_cursor.min(projection.missions.len() - 1))
    }

    pub fn key(&mut self, key: PaneKey, projection: &Projection) -> PaneAction {
        let Some(section) = self.focus else {
            return PaneAction::None;
        };
        match key {
            PaneKey::Esc => {
                self.leave();
                PaneAction::Exit
            }
            PaneKey::Tab => {
                self.focus = Some(match section {
                    Section::Work if !projection.missions.is_empty() => Section::Missions,
                    _ => Section::Work,
                });
                PaneAction::None
            }
            PaneKey::Up | PaneKey::Down => {
                let forward = key == PaneKey::Down;
                match section {
                    Section::Missions => {
                        if let Some(index) = self.mission_index(projection) {
                            self.mission_cursor = step(index, projection.missions.len(), forward);
                        }
                        PaneAction::None
                    }
                    Section::Work => {
                        let Some(index) = self.work_index(projection) else {
                            return PaneAction::None;
                        };
                        let row = &projection.work[step(index, projection.work.len(), forward)];
                        self.work_cursor = Some(row.key);
                        PaneAction::Moved(row.key)
                    }
                }
            }
            PaneKey::Enter => {
                let target = match section {
                    Section::Missions => self
                        .mission_index(projection)
                        .map(|index| projection.missions[index].target.clone()),
                    Section::Work => self
                        .work_index(projection)
                        .map(|index| projection.work[index].target.clone()),
                };
                match target {
                    Some(target) => {
                        self.leave();
                        PaneAction::Open(target)
                    }
                    None => PaneAction::None,
                }
            }
        }
    }
}

fn step(index: usize, len: usize, forward: bool) -> usize {
    if forward {
        (index + 1).min(len.saturating_sub(1))
    } else {
        index.saturating_sub(1)
    }
}

/// Open a work row in the right pane: a task opens its agent view (the
/// transcript of its repair lineage), the architect opens its agent view, a
/// group opens its detail and a chat its transcript. Missions are handed off by
/// the terminal (they switch work).
pub fn open_work_target(app: &mut App, tasks: &mut TaskControl, target: &OpenTarget) {
    match target {
        OpenTarget::Node(NodeId::Task(id)) => {
            let root = tasks
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.repair_root(*id))
                .unwrap_or(*id);
            tasks.focus_node(NodeId::Task(*id));
            crate::agent_view::open(app, tasks, crate::agent_view::Target::Task(root));
        }
        OpenTarget::Node(node) => {
            crate::agent_view::close(app, tasks, false);
            app.models_visible = false;
            tasks.evidence = None;
            tasks.activity = None;
            tasks.planner.visible = false;
            tasks.set_visible(true);
            tasks.focus_node(*node);
        }
        OpenTarget::Architect => {
            crate::agent_view::open(app, tasks, crate::agent_view::Target::Architect);
        }
        OpenTarget::Chat(index) if *index < app.sessions.len() => {
            crate::agent_view::close(app, tasks, false);
            app.models_visible = false;
            app.selected = *index;
            tasks.set_visible(false);
        }
        OpenTarget::Chat(_) | OpenTarget::Mission(_) => {}
    }
}

/// Model name without its tag: `qwen2.5-coder:14b` → `qwen2.5-coder`.
pub fn short_model(model: &str) -> &str {
    model.split_once(':').map_or(model, |(name, _)| name)
}

/// One short word for a worker stage.
pub fn stage_word(stage: &str) -> &'static str {
    match stage {
        "Running approved check" => "check",
        "Receiving model plan" => "generating",
        "Thinking" => "thinking",
        "Waiting for shared Alfredo capacity" => "queued",
        "Waiting for model server" => "waiting",
        "Reconnecting to model server" => "reconnecting",
        "Validating model plan" => "validating",
        "Writing approved files" => "writing",
        stage if stage.starts_with("Saving") => "saving",
        _ => "preparing",
    }
}

/// `m:ss`, or `h:mm:ss` past an hour.
pub fn elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

/// Status of a task row from canonical state plus observed local workers.
pub fn task_status(
    snapshot: &crate::tasks::Snapshot,
    task: &Task,
    tasks: &TaskControl,
) -> RowStatus {
    match task.status {
        TaskStatus::Running => match tasks.worker_stage(task.id) {
            Some((stage, _, queued)) if queued || stage_word(stage) == "queued" => {
                RowStatus::Queued
            }
            Some(_) => RowStatus::Working,
            None => RowStatus::Unverified,
        },
        TaskStatus::Accepted => RowStatus::Complete,
        TaskStatus::ReviewReady => RowStatus::Review,
        TaskStatus::NeedsHumanReview => RowStatus::Decision,
        TaskStatus::Failed | TaskStatus::Rejected => RowStatus::Failed,
        TaskStatus::Cancelled => RowStatus::Cancelled,
        TaskStatus::Proposed | TaskStatus::Approved => {
            if crate::dashboard::glyph(snapshot, task).0 == "‖" {
                RowStatus::Blocked
            } else {
                RowStatus::Pending
            }
        }
    }
}

fn chat_status(session: &crate::model::Session) -> RowStatus {
    use crate::model::Status;
    match &session.status {
        Status::Connecting | Status::Streaming if session.short_status() == "queued" => {
            RowStatus::Queued
        }
        Status::Connecting | Status::Streaming => RowStatus::Working,
        Status::Failed(_) => RowStatus::Failed,
        _ => RowStatus::Idle,
    }
}

/// Cheap check for the event loop: does any row show a working spinner?
pub fn any_working(app: &App, tasks: Option<&TaskControl>) -> bool {
    app.sessions
        .iter()
        .any(|session| chat_status(session) == RowStatus::Working)
        || tasks.is_some_and(|tasks| {
            tasks.planner.active()
                || tasks.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.tasks.iter().any(|task| {
                        task.status == TaskStatus::Running
                            && task_status(snapshot, task, tasks) == RowStatus::Working
                    })
                })
        })
}

fn second_line(stage: &str, model: &str, elapsed_time: Duration) -> String {
    format!("{stage}  {model}  {}", elapsed(elapsed_time))
}

pub fn project(app: &App, tasks: Option<&TaskControl>, now: Instant) -> Projection {
    let mut projection = Projection::default();
    let snapshot = tasks.and_then(|tasks| tasks.snapshot.as_ref());
    if let (Some(tasks), Some(snapshot)) = (tasks, snapshot) {
        let (done, total, repairs) = crate::dashboard::done_total(snapshot);
        projection.summary.done = done;
        projection.summary.total = total;
        projection.summary.repairs = repairs;
        let progress = match &tasks.autopilot {
            Some(status) if status.total == 0 => crate::dashboard::clock(status.elapsed),
            Some(status) => format!(
                "{}/{}   {}",
                status.done,
                status.total,
                crate::dashboard::clock(status.elapsed)
            ),
            None if total > 0 => format!("{done}/{total}"),
            None => "idle".into(),
        };
        projection.missions.push(MissionRow {
            name: crate::dashboard::single_line(&snapshot.mission),
            current: true,
            progress,
            target: OpenTarget::Mission(snapshot.mission.clone()),
        });
        projection
            .missions
            .extend(app.pane.missions.iter().map(|entry| MissionRow {
                name: crate::dashboard::single_line(&entry.name),
                current: false,
                progress: entry.progress.label().into(),
                target: OpenTarget::Mission(entry.name.clone()),
            }));

        let planner = &tasks.planner;
        let drafting = planner.checkpoint().is_some();
        if planner.active() || drafting {
            let active = planner.active();
            projection.work.push(WorkRow {
                key: RowKey::Architect,
                kind: RowKind::Record(Record::Agent),
                status: Some(if active {
                    RowStatus::Working
                } else {
                    RowStatus::Decision
                }),
                depth: 0,
                label: "architect".into(),
                right: if active { "planning" } else { "draft" }.into(),
                // The state word is already on the row: model and elapsed only.
                second: planner.started().map(|started| {
                    format!(
                        "{}  {}",
                        planner.model(),
                        elapsed(now.saturating_duration_since(started))
                    )
                }),
                expanded: false,
                current: (tasks.visible && planner.visible)
                    || tasks
                        .agent_shown()
                        .is_some_and(|view| view.target == crate::agent_view::Target::Architect),
                target: OpenTarget::Architect,
            });
        }

        let tree = tasks.work_tree();
        let focused = tasks
            .visible
            .then(|| tasks.focused_work_node())
            .flatten()
            .filter(|_| {
                !planner.visible
                    && tasks
                        .agent
                        .as_ref()
                        .is_none_or(|view| view.target != crate::agent_view::Target::Architect)
                    && tasks.evidence.is_none()
                    && tasks.activity.is_none()
                    && tasks.scope_view.is_none()
            });
        for row in &tree.rows {
            let Some(id) = row.task else {
                let label = match row.id {
                    NodeId::Plan(_) => row
                        .label
                        .split_once(" · ")
                        .map_or(row.label.as_str(), |(_, goal)| goal)
                        .to_owned(),
                    _ => "Manual tasks".into(),
                };
                projection.work.push(WorkRow {
                    key: RowKey::Node(row.id),
                    kind: RowKind::Group,
                    status: None,
                    depth: row.depth,
                    label: crate::dashboard::single_line(&label),
                    right: row.task_count.to_string(),
                    second: None,
                    expanded: row.expanded,
                    current: focused == Some(row.id),
                    target: OpenTarget::Node(row.id),
                });
                continue;
            };
            let task = snapshot.tasks.iter().find(|task| task.id == id);
            let status = task.map(|task| task_status(snapshot, task, tasks));
            if status == Some(RowStatus::Working) {
                projection.summary.working += 1;
            }
            let second = task
                .filter(|_| matches!(status, Some(RowStatus::Working | RowStatus::Queued)))
                .and_then(|task| {
                    let (stage, time, queued) = tasks.worker_stage(task.id)?;
                    let word = if queued { "queued" } else { stage_word(stage) };
                    Some(second_line(word, &task.model, time))
                });
            let repair = matches!(row.parent, Some(NodeId::Task(_)));
            projection.work.push(WorkRow {
                key: RowKey::Node(row.id),
                kind: RowKind::Record(if repair { Record::Repair } else { Record::Task }),
                status,
                depth: row.depth,
                label: format!("#{id} {}", crate::dashboard::single_line(&row.label)),
                right: String::new(),
                second,
                expanded: row.expanded,
                current: focused == Some(row.id),
                target: OpenTarget::Node(row.id),
            });
        }
        if !tasks.task_query.trim().is_empty() {
            projection.filter = Some((
                crate::dashboard::single_line(&tasks.task_query),
                tree.matched_tasks,
                tree.total_tasks,
            ));
        }
    }
    let chat_current = tasks.is_none_or(|tasks| !tasks.visible) && !app.models_visible;
    for (index, session) in app.sessions.iter().enumerate() {
        let status = chat_status(session);
        projection.work.push(WorkRow {
            key: RowKey::Chat(index),
            kind: RowKind::Record(Record::Agent),
            status: Some(status),
            depth: 0,
            label: format!("chat {}", index + 1),
            right: session.short_status(),
            second: None,
            expanded: false,
            current: chat_current && index == app.selected,
            target: OpenTarget::Chat(index),
        });
    }
    projection
}

/// Other missions of this workspace with their saved autopilot phase. Reads
/// discovery records and autopilot files only: no locks, no writes.
pub fn load_missions(
    state: &Path,
    workspace: &Path,
    current: &str,
    conversation: &str,
) -> Vec<MissionEntry> {
    let Ok(discovery) = crate::missions::discover(state, workspace) else {
        return vec![];
    };
    discovery
        .names
        .into_iter()
        .filter(|name| name != current)
        .map(|name| {
            let progress = crate::tasks::TaskStore::new(state, workspace, &name)
                .map(|store| crate::autopilot::state_path(store.state_directory(), conversation))
                .and_then(|path| crate::autopilot::peek(&path));
            let progress = match progress {
                Ok(None) => MissionProgress::Idle,
                Ok(Some(word)) => MissionProgress::State(word.into()),
                Err(_) => MissionProgress::Unknown,
            };
            MissionEntry { name, progress }
        })
        .collect()
}
