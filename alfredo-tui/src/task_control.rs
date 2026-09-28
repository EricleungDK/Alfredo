//! Async terminal adapter for durable task commands. Model prose never calls it.
use crate::mission_work::{NodeId, Tree};
use crate::provider::Ollama;
use crate::tasks::{Action, Refusal, Request, Snapshot, TaskStore};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{
    runtime::Runtime,
    sync::{mpsc, watch},
};

struct Projection {
    gate: Option<crate::task_view::ScopeStatus>,
    scope: Option<crate::understanding::Snapshot>,
    canonical_scope: Option<crate::understanding::Snapshot>,
    snapshot: Snapshot,
    notice: String,
    evidence: Option<(u64, crate::review::View)>,
    run_observations: Option<BTreeMap<u64, String>>,
}
impl From<(Snapshot, String)> for Projection {
    fn from((snapshot, notice): (Snapshot, String)) -> Self {
        Self {
            snapshot,
            notice,
            evidence: None,
            scope: None,
            canonical_scope: None,
            gate: None,
            run_observations: None,
        }
    }
}
type Outcome = Result<Projection, String>;
/// A store write's outcome; a transient refusal may carry the current snapshot.
type Checked = Result<Projection, Refusal>;
type ControlResult = (
    crate::control_command::Request,
    Result<crate::understanding::Snapshot, String>,
);

#[derive(Clone, PartialEq, Eq)]
struct WorkerIdentity {
    correlation: String,
    start_revision: u64,
}

#[derive(PartialEq, Eq)]
struct WorkTreeKey {
    revision: Option<u64>,
    scope_revision: Option<u64>,
    scope_blocked: bool,
    scope_label: String,
    query: String,
    collapsed: BTreeSet<NodeId>,
}

pub struct TaskControl {
    pub scope_status: crate::task_view::ScopeStatus,
    pub scope_view: Option<crate::understanding::Snapshot>,
    pub canonical_scope: Option<crate::understanding::Snapshot>,
    scope_retry: Option<crate::understanding::Request>,
    store: TaskStore,
    sender: mpsc::Sender<Checked>,
    receiver: mpsc::Receiver<Checked>,
    background_sender: mpsc::Sender<Outcome>,
    background_receiver: mpsc::Receiver<Outcome>,
    refreshing: bool,
    pub snapshot: Option<Snapshot>,
    observed_revision: Option<u64>,
    prepared_retry: Option<crate::command_intent::Intent>,
    prepared_active: Option<crate::command_intent::Intent>,
    prepared_workers: BTreeMap<u64, crate::command_intent::Intent>,
    prepared_errors: BTreeMap<String, String>,
    /// Correlations whose refusal was transient: nothing was written.
    prepared_transient: BTreeSet<String>,
    controller_epoch: u64,
    dispatch_origin: Option<crate::control_command::Request>,
    control_sender: mpsc::UnboundedSender<ControlResult>,
    control_receiver: mpsc::UnboundedReceiver<ControlResult>,
    control_pending: Option<crate::control_command::Request>,
    control_events: Vec<crate::control_command::Event>,
    control_acknowledged: BTreeMap<String, crate::control_command::Event>,
    worker_identities: BTreeMap<u64, WorkerIdentity>,
    pub planner: crate::planner::Planner,
    architecture_launch: Option<Request>,
    pub pending: bool,
    pub writing: bool,
    pub notice: String,
    pub visible: bool,
    pub scroll: usize,
    pub(crate) scroll_max: std::cell::Cell<usize>,
    pub(crate) scroll_height: std::cell::Cell<u16>,
    selected_task: Option<u64>,
    focused_work_node: Option<NodeId>,
    collapsed_work_nodes: BTreeSet<NodeId>,
    work_tree_cache: std::cell::RefCell<Option<(WorkTreeKey, Arc<Tree>)>>,
    pending_evidence: Option<u64>,
    pub task_query: String,
    retry: Option<Request>,
    incarnation: String,
    sequence: u64,
    worker_sender: mpsc::Sender<(u64, Checked)>,
    worker_receiver: mpsc::Receiver<(u64, Checked)>,
    pub workers: BTreeMap<u64, Arc<AtomicBool>>,
    pub dispatch: crate::dispatch::Dispatch,
    progress: BTreeMap<u64, watch::Receiver<crate::worker::Progress>>,
    provider: Option<Ollama>,
    pub evidence: Option<crate::review::View>,
    pub activity: Option<String>,
    pub run_observations: BTreeMap<u64, String>,
    /// Read-only autopilot projection for rendering; the controller owns decisions.
    pub autopilot: Option<crate::autopilot::Status>,
    /// Completion/status report opened by autopilot; any task view replaces it.
    pub autopilot_report: Option<String>,
    /// Last user-driven selection move; autopilot focus waits while it is recent.
    manual_selection: Option<std::time::Instant>,
    /// Detail pane follows the live tail until the user scrolls away from it.
    pub(crate) follow_tail: std::cell::Cell<bool>,
    /// Whether the last rendered detail pane showed a live worker.
    pub(crate) detail_live: std::cell::Cell<bool>,
    /// Compact verified outcome per task, keyed by the acknowledged evidence hash.
    outcomes: std::cell::RefCell<BTreeMap<u64, (String, OutcomeLines)>>,
}

/// Rendered outcome lines, or why verified evidence could not be shown.
pub type OutcomeLines = Arc<Result<Vec<ratatui::text::Line<'static>>, String>>;

/// Autopilot focus waits this long after a manual selection move.
const MANUAL_SELECTION_HOLD: std::time::Duration = std::time::Duration::from_secs(10);

impl TaskControl {
    pub fn new(store: TaskStore) -> Self {
        let (sender, receiver) = mpsc::channel(1);
        let (background_sender, background_receiver) = mpsc::channel(1);
        let (worker_sender, worker_receiver) = mpsc::channel(8);
        let (control_sender, control_receiver) = mpsc::unbounded_channel();
        Self {
            scope_status: crate::task_view::ScopeStatus::loading(),
            scope_view: None,
            canonical_scope: None,
            scope_retry: None,
            store,
            sender,
            receiver,
            background_sender,
            background_receiver,
            refreshing: false,
            snapshot: None,
            observed_revision: None,
            prepared_retry: None,
            prepared_active: None,
            prepared_workers: BTreeMap::new(),
            prepared_errors: BTreeMap::new(),
            prepared_transient: BTreeSet::new(),
            controller_epoch: 0,
            dispatch_origin: None,
            control_sender,
            control_receiver,
            control_pending: None,
            control_events: Vec::new(),
            control_acknowledged: BTreeMap::new(),
            worker_identities: BTreeMap::new(),
            planner: Default::default(),
            architecture_launch: None,
            pending: false,
            writing: false,
            notice: "Loading task queue".into(),
            visible: false,
            scroll: 0,
            scroll_max: Default::default(),
            scroll_height: Default::default(),
            selected_task: None,
            focused_work_node: None,
            collapsed_work_nodes: BTreeSet::new(),
            work_tree_cache: Default::default(),
            pending_evidence: None,
            task_query: String::new(),
            retry: None,
            incarnation: format!(
                "{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            sequence: 0,
            worker_sender,
            worker_receiver,
            workers: BTreeMap::new(),
            dispatch: Default::default(),
            progress: BTreeMap::new(),
            provider: None,
            evidence: None,
            activity: None,
            run_observations: BTreeMap::new(),
            autopilot: None,
            autopilot_report: None,
            manual_selection: None,
            follow_tail: std::cell::Cell::new(true),
            detail_live: Default::default(),
            outcomes: Default::default(),
        }
    }

    pub fn store(&self) -> &TaskStore {
        &self.store
    }

    pub fn observe_scope(&mut self, state: crate::understanding::Snapshot) {
        if self
            .scope_status
            .revision
            .is_none_or(|revision| state.revision >= revision)
        {
            self.canonical_scope = Some(state.clone());
            self.scope_status = crate::task_view::ScopeStatus::observe(Ok(state));
            if self.scope_status.blocked {
                self.disable_dispatch();
            }
        }
    }
    /// Observe only newly admitted canonical receipts. Initial history stays in
    /// Activity; this cursor neither invents past chronology nor acknowledges writes.
    pub fn newly_observed_receipts(&mut self) -> Vec<crate::model::TaskReceiptRef> {
        let Some(snapshot) = &self.snapshot else {
            return vec![];
        };
        let Some(previous) = self.observed_revision else {
            self.observed_revision = Some(snapshot.revision);
            return vec![];
        };
        if snapshot.revision <= previous {
            return vec![];
        }
        let receipts = snapshot
            .receipts
            .iter()
            .filter(|receipt| receipt.revision > previous)
            .map(|receipt| crate::model::TaskReceiptRef {
                revision: receipt.revision,
                correlation: receipt.request.correlation.clone(),
                task: receipt.task,
                after_messages: 0,
                sequence: 0,
            })
            .collect();
        self.observed_revision = Some(snapshot.revision);
        receipts
    }

    pub fn work_status(&self) -> crate::task_view::WorkStatus {
        use crate::tasks::TaskStatus;
        let mut status = crate::task_view::WorkStatus {
            workers: self.workers.len(),
            loaded: self.snapshot.is_some(),
            ..Default::default()
        };
        let Some(snapshot) = &self.snapshot else {
            return status;
        };
        for task in &snapshot.tasks {
            if task.status == TaskStatus::Running {
                if !self.workers.contains_key(&task.id) {
                    status.recorded += 1;
                }
                continue;
            }
            if snapshot.resolution_for_family(task.id).is_some() {
                continue;
            }
            if snapshot.architecture_required(task.id) {
                status.architect += 1;
                continue;
            }
            if snapshot.architecture_obsolete(task.id) {
                continue;
            }
            if task.status == TaskStatus::NeedsHumanReview {
                status.held += 1;
                continue;
            }
            if snapshot.tasks.iter().any(|child| {
                child.repair_of == Some(task.id)
                    && !(child.status == TaskStatus::Cancelled && child.run.is_none())
            }) {
                continue;
            }
            match task.status {
                TaskStatus::Proposed if task.repair_of.is_some() => status.repair += 1,
                TaskStatus::ReviewReady => status.review += 1,
                TaskStatus::Failed | TaskStatus::Rejected | TaskStatus::Cancelled
                    if task.run.is_some() =>
                {
                    status.repair += 1
                }
                TaskStatus::Accepted if task.repair_of.is_some() => status.resolve += 1,
                _ => {}
            }
        }
        status
    }

    pub fn can_switch(&self) -> Result<(), String> {
        if self.dispatch.enabled {
            return Err("Turn /dispatch off before switching work".into());
        }
        if !self.workers.is_empty() {
            return Err("Finish or cancel coding workers before switching work".into());
        }
        if self.pending || self.writing || self.control_pending.is_some() {
            return Err("Wait for the task operation before switching work".into());
        }
        if self.retry.is_some() {
            return Err(
                "Resolve the task request with /retry-task or /refresh before switching work"
                    .into(),
            );
        }
        if self.planner.active() {
            return Err("Wait for the plan or use /plan-cancel before switching work".into());
        }
        if let Some(draft) = self.planner.checkpoint() {
            draft.validate().map_err(|error| {
                format!("Cannot preserve plan: {error}; /plan-cancel discards it")
            })?;
        }
        Ok(())
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = self.planner.poll();
        while let Ok((request, result)) = self.control_receiver.try_recv() {
            changed = true;
            if self.control_pending.as_ref() != Some(&request) {
                continue;
            }
            self.control_pending = None;
            let result = result.and_then(|state| {
                let crate::control_command::Operation::Dispatch {
                    enabled: true,
                    expected_epoch,
                    scope_revision_for_on,
                } = request.operation
                else {
                    return Err("Invalid pending dispatch operation".into());
                };
                let gate = crate::task_view::ScopeStatus::observe(Ok(state.clone()));
                if request.controller != self.incarnation || expected_epoch != self.controller_epoch
                {
                    return Err("Controller changed while dispatch enable was pending".into());
                }
                if gate.blocked
                    || Some(state.revision) != scope_revision_for_on
                    || self.scope_status.blocked
                    || self.scope_status.revision != scope_revision_for_on
                {
                    self.observe_scope(state);
                    return Err("Scope changed; review it before enabling dispatch".into());
                }
                self.observe_scope(state);
                self.controller_epoch = self
                    .controller_epoch
                    .checked_add(1)
                    .ok_or("Controller epoch exhausted; restart before enabling dispatch")?;
                self.dispatch.enabled = true;
                self.dispatch.contended = None;
                self.dispatch_origin = Some(request.clone());
                Ok(())
            });
            match result {
                Ok(()) => {
                    self.notice = "Dispatch on · all ready approved tasks (including filtered-out rows) · failed starts need explicit /run ID".into();
                    self.acknowledge_control(
                        request,
                        crate::control_command::Outcome::DispatchChanged { enabled: true },
                    );
                }
                Err(error) => {
                    self.disable_dispatch();
                    self.notice = format!("Dispatch enable refused: {error}");
                    self.prepared_errors
                        .insert(request.correlation.clone(), error);
                }
            }
        }
        if let Ok(result) = self.background_receiver.try_recv() {
            self.refreshing = false;
            changed = true;
            match result {
                Ok(mut projection) => {
                    projection.notice = self.notice.clone();
                    self.apply_outcome(Ok(projection));
                }
                Err(error) => {
                    self.disable_dispatch();
                    self.notice = format!("Task refresh failed; dispatch off: {error}");
                }
            }
        }
        while let Ok((task, result)) = self.worker_receiver.try_recv() {
            let intent = self.prepared_workers.remove(&task);
            if let Some(intent) = &intent {
                self.record_refusal(intent.correlation(), &result);
            }
            let automatic = matches!(
                intent,
                Some(crate::command_intent::Intent::DispatchRun { .. })
            );
            self.workers.remove(&task);
            self.worker_identities.remove(&task);
            self.progress.remove(&task);
            match &result {
                Err(refusal) if automatic && refusal.transient() => {
                    let reason = refusal.to_string();
                    self.apply_checked(result);
                    self.defer_launch(task, &reason);
                }
                _ => {
                    self.dispatch.transient.remove(&task);
                    if let Err(error) = &result {
                        self.dispatch.failures.insert(task, error.to_string());
                    }
                    self.apply_checked(result);
                }
            }
            changed = true;
        }
        for receiver in self.progress.values_mut() {
            if receiver.has_changed().unwrap_or(false) {
                receiver.borrow_and_update();
                changed = true;
            }
        }
        let Ok(result) = self.receiver.try_recv() else {
            return changed;
        };
        if let Some(intent) = self.prepared_active.take() {
            self.record_refusal(intent.correlation(), &result);
        }
        self.pending = false;
        self.writing = false;
        if result.is_err() {
            self.architecture_launch = None;
        }
        if result.is_ok() {
            if self.retry.as_ref().is_some_and(|request| {
                matches!(request.action, Action::Plan { .. })
                    && result.as_ref().is_ok_and(|projection| {
                        projection
                            .snapshot
                            .receipts
                            .iter()
                            .any(|receipt| receipt.request == *request)
                    })
            }) {
                self.planner.draft = None;
                self.planner.visible = false;
            }
            self.retry = None;
        }
        self.apply_checked(result);
        true
    }

    /// A stale refusal still delivers the current snapshot, so the next decision
    /// is prepared on current state instead of repeating the refused revision.
    fn apply_checked(&mut self, result: Checked) {
        match result {
            Ok(projection) => self.apply_outcome(Ok(projection)),
            Err(Refusal::Stale(Some(current))) => {
                let notice = format!(
                    "{} · current state loaded (revision {})",
                    Refusal::Stale(None),
                    current.revision
                );
                self.apply_outcome(Ok((*current, notice).into()));
            }
            Err(refusal) => self.apply_outcome(Err(refusal.into())),
        }
    }

    fn record_refusal(&mut self, correlation: &str, result: &Checked) {
        self.prepared_transient.remove(correlation);
        if let Err(refusal) = result {
            self.prepared_errors
                .insert(correlation.into(), refusal.to_string());
            if refusal.transient() {
                self.prepared_transient.insert(correlation.into());
            }
        }
    }

    /// Nothing was claimed: release this approval's start reservation so a fresh
    /// request on current state may launch it. Bounded; then dispatch stops.
    fn defer_launch(&mut self, task: u64, reason: &str) {
        use crate::dispatch::TRANSIENT_LIMIT;
        self.dispatch.attempts.remove(&task);
        let count = self.dispatch.transient.entry(task).or_default();
        *count += 1;
        if *count >= TRANSIENT_LIMIT {
            self.dispatch.transient.remove(&task);
            let message = format!(
                "Automatic launch #{task} deferred {TRANSIENT_LIMIT} times; task state kept changing: {reason}"
            );
            self.disable_dispatch();
            self.dispatch.contended = Some(message.clone());
            self.notice = message;
        } else {
            self.notice =
                format!("Automatic launch #{task} deferred: {reason}; retrying on current state");
        }
    }

    /// The intent was refused without effect because task state moved underneath it.
    pub fn intent_transient(&self, intent: &crate::command_intent::Intent) -> bool {
        self.prepared_transient.contains(intent.correlation())
    }

    fn apply_outcome(&mut self, result: Outcome) {
        match result {
            Ok(Projection {
                snapshot,
                notice,
                evidence,
                run_observations,
                scope,
                canonical_scope,
                gate,
            }) => {
                if let Some(state) = canonical_scope {
                    if self
                        .canonical_scope
                        .as_ref()
                        .is_none_or(|old| state.revision >= old.revision)
                    {
                        self.canonical_scope = Some(state);
                    }
                }
                if let Some(gate) = gate {
                    if gate.revision.is_none()
                        || self.scope_status.revision.is_none()
                        || gate.revision >= self.scope_status.revision
                    {
                        if gate.blocked {
                            self.disable_dispatch();
                        }
                        self.scope_status = gate;
                    }
                }
                if let Some(scope) = scope {
                    self.scope_view = Some(scope);
                    self.activity = None;
                    self.evidence = None;
                    self.planner.visible = false;
                }
                if evidence.is_none()
                    && self
                        .snapshot
                        .as_ref()
                        .is_none_or(|old| snapshot.revision >= old.revision)
                {
                    if let Some(task) = self.evidence.as_ref().map(|view| view.task) {
                        if let Some(summary) = snapshot.review_summary_for_task(task) {
                            let previous = self
                                .snapshot
                                .as_ref()
                                .and_then(|old| old.review_summary_for_task(task));
                            if previous.as_ref() != Some(&summary) {
                                self.evidence = self
                                    .evidence
                                    .take()
                                    .map(|view| view.with_review(Some(&summary)));
                            }
                        }
                    }
                }
                if self
                    .snapshot
                    .as_ref()
                    .is_none_or(|current| snapshot.revision >= current.revision)
                {
                    self.snapshot = Some(snapshot);
                    if let Some(observations) = run_observations {
                        self.run_observations = observations;
                    }
                }
                if let Some((request, view)) = evidence {
                    if self.pending_evidence == Some(request) {
                        self.pending_evidence = None;
                        if self.visible {
                            // The evidence reader can finish after a newer review
                            // snapshot was admitted. Keep its verified run body,
                            // but project reviewer text from current receipt truth.
                            let summary = self
                                .snapshot
                                .as_ref()
                                .and_then(|snapshot| snapshot.review_summary_for_task(view.task));
                            let view = view.with_review(summary.as_deref());
                            self.task_query.clear();
                            self.reveal_work_task(view.task);
                            self.focus_work_node(NodeId::Task(view.task));
                            self.evidence = Some(view);
                        }
                    }
                }
                self.remember_initial_work_focus();
                self.notice = notice;
            }
            Err(error) => {
                self.notice = if self.retry.is_some() {
                    format!(
                        "{error}. /retry-task repeats the exact request; /refresh reloads state"
                    )
                } else {
                    format!("{error}. /refresh reloads state")
                }
            }
        }
    }

    pub fn visible_tasks(&self) -> Vec<&crate::tasks::Task> {
        let Some(snapshot) = &self.snapshot else {
            return vec![];
        };
        self.work_tree()
            .rows
            .iter()
            .filter(|row| row.matches_filter)
            .filter_map(|row| row.task)
            .filter_map(|id| snapshot.tasks.iter().find(|task| task.id == id))
            .collect()
    }

    pub fn view_preferences(&self) -> crate::conversations::TaskView {
        crate::conversations::TaskView {
            visible: self.visible,
            selected: self
                .selected_task
                .or_else(|| self.selected_task().map(|task| task.id)),
            query: self.task_query.clone(),
        }
    }

    /// Leaving Mission Work cancels a pending evidence view choice even if the
    /// pane is reopened before its asynchronous response arrives.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            self.pending_evidence = None;
            self.autopilot_report = None;
        }
    }

    pub fn restore_view(&mut self, view: crate::conversations::TaskView) -> Result<(), String> {
        view.validate()?;
        self.visible = view.visible;
        self.selected_task = view.selected;
        self.focused_work_node = view.selected.map(NodeId::Task);
        self.collapsed_work_nodes.clear();
        self.pending_evidence = None;
        self.task_query = view.query;
        self.activity = None;
        self.evidence = None;
        self.scroll = 0;
        self.follow_tail.set(true);
        Ok(())
    }

    pub fn selected_task(&self) -> Option<&crate::tasks::Task> {
        let NodeId::Task(id) = self.focused_work_node()? else {
            return None;
        };
        self.snapshot
            .as_ref()?
            .tasks
            .iter()
            .find(|task| task.id == id)
    }

    /// Cached read-only hierarchy. Focus changes do not recompute canonical readiness.
    pub fn work_tree(&self) -> Arc<Tree> {
        let key = WorkTreeKey {
            revision: self.snapshot.as_ref().map(|snapshot| snapshot.revision),
            scope_revision: self.scope_status.revision,
            scope_blocked: self.scope_status.blocked,
            scope_label: self.scope_status.label.clone(),
            query: self.task_query.clone(),
            collapsed: self.collapsed_work_nodes.clone(),
        };
        if let Some((previous, tree)) = self.work_tree_cache.borrow().as_ref() {
            if *previous == key {
                return Arc::clone(tree);
            }
        }
        let tree = Arc::new(self.snapshot.as_ref().map_or_else(
            || Tree {
                rows: vec![],
                total_tasks: 0,
                matched_tasks: 0,
            },
            |snapshot| {
                crate::mission_work::project(
                    snapshot,
                    &self.scope_status,
                    &self.task_query,
                    &self.collapsed_work_nodes,
                )
            },
        ));
        *self.work_tree_cache.borrow_mut() = Some((key, Arc::clone(&tree)));
        tree
    }

    /// A saved task hidden by the view never falls back to a different action target.
    pub fn focused_work_node(&self) -> Option<NodeId> {
        let tree = self.work_tree();
        let focused = self
            .focused_work_node
            .or_else(|| self.selected_task.map(NodeId::Task))
            .or_else(|| tree.rows.iter().find_map(|row| row.task.map(NodeId::Task)))?;
        tree.rows
            .iter()
            .any(|row| row.id == focused)
            .then_some(focused)
    }

    fn remember_initial_work_focus(&mut self) {
        if self.focused_work_node.is_none() && self.selected_task.is_none() {
            if let Some(NodeId::Task(task)) = self.focused_work_node() {
                self.selected_task = Some(task);
                self.focused_work_node = Some(NodeId::Task(task));
            }
        }
    }

    fn focus_work_node(&mut self, node: NodeId) {
        self.remember_initial_work_focus();
        self.focused_work_node = Some(node);
        if let NodeId::Task(task) = node {
            self.selected_task = Some(task);
        }
        self.pending_evidence = None;
        self.activity = None;
        self.evidence = None;
        self.autopilot_report = None;
        self.scroll = 0;
        self.follow_tail.set(true);
    }

    fn reveal_work_task(&mut self, task: u64) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let tree = crate::mission_work::project(snapshot, &self.scope_status, "", &BTreeSet::new());
        let mut parent = tree
            .rows
            .iter()
            .find(|row| row.id == NodeId::Task(task))
            .and_then(|row| row.parent);
        while let Some(node) = parent {
            self.collapsed_work_nodes.remove(&node);
            parent = tree
                .rows
                .iter()
                .find(|row| row.id == node)
                .and_then(|row| row.parent);
        }
    }

    /// Expand the focused branch, or move into its first visible child.
    pub fn expand_work_node(&mut self) -> bool {
        if self.pending {
            return false;
        }
        self.manual_selection = Some(std::time::Instant::now());
        let Some(focused) = self.focused_work_node() else {
            return false;
        };
        let tree = self.work_tree();
        let Some(row) = tree.rows.iter().find(|row| row.id == focused) else {
            return false;
        };
        if row.expandable && !row.expanded {
            self.remember_initial_work_focus();
            self.collapsed_work_nodes.remove(&focused);
            self.scroll = 0;
            return true;
        }
        if let Some(child) = tree.rows.iter().find(|row| row.parent == Some(focused)) {
            self.focus_work_node(child.id);
            return true;
        }
        false
    }

    /// Collapse a branch, or move to its parent while retaining the last task anchor.
    pub fn collapse_work_node(&mut self) -> bool {
        if self.pending {
            return false;
        }
        self.manual_selection = Some(std::time::Instant::now());
        let Some(focused) = self.focused_work_node() else {
            return false;
        };
        let tree = self.work_tree();
        let Some(row) = tree.rows.iter().find(|row| row.id == focused) else {
            return false;
        };
        if row.expandable && row.expanded && self.task_query.trim().is_empty() {
            self.remember_initial_work_focus();
            self.collapsed_work_nodes.insert(focused);
            self.scroll = 0;
            return true;
        }
        if let Some(parent) = row.parent {
            self.focus_work_node(parent);
            return true;
        }
        false
    }

    /// Navigate from the last rendered boundary, avoiding invisible overscroll.
    pub fn scroll_rows(&mut self, rows: i32) {
        let maximum = self.scroll_max.get();
        let current = if self.detail_live.get() && self.follow_tail.get() {
            maximum
        } else {
            self.scroll.min(maximum)
        };
        self.scroll = if rows < 0 {
            current.saturating_sub(rows.unsigned_abs() as usize)
        } else {
            current.saturating_add(rows as usize).min(maximum)
        };
        self.follow_tail.set(self.scroll >= maximum);
    }

    /// Keep one rendered row in common between pages; tiny panes still advance.
    pub fn page_details(&mut self, forward: bool) {
        let rows = i32::from(self.scroll_height.get().saturating_sub(1).max(1));
        self.scroll_rows(if forward { rows } else { -rows });
    }

    pub fn select_task(&mut self, forward: bool) {
        if self.pending {
            return;
        }
        self.manual_selection = Some(std::time::Instant::now());
        let tree = self.work_tree();
        if tree.rows.is_empty() {
            return;
        }
        let index = self
            .focused_work_node()
            .and_then(|focused| tree.rows.iter().position(|row| row.id == focused));
        let next = match (index, forward) {
            (Some(index), true) => (index + 1) % tree.rows.len(),
            (Some(index), false) => (index + tree.rows.len() - 1) % tree.rows.len(),
            (None, true) => 0,
            (None, false) => tree.rows.len() - 1,
        };
        self.focus_work_node(tree.rows[next].id);
    }

    /// While autopilot is active, focus the running task unless the user moved
    /// the selection within the last few seconds. Returns whether focus moved.
    pub fn follow_running_task(&mut self) -> bool {
        use crate::autopilot::RunState;
        if self.pending
            || !self.autopilot.as_ref().is_some_and(|status| {
                matches!(
                    status.state,
                    RunState::Planning | RunState::Running | RunState::Finishing
                )
            })
            || self
                .manual_selection
                .is_some_and(|at| at.elapsed() < MANUAL_SELECTION_HOLD)
        {
            return false;
        }
        let Some(snapshot) = &self.snapshot else {
            return false;
        };
        let running = |id: u64| {
            snapshot
                .tasks
                .iter()
                .any(|task| task.id == id && task.status == crate::tasks::TaskStatus::Running)
        };
        if self.selected_task().is_some_and(|task| running(task.id)) {
            return false;
        }
        // Prefer a task with a live local worker, then any recorded running task.
        let target = self
            .progress
            .keys()
            .copied()
            .find(|id| running(*id))
            .or_else(|| {
                snapshot
                    .tasks
                    .iter()
                    .find(|task| task.status == crate::tasks::TaskStatus::Running)
                    .map(|task| task.id)
            });
        let Some(target) = target else {
            return false;
        };
        if !self
            .work_tree()
            .rows
            .iter()
            .any(|row| row.task == Some(target))
        {
            self.reveal_work_task(target);
        }
        if !self
            .work_tree()
            .rows
            .iter()
            .any(|row| row.task == Some(target))
        {
            return false;
        }
        self.focus_work_node(NodeId::Task(target));
        true
    }

    /// Age the last manual selection (tests and long-idle sessions).
    pub fn expire_manual_selection(&mut self, age: std::time::Duration) {
        self.manual_selection = self.manual_selection.and_then(|at| at.checked_sub(age));
    }

    /// Register observed worker progress (used by dispatch and by render fixtures).
    pub fn attach_progress(
        &mut self,
        task: u64,
        progress: watch::Receiver<crate::worker::Progress>,
        cancel: Arc<AtomicBool>,
    ) {
        self.progress.insert(task, progress);
        self.workers.insert(task, cancel);
    }

    /// True once per batch of new worker output, so streaming redraws promptly.
    pub fn progress_changed(&mut self) -> bool {
        let mut changed = false;
        for receiver in self.progress.values_mut() {
            if receiver.has_changed().unwrap_or(false) {
                receiver.borrow_and_update();
                changed = true;
            }
        }
        changed
    }

    pub fn has_live_workers(&self) -> bool {
        !self.progress.is_empty()
    }

    /// Current live worker view for a task, if a local worker is observed.
    pub fn worker_live(&self, task: u64) -> Option<crate::dashboard::Live> {
        let progress = self.progress.get(&task)?.borrow();
        let stage = progress
            .queue
            .as_ref()
            .map(crate::client_timing::queue_summary)
            .unwrap_or_else(|| progress.stage.into());
        Some(crate::dashboard::Live {
            stage: format!(
                "{stage} · {:.1}s · {} received",
                progress.started.elapsed().as_secs_f64(),
                crate::dashboard::bytes(progress.received_bytes)
            ),
            cancelling: self
                .workers
                .get(&task)
                .is_some_and(|flag| flag.load(Ordering::SeqCst)),
            model_output: progress.model_output.clone(),
            stdout: String::from_utf8_lossy(&progress.check_stdout).into_owned(),
            stderr: String::from_utf8_lossy(&progress.check_stderr).into_owned(),
        })
    }

    /// Compact verified outcome for a finished task. Evidence is read and verified
    /// through the store once per acknowledged evidence hash, then cached.
    pub fn outcome(&self, task: &crate::tasks::Task) -> Option<OutcomeLines> {
        let hash = task.run.as_ref()?.evidence_sha256.clone()?;
        if let Some((cached, lines)) = self.outcomes.borrow().get(&task.id) {
            if *cached == hash {
                return Some(Arc::clone(lines));
            }
        }
        let lines = Arc::new(
            self.store
                .evidence(task.id)
                .and_then(|raw| crate::dashboard::outcome_lines(&raw)),
        );
        self.outcomes
            .borrow_mut()
            .insert(task.id, (hash, Arc::clone(&lines)));
        Some(lines)
    }

    pub fn worker_output(&self, task: u64) -> Option<(String, String)> {
        let progress = self.progress.get(&task)?.borrow();
        Some((
            String::from_utf8_lossy(&progress.check_stdout).into_owned(),
            String::from_utf8_lossy(&progress.check_stderr).into_owned(),
        ))
    }

    pub fn worker_progress(&self, task: u64) -> Option<String> {
        self.progress.get(&task).map(|receiver| {
            let label = receiver.borrow().label();
            if self
                .workers
                .get(&task)
                .is_some_and(|flag| flag.load(Ordering::SeqCst))
            {
                format!("Cancellation requested · {label}")
            } else {
                label
            }
        })
    }

    pub fn set_provider(&mut self, provider: Ollama) {
        self.provider = Some(provider);
    }
    /// Invalidates every previously prepared toggle and pending enable.
    pub fn disable_dispatch(&mut self) {
        self.dispatch.enabled = false;
        self.dispatch_origin = None;
        self.controller_epoch = self.controller_epoch.saturating_add(1);
        if let Some(request) = self.control_pending.take() {
            self.prepared_errors.insert(
                request.correlation,
                "Dispatch enable superseded by a later shutdown or scope hold".into(),
            );
        }
    }

    pub fn take_control_events(&mut self) -> Vec<crate::control_command::Event> {
        std::mem::take(&mut self.control_events)
    }

    fn acknowledge_control(
        &mut self,
        request: crate::control_command::Request,
        outcome: crate::control_command::Outcome,
    ) {
        let event = crate::control_command::Event { request, outcome };
        self.control_acknowledged
            .insert(event.request.correlation.clone(), event.clone());
        self.control_events.push(event);
    }

    fn prepare_control(
        &mut self,
        operation: crate::control_command::Operation,
    ) -> Result<crate::command_intent::Intent, String> {
        self.sequence += 1;
        let request = crate::control_command::Request {
            correlation: format!("{}-{}", self.incarnation, self.sequence),
            controller: self.incarnation.clone(),
            operation,
        };
        request.validate()?;
        Ok(crate::command_intent::Intent::Control { request })
    }

    fn dispatch_control(
        &mut self,
        runtime: &Runtime,
        request: &crate::control_command::Request,
    ) -> Result<(), String> {
        use crate::control_command::{Operation, Outcome};
        request.validate()?;
        if request.controller != self.incarnation {
            return Err(
                "Saved control belongs to a previous controller; issue a new command".into(),
            );
        }
        if let Some(event) = self.control_acknowledged.get(&request.correlation) {
            if event.request != *request {
                return Err("Controller command identity was reused with different input".into());
            }
            self.control_events.push(event.clone());
            return Ok(());
        }
        self.prepared_errors.remove(&request.correlation);
        match &request.operation {
            Operation::CancelWorker {
                task,
                start_correlation,
                expected_start_revision,
            } => {
                let expected = WorkerIdentity {
                    correlation: start_correlation.clone(),
                    start_revision: *expected_start_revision,
                };
                if self.worker_identities.get(task) != Some(&expected) {
                    return Err("Exact worker owner is no longer active; inspect its result".into());
                }
                let flag = self
                    .workers
                    .get(task)
                    .ok_or("Exact worker owner is no longer active")?;
                flag.store(true, Ordering::SeqCst);
                self.notice = "Cancellation requested; awaiting worker receipt".into();
                self.acknowledge_control(request.clone(), Outcome::CancellationRequested);
            }
            Operation::Dispatch {
                enabled,
                expected_epoch,
                scope_revision_for_on,
            } => {
                if *expected_epoch != self.controller_epoch {
                    return Err("Dispatch controller changed; issue a new toggle".into());
                }
                if !enabled {
                    self.disable_dispatch();
                    self.notice =
                        "Dispatch off · active workers continue; /cancel-task ID stops a worker"
                            .into();
                    self.acknowledge_control(
                        request.clone(),
                        Outcome::DispatchChanged { enabled: false },
                    );
                } else {
                    if self.control_pending.is_some() {
                        return Err("Dispatch enable is already pending".into());
                    }
                    if self.scope_status.blocked
                        || self.scope_status.revision != *scope_revision_for_on
                    {
                        return Err("Scope changed; review it before enabling dispatch".into());
                    }
                    let store = self.store.clone();
                    let sender = self.control_sender.clone();
                    let request = request.clone();
                    self.control_pending = Some(request.clone());
                    self.notice = "Checking current scope before enabling dispatch".into();
                    runtime.spawn(async move {
                        let result =
                            tokio::task::spawn_blocking(move || store.understanding().snapshot())
                                .await
                                .unwrap_or_else(|_| {
                                    Err("Scope verifier stopped before dispatch enable".into())
                                });
                        let _ = sender.send((request, result));
                    });
                }
            }
        }
        Ok(())
    }

    pub fn cancel_all(&mut self) {
        self.disable_dispatch();
        for flag in self.workers.values() {
            flag.store(true, Ordering::SeqCst);
        }
    }

    pub fn refresh(&mut self, runtime: &Runtime) {
        if self.pending {
            return;
        }
        self.launch(runtime, None);
    }

    pub fn refresh_background(&mut self, runtime: &Runtime) {
        if self.refreshing || self.pending {
            return;
        }
        self.refreshing = true;
        let store = self.store.clone();
        let sender = self.background_sender.clone();
        runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                store.snapshot().map(|snapshot| {
                    let run_observations = Some(store.run_observations(&snapshot));
                    let scope_state = store.understanding().snapshot();
                    Projection {
                        snapshot,
                        notice: String::new(),
                        evidence: None,
                        run_observations,
                        gate: Some(crate::task_view::ScopeStatus::observe(scope_state.clone())),
                        scope: None,
                        canonical_scope: scope_state.ok(),
                    }
                })
            })
            .await
            .unwrap_or_else(|_| Err("Background task reader stopped".into()));
            let _ = sender.send(result).await;
        });
    }

    fn launch(&mut self, runtime: &Runtime, request: Option<Request>) {
        self.pending = true;
        self.writing = request.is_some();
        self.notice = "Saving / loading task queue…".into();
        let store = self.store.clone();
        let sender = self.sender.clone();
        if let Some(request) = request
            .as_ref()
            .filter(|request| matches!(request.action, Action::Assign { .. }))
        {
            let request = request.clone();
            let provider = self.provider.clone();
            self.notice = "Checking installed worker model; awaiting assignment receipt".into();
            runtime.spawn(async move {
                let result = match provider {
                    Some(provider) => crate::assignment::assign(store, request, provider)
                        .await
                        .map(Projection::from),
                    None => Err("Worker provider unavailable".into()),
                };
                let _ = sender.send(result.map_err(Refusal::from)).await;
            });
            return;
        }
        runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || match request {
                Some(request) => store.transact_checked(request).map(|(snapshot, receipt)| {
                    let notice = match &receipt.request.action {
                        Action::ReviewArchitecture { task, .. } if receipt.task == *task => format!("Architecture failures repeated · Architect revision required for #{task} · revision {}", receipt.revision),
                        Action::ReviewArchitecture { task, .. } => format!("Architecture review #{task} saved · repair #{} proposed · revision {}", receipt.task, receipt.revision),
                        Action::ResolveRepair { task } => format!("Repair #{task} selected for original dependencies · revision {}", receipt.revision),
                        Action::Plan { plan } => format!("Plan saved · {} proposed tasks starting at #{} · revision {} · approve each policy before /run", plan.tasks.len(), receipt.task, receipt.revision),
                        Action::ReviewAndRepair { task, .. } => format!("Review #{task} saved · repair #{} proposed · /approve {} before running · revision {}", receipt.task, receipt.task, receipt.revision),
                        _ => format!("Task #{} saved · revision {}", receipt.task, receipt.revision),
                    };
                    (snapshot, notice).into()
                }),
                None => store.snapshot().map(|snapshot| {
                    let run_observations = Some(store.run_observations(&snapshot));
                    let scope_state = store.understanding().snapshot();
                    Projection {
                        snapshot,
                        notice: "Task queue refreshed".into(),
                        evidence: None,
                        run_observations,
                        gate: Some(crate::task_view::ScopeStatus::observe(scope_state.clone())),
                        scope: None,
                        canonical_scope: scope_state.ok(),
                    }
                }).map_err(Refusal::from),
            })
            .await
            .unwrap_or_else(|_| Err("Task storage worker stopped; outcome unknown".into()));
            let _ = sender.send(result).await;
        });
    }

    fn start_worker(&mut self, runtime: &Runtime, task: u64) -> Result<(), String> {
        let expected_revision = self
            .snapshot
            .as_ref()
            .ok_or("Refresh task state first")?
            .revision;
        self.sequence += 1;
        let correlation = format!("{}-{}", self.incarnation, self.sequence);
        self.start_worker_prepared(runtime, task, expected_revision, correlation)
    }
    fn start_worker_prepared(
        &mut self,
        runtime: &Runtime,
        task: u64,
        expected_revision: u64,
        correlation: String,
    ) -> Result<(), String> {
        if self.scope_status.blocked {
            return Err(self.scope_status.label.clone());
        }
        if self.workers.len() >= 4 || self.workers.contains_key(&task) {
            return Err("Worker already active or four-worker limit reached".into());
        }
        if self
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.revision != expected_revision)
            && !self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .receipts
                    .iter()
                    .any(|receipt| receipt.request.correlation == correlation)
            })
        {
            return Err("Task state changed; refresh before running saved intent".into());
        }
        let snapshot = self.snapshot.as_ref().unwrap();
        let selected = snapshot
            .tasks
            .iter()
            .find(|candidate| candidate.id == task)
            .ok_or("Task not found")?;
        if let Some(blocker) = self.scope_status.run_blocker(snapshot, selected) {
            return Err(blocker);
        }
        if let Some(approval) = crate::dispatch::approval(self.snapshot.as_ref().unwrap(), task) {
            self.dispatch.attempts.insert(task, approval);
        }
        self.dispatch.failures.remove(&task);
        let provider = self.provider.clone().ok_or("Worker provider unavailable")?;
        let store = self.store.clone();
        let sender = self.worker_sender.clone();
        let start_revision = expected_revision
            .checked_add(1)
            .filter(|revision| *revision <= 4096)
            .ok_or("Start receipt capacity exhausted")?;
        let flag = Arc::new(AtomicBool::new(false));
        self.worker_identities.insert(
            task,
            WorkerIdentity {
                correlation: correlation.clone(),
                start_revision,
            },
        );
        self.workers.insert(task, flag.clone());
        let (observer, progress) = crate::worker::Observer::channel();
        self.progress.insert(task, progress);
        runtime.spawn(async move {
            let result = tokio::spawn(crate::worker::start_checked(
                store,
                task,
                correlation,
                expected_revision,
                provider,
                flag,
                observer,
            ))
            .await
            .unwrap_or_else(|_| {
                Err(
                    "Worker stopped unexpectedly; inspect retained run, do not replay effects"
                        .into(),
                )
            });
            let _ = sender.send((task, result.map(Projection::from))).await;
        });
        self.notice = format!("Starting worker for task #{task}; awaiting run receipt");
        Ok(())
    }

    /// Consume an acknowledged review route and prepare its draft without starting
    /// inference. Only the caller's exact saved-intent gate may dispatch it.
    pub fn prepare_architect(
        &mut self,
    ) -> Result<Option<crate::planner_command::ArchitectRequest>, String> {
        if self.pending {
            return Ok(None);
        }
        let Some(source) = self.architecture_launch.as_ref() else {
            return Ok(None);
        };
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or("Task queue unavailable before Architect preparation")?;
        let Some(receipt) = snapshot
            .receipts
            .get(source.expected_revision as usize)
            .filter(|receipt| {
                receipt.request == *source && receipt.revision == source.expected_revision + 1
            })
        else {
            return Err("Architect source review has not been acknowledged".into());
        };
        let source = self.architecture_launch.take().unwrap();
        let Action::ReviewArchitecture { task, .. } = source.action else {
            return Err("Architect source is not an architecture review".into());
        };
        // First failures create ordinary repairs; only an actual escalation routes here.
        if receipt.task != task || !snapshot.architecture_required(task) {
            return Ok(None);
        }
        self.sequence += 1;
        let correlation = format!("{}-{}", self.incarnation, self.sequence);
        let request = self
            .planner
            .prepare_command(
                &correlation,
                &format!("/architect-revise {task}"),
                "pending",
                snapshot,
            )?
            .ok_or("Architect operation could not be prepared")?;
        let request = crate::planner_command::ArchitectRequest { request, source };
        request.validate()?;
        Ok(Some(request))
    }

    fn dispatch_architect(
        &mut self,
        runtime: &Runtime,
        request: &crate::planner_command::ArchitectRequest,
    ) -> Result<(), String> {
        request.validate()?;
        if self.pending {
            return Err("Task acknowledgment is pending before Architect dispatch".into());
        }
        let crate::planner_command::Operation::Architect { origin, revision } =
            &request.request.operation
        else {
            return Err("Automatic Architect operation is invalid".into());
        };
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or("Task queue unavailable before Architect dispatch")?;
        if snapshot.revision != *revision
            || !snapshot
                .receipts
                .get(request.source.expected_revision as usize)
                .is_some_and(|receipt| {
                    receipt.request == request.source
                        && receipt.revision == origin.review_revision
                        && receipt.task == origin.task
                })
            || snapshot.architecture_origin(origin.task)? != *origin
        {
            return Err("Architect review source or task state changed before dispatch".into());
        }
        let provider = self
            .provider
            .clone()
            .ok_or("Planner provider unavailable")?;
        self.planner
            .dispatch_command(runtime, provider, &request.request, self.store.clone())?;
        self.prepared_errors.remove(&request.request.correlation);
        Ok(())
    }

    /// Select one eligible launch without starting a worker or reserving its attempt.
    /// The caller must save the returned exact intent before dispatch_prepared.
    pub fn prepare_dispatch(&mut self) -> Result<Option<crate::dispatch::RunRequest>, String> {
        if self.scope_status.blocked
            || !self.dispatch.enabled
            || self.pending
            || self.control_pending.is_some()
            || self.workers.len() >= 4
        {
            return Ok(None);
        }
        let source = self
            .dispatch_origin
            .as_ref()
            .ok_or("Enabled dispatch has no acknowledged source command")?;
        self.validate_dispatch_origin(source)?;
        let Some(snapshot) = &self.snapshot else {
            return Ok(None);
        };
        // Wait for an acknowledged claim before choosing another global revision.
        if self.workers.keys().any(|id| {
            snapshot
                .tasks
                .iter()
                .any(|task| task.id == *id && task.run.is_none())
        }) {
            return Ok(None);
        }
        let Some(task) = self.dispatch.next_matching(
            snapshot,
            &self.workers.keys().copied().collect(),
            |task| self.scope_status.run_blocker(snapshot, task).is_none(),
        ) else {
            return Ok(None);
        };
        let approval_revision = crate::dispatch::approval(snapshot, task)
            .ok_or("Automatic launch has no approval receipt")?;
        self.sequence += 1;
        let request = crate::dispatch::RunRequest {
            correlation: format!("{}-{}", self.incarnation, self.sequence),
            expected_revision: snapshot.revision,
            task,
            approval_revision,
            source: source.clone(),
        };
        request.validate()?;
        Ok(Some(request))
    }

    fn validate_dispatch_origin(
        &self,
        source: &crate::control_command::Request,
    ) -> Result<(), String> {
        let crate::control_command::Operation::Dispatch {
            enabled: true,
            expected_epoch,
            scope_revision_for_on,
        } = source.operation
        else {
            return Err("Automatic launch source must enable dispatch".into());
        };
        if !self.dispatch.enabled
            || source.controller != self.incarnation
            || self.dispatch_origin.as_ref() != Some(source)
            || expected_epoch.checked_add(1) != Some(self.controller_epoch)
        {
            return Err(
                "Automatic launch controller changed or stopped; issue an explicit /run to retry"
                    .into(),
            );
        }
        if self.scope_status.blocked || self.scope_status.revision != scope_revision_for_on {
            return Err("Scope changed since dispatch was enabled".into());
        }
        Ok(())
    }

    /// Stop retries of a launch that could not be published or admitted. Existing
    /// workers continue; a stale request cannot turn off a newer controller.
    pub fn refuse_dispatch(&mut self, request: &crate::dispatch::RunRequest, reason: &str) {
        if self.prepared_transient.contains(&request.correlation) {
            // Already deferred without effect; a fresh request retries on current state.
            return;
        }
        let reason = crate::console_command::bounded_reason(reason);
        self.prepared_errors
            .insert(request.correlation.clone(), reason.clone());
        if self.dispatch_origin.as_ref() == Some(&request.source) {
            self.dispatch
                .attempts
                .entry(request.task)
                .and_modify(|revision| *revision = (*revision).max(request.approval_revision))
                .or_insert(request.approval_revision);
            self.dispatch.failures.insert(request.task, reason.clone());
            self.disable_dispatch();
        }
        self.notice = format!("Automatic launch #{} refused: {reason}", request.task);
    }

    fn dispatch_automatic(
        &mut self,
        runtime: &Runtime,
        request: &crate::dispatch::RunRequest,
    ) -> Result<(), String> {
        let result: Result<(), Refusal> = (|| {
            request.validate()?;
            self.validate_dispatch_origin(&request.source)?;
            if self.pending || self.control_pending.is_some() {
                // Another acknowledgment will move task state; prepare again after it.
                return Err(Refusal::Busy);
            }
            let snapshot = self.snapshot.as_ref().ok_or("Task queue unavailable")?;
            if snapshot.revision != request.expected_revision {
                // This controller already holds newer state; nothing was claimed.
                return Err(Refusal::Stale(None));
            }
            if crate::dispatch::approval(snapshot, request.task) != Some(request.approval_revision)
            {
                return Err("Task revision or approval changed before automatic launch".into());
            }
            if self.dispatch.next_matching(
                snapshot,
                &self.workers.keys().copied().collect(),
                |task| {
                    task.id == request.task
                        && self.scope_status.run_blocker(snapshot, task).is_none()
                },
            ) != Some(request.task)
            {
                return Err(
                    "Automatic launch is no longer ready or has already been attempted".into(),
                );
            }
            self.start_worker_prepared(
                runtime,
                request.task,
                request.expected_revision,
                request.correlation.clone(),
            )?;
            self.prepared_workers.insert(
                request.task,
                crate::command_intent::Intent::DispatchRun {
                    request: request.clone(),
                },
            );
            Ok(())
        })();
        match result {
            Ok(()) => Ok(()),
            Err(refusal) if refusal.transient() => {
                let reason = refusal.to_string();
                self.prepared_errors
                    .insert(request.correlation.clone(), reason.clone());
                self.prepared_transient.insert(request.correlation.clone());
                self.defer_launch(request.task, &reason);
                Err(reason)
            }
            Err(refusal) => {
                let reason = refusal.to_string();
                self.refuse_dispatch(request, &reason);
                Err(reason)
            }
        }
    }

    pub fn intent_pending(&self, intent: &crate::command_intent::Intent) -> bool {
        matches!(intent, crate::command_intent::Intent::Control { request } if self.control_pending.as_ref() == Some(request))
            || intent
                .planner_request()
                .is_some_and(|request| self.planner.command_pending(request))
            || self.prepared_active.as_ref() == Some(intent)
            || self
                .prepared_workers
                .values()
                .any(|active| active == intent)
    }
    pub fn intent_error(&self, intent: &crate::command_intent::Intent) -> Option<String> {
        self.prepared_errors.get(intent.correlation()).cloned()
    }
    /// Capture targets and revisions now. This method never starts work or writes state.
    pub fn prepare_command(
        &mut self,
        text: &str,
        model: &str,
    ) -> Result<Option<crate::command_intent::Intent>, String> {
        use crate::command_intent::Intent;
        let mut text = text.trim().to_string();
        if matches!(
            text.as_str(),
            "/approve"
                | "/branch"
                | "/run"
                | "/cancel-task"
                | "/evidence"
                | "/accept"
                | "/reject"
                | "/recover"
        ) {
            text = format!(
                "{} {}",
                text,
                self.selected_task().ok_or("Select a task first")?.id
            );
        }
        let verb = text.split_whitespace().next().unwrap_or("");
        if matches!(
            verb,
            "/tasks" | "/activity" | "/chat" | "/refresh" | "/evidence"
        ) || text == "/scope"
            || text == "/plan"
        {
            return Ok(None);
        }
        if verb == "/dispatch" {
            let enabled = match text.as_str() {
                "/dispatch on" => true,
                "/dispatch off" => false,
                _ => return Err("Usage: /dispatch on|off".into()),
            };
            let scope_revision_for_on = if enabled {
                if self.scope_status.blocked {
                    return Err(self.scope_status.label.clone());
                }
                Some(
                    self.scope_status
                        .revision
                        .ok_or("Refresh scope before enabling dispatch")?,
                )
            } else {
                None
            };
            return self
                .prepare_control(crate::control_command::Operation::Dispatch {
                    enabled,
                    expected_epoch: self.controller_epoch,
                    scope_revision_for_on,
                })
                .map(Some);
        }
        if let Some(id) = text
            .strip_prefix("/cancel-task ")
            .and_then(|id| id.parse::<u64>().ok())
        {
            if let Some(identity) = self.worker_identities.get(&id) {
                return self
                    .prepare_control(crate::control_command::Operation::CancelWorker {
                        task: id,
                        start_correlation: identity.correlation.clone(),
                        expected_start_revision: identity.start_revision,
                    })
                    .map(Some);
            }
        }
        if self.pending {
            return Err("Wait for the pending task acknowledgment".into());
        }
        if text == "/retry-task" {
            return self
                .prepared_retry
                .clone()
                .or_else(|| self.retry.clone().map(|request| Intent::Task { request }))
                .map(Some)
                .ok_or("No saved task intent to retry".into());
        }
        if text == "/scope-retry" {
            return self
                .scope_retry
                .clone()
                .map(|request| Some(Intent::Scope { request }))
                .ok_or("No scope request to retry".into());
        }
        self.sequence += 1;
        let correlation = format!("{}-{}", self.incarnation, self.sequence);
        if matches!(
            verb,
            "/plan" | "/plan-revise" | "/architect-revise" | "/plan-cancel"
        ) {
            let snapshot = self.snapshot.as_ref().ok_or("Refresh task state first")?;
            return self
                .planner
                .prepare_command(&correlation, &text, model, snapshot)
                .map(|request| request.map(|request| Intent::Planner { request }));
        }
        let intent = if text.starts_with("/scope ") || text.starts_with("/scope-confirm ") {
            let revision = self
                .scope_view
                .as_ref()
                .ok_or("Open /scope and review the current state first")?
                .revision;
            let action = if let Some(json) = text.strip_prefix("/scope ") {
                crate::understanding::Action::Draft {
                    brief: serde_json::from_str(json).map_err(|_| "Invalid scope draft JSON")?,
                }
            } else {
                crate::understanding::Action::Confirm {
                    draft_revision: text
                        .strip_prefix("/scope-confirm ")
                        .unwrap()
                        .parse()
                        .map_err(|_| "Usage: /scope-confirm DRAFT_REVISION")?,
                }
            };
            Intent::Scope {
                request: crate::understanding::Request {
                    correlation,
                    expected_revision: revision,
                    action,
                },
            }
        } else {
            let snapshot = self
                .snapshot
                .as_ref()
                .ok_or("Task queue unavailable; /refresh first")?;
            if matches!(verb, "/run" | "/branch" | "/recover") {
                let task: u64 = text
                    .split_once(' ')
                    .ok_or("Command needs a task ID")?
                    .1
                    .parse()
                    .map_err(|_| "Invalid task ID")?;
                let item = snapshot
                    .tasks
                    .iter()
                    .find(|t| t.id == task)
                    .ok_or("Unknown task")?;
                match verb {
                    "/run" => Intent::Run {
                        correlation,
                        expected_revision: snapshot.revision,
                        task,
                    },
                    "/branch" => {
                        let existing = snapshot.receipts.iter().find(|receipt| matches!(receipt.request.action, Action::Branch { task: id, .. } if id == task));
                        Intent::Branch {
                            correlation: existing
                                .map_or(correlation, |receipt| receipt.request.correlation.clone()),
                            expected_revision: existing.map_or(snapshot.revision, |receipt| {
                                receipt.request.expected_revision
                            }),
                            task,
                            run: item.run.as_ref().ok_or("Task has not run")?.id.clone(),
                        }
                    }
                    _ => Intent::Recover {
                        correlation,
                        task,
                        run: item.run.as_ref().ok_or("Task has not run")?.id.clone(),
                    },
                }
            } else if text == "/plan-save" {
                if self.planner.active() {
                    return Err("Wait for the complete plan before saving".into());
                }
                Intent::Task {
                    request: Request {
                        correlation,
                        expected_revision: self.planner.revision,
                        action: Action::Plan {
                            plan: self
                                .planner
                                .draft
                                .clone()
                                .ok_or("No complete validated plan to save")?,
                        },
                    },
                }
            } else {
                Intent::Task {
                    request: Request {
                        correlation,
                        expected_revision: snapshot.revision,
                        action: parse(&text, model)?,
                    },
                }
            }
        };
        intent.validate()?;
        Ok(Some(intent))
    }
    /// Only the caller's durable-save acknowledgment may call this for a new intent.
    pub fn dispatch_prepared(
        &mut self,
        runtime: &Runtime,
        intent: &crate::command_intent::Intent,
    ) -> Result<(), String> {
        use crate::command_intent::Intent;
        intent.validate()?;
        if intent.selection_request().is_some() {
            return Err("Selection must pass the workspace journal and handoff boundary".into());
        }
        if matches!(intent, Intent::Wayfinder { .. }) {
            return Err("Wayfinder turns require the saved Router dispatch boundary".into());
        }
        if let Intent::Control { request } = intent {
            self.prepared_retry = Some(intent.clone());
            return self.dispatch_control(runtime, request);
        }
        if let Intent::DispatchRun { request } = intent {
            return self.dispatch_automatic(runtime, request);
        }
        if let Intent::ArchitectDraft { request } = intent {
            return self.dispatch_architect(runtime, request);
        }
        if self.pending || self.intent_pending(intent) {
            return Err("Command acknowledgment is already pending".into());
        }
        self.prepared_errors.remove(intent.correlation());
        self.prepared_transient.remove(intent.correlation());
        self.prepared_retry = Some(intent.clone());
        self.visible = true;
        let target = match intent {
            Intent::Task {
                request:
                    Request {
                        action:
                            Action::Review { task, .. }
                            | Action::Assess { task, .. }
                            | Action::Decide { task, .. }
                            | Action::ReviewAndRepair { task, .. }
                            | Action::ReviewArchitecture { task, .. },
                        ..
                    },
            } => Some(*task),
            _ => None,
        };
        if self
            .evidence
            .as_ref()
            .is_none_or(|view| Some(view.task) != target)
        {
            self.evidence = None;
        }
        self.activity = None;
        self.planner.visible = false;
        match intent {
            Intent::Selection { .. }
            | Intent::SelectionArrival { .. }
            | Intent::Control { .. }
            | Intent::DispatchRun { .. }
            | Intent::ArchitectDraft { .. }
            | Intent::Wayfinder { .. } => {
                unreachable!("controller commands dispatch before task admission")
            }
            Intent::Planner { request } => {
                let provider = self
                    .provider
                    .clone()
                    .ok_or("Planner provider unavailable")?;
                self.planner
                    .dispatch_command(runtime, provider, request, self.store.clone())?;
                self.scroll = 0;
            }
            Intent::Task { request } => {
                self.retry = Some(request.clone());
                self.architecture_launch =
                    matches!(request.action, Action::ReviewArchitecture { .. })
                        .then(|| request.clone());
                self.prepared_active = Some(intent.clone());
                self.launch(runtime, Some(request.clone()));
            }
            Intent::Run {
                correlation,
                expected_revision,
                task,
            } => {
                self.start_worker_prepared(
                    runtime,
                    *task,
                    *expected_revision,
                    correlation.clone(),
                )?;
                self.prepared_workers.insert(*task, intent.clone());
            }
            Intent::Scope { request } => {
                self.scope_retry = Some(request.clone());
                let request = request.clone();
                let store = self.store.clone();
                let sender = self.sender.clone();
                self.pending = true;
                self.writing = true;
                self.prepared_active = Some(intent.clone());
                runtime.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let state = store.understanding().transact(request)?;
                        Ok(Projection {
                            snapshot: store.snapshot()?,
                            notice: format!("Scope receipt saved · revision {}", state.revision),
                            evidence: None,
                            run_observations: None,
                            scope: Some(state.clone()),
                            canonical_scope: Some(state.clone()),
                            gate: Some(crate::task_view::ScopeStatus::observe(Ok(state))),
                        })
                    })
                    .await
                    .unwrap_or_else(|_| Err("Scope worker stopped; outcome unknown".into()));
                    let _ = sender.send(result).await;
                });
            }
            Intent::Branch {
                correlation,
                expected_revision,
                task,
                run,
            } => {
                let (task, expected_revision, correlation, run) =
                    (*task, *expected_revision, correlation.clone(), run.clone());
                let store = self.store.clone();
                let sender = self.sender.clone();
                self.pending = true;
                self.writing = true;
                self.prepared_active = Some(intent.clone());
                runtime.spawn(async move {
                    let check_store = store.clone();
                    let checked =
                        tokio::task::spawn_blocking(move || {
                            let snapshot = check_store.snapshot()?;
                            if !snapshot.tasks.iter().any(|t| {
                                t.id == task && t.run.as_ref().is_some_and(|r| r.id == run)
                            }) {
                                return Err("Branch source run changed".to_string());
                            }
                            Ok(())
                        })
                        .await
                        .unwrap_or_else(|_| Err("Branch verifier stopped; outcome unknown".into()));
                    let result = match checked {
                        Ok(()) => {
                            crate::branch::publish(store, task, expected_revision, correlation)
                                .await
                                .map(Projection::from)
                        }
                        Err(error) => Err(error),
                    };
                    let _ = sender.send(result.map_err(Refusal::from)).await;
                });
            }
            Intent::Recover { task, run, .. } => {
                let (task, run) = (*task, run.clone());
                let store = self.store.clone();
                let sender = self.sender.clone();
                self.pending = true;
                self.writing = true;
                self.prepared_active = Some(intent.clone());
                runtime.spawn(async move {
                    let result =
                        tokio::task::spawn_blocking(move || {
                            let snapshot = store.snapshot()?;
                            if !snapshot.tasks.iter().any(|t| {
                                t.id == task && t.run.as_ref().is_some_and(|r| r.id == run)
                            }) {
                                return Err("Recovery source run changed".into());
                            }
                            store.recover(task).map(Projection::from)
                        })
                        .await
                        .unwrap_or_else(|_| Err("Recovery worker stopped; outcome unknown".into()));
                    let _ = sender.send(result.map_err(Refusal::from)).await;
                });
            }
        }
        Ok(())
    }

    pub fn command(&mut self, runtime: &Runtime, text: &str, model: &str) -> Result<(), String> {
        let selected_command;
        let text = if matches!(
            text.trim(),
            "/approve"
                | "/branch"
                | "/run"
                | "/cancel-task"
                | "/evidence"
                | "/accept"
                | "/reject"
                | "/recover"
        ) {
            let id = self.selected_task().ok_or("Select a task first")?.id;
            selected_command = format!("{} {id}", text.trim());
            &selected_command
        } else {
            text
        };
        self.visible = true;
        self.autopilot_report = None;
        let mut words = text.split_whitespace();
        let review_target = match words.next() {
            Some("/review" | "/accept" | "/reject") => {
                words.next().and_then(|id| id.parse::<u64>().ok())
            }
            Some("/retry-task") => self
                .retry
                .as_ref()
                .and_then(|request| match request.action {
                    Action::Assess { task, .. }
                    | Action::Decide { task, .. }
                    | Action::ReviewAndRepair { task, .. }
                    | Action::ReviewArchitecture { task, .. }
                    | Action::Review { task, .. } => Some(task),
                    _ => None,
                }),
            _ => None,
        };
        if self
            .evidence
            .as_ref()
            .is_none_or(|view| Some(view.task) != review_target)
        {
            self.evidence = None;
            self.pending_evidence = None;
        }
        if text.trim() == "/scope"
            || text.trim().starts_with("/scope ")
            || text.trim().starts_with("/scope-confirm ")
            || text.trim() == "/scope-retry"
        {
            if self.pending {
                return Err("Wait for the pending acknowledgment".into());
            }
            let text = text.trim();
            let request = if text == "/scope" {
                None
            } else if text == "/scope-retry" {
                Some(
                    self.scope_retry
                        .clone()
                        .ok_or("No scope request to retry")?,
                )
            } else {
                let revision = self
                    .scope_view
                    .as_ref()
                    .ok_or("Open /scope and review the current state first")?
                    .revision;
                let action = if let Some(json) = text.strip_prefix("/scope ") {
                    crate::understanding::Action::Draft {
                        brief: serde_json::from_str(json).map_err(|_| {
                            "Usage: /scope JSON with destination, scope, constraints, uncertainty"
                        })?,
                    }
                } else {
                    crate::understanding::Action::Confirm {
                        draft_revision: text
                            .strip_prefix("/scope-confirm ")
                            .unwrap()
                            .parse()
                            .map_err(|_| "Usage: /scope-confirm DRAFT_REVISION")?,
                    }
                };
                self.sequence += 1;
                Some(crate::understanding::Request {
                    correlation: format!("{}-scope-{}", self.incarnation, self.sequence),
                    expected_revision: revision,
                    action,
                })
            };
            if request.is_some() {
                self.scope_retry = request.clone();
            }
            self.pending = true;
            self.writing = request.is_some();
            self.activity = None;
            self.planner.visible = false;
            self.scroll = 0;
            self.notice = "Shared Understanding · awaiting saved state".into();
            let store = self.store.clone();
            let sender = self.sender.clone();
            runtime.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let scope = store.understanding();
                    let state = match request {
                        Some(request) => scope.transact(request)?,
                        None => scope.snapshot()?,
                    };
                    Ok(Projection {
                        snapshot: store.snapshot()?,
                        notice: format!(
                            "Shared Understanding observed · revision {} · no task action taken",
                            state.revision
                        ),
                        scope: Some(state.clone()),
                        canonical_scope: Some(state.clone()),
                        evidence: None,
                        gate: Some(crate::task_view::ScopeStatus::observe(Ok(state))),
                        run_observations: None,
                    })
                })
                .await
                .unwrap_or_else(|_| {
                    Err("Understanding worker stopped; retry the exact request".into())
                });
                let _ = sender.send(result).await;
            });
            return Ok(());
        }
        self.scope_view = None;
        if text.trim() == "/activity" || text.trim().starts_with("/activity ") {
            let query = text.trim().strip_prefix("/activity").unwrap().trim();
            if query.len() > 200 || query.chars().any(char::is_control) {
                return Err("Activity query must be at most 200 bytes without controls".into());
            }
            self.planner.visible = false;
            self.activity = Some(query.into());
            self.scroll = 0;
            self.refresh(runtime);
            return Ok(());
        }
        if text.trim() != "/refresh" {
            self.activity = None;
        }
        if let Some(id) = text
            .trim()
            .strip_prefix("/cancel-task ")
            .and_then(|id| id.parse::<u64>().ok())
        {
            if self.worker_identities.contains_key(&id) {
                let intent = self
                    .prepare_command(text, model)?
                    .ok_or("Worker control unavailable")?;
                return self.dispatch_prepared(runtime, &intent);
            }
        }
        if let Some(query) = text.trim().strip_prefix("/tasks ") {
            if self.pending {
                return Err("Wait for the pending task receipt before changing the filter".into());
            }
            let query = query.trim();
            if query.len() > 200 || query.chars().any(char::is_control) {
                return Err("Task query must be at most 200 bytes without controls".into());
            }
            self.remember_initial_work_focus();
            self.task_query = query.into();
            if let Some(task) = query
                .strip_prefix('#')
                .and_then(|id| id.parse::<u64>().ok())
                .filter(|id| *id > 0)
            {
                self.reveal_work_task(task);
                self.focus_work_node(NodeId::Task(task));
            }
            self.planner.visible = false;
            self.scroll = 0;
            self.notice = "Task filter applied · /tasks clears it · no task action taken".into();
            return Ok(());
        }
        match text.trim() {
            "/tasks" => {
                self.remember_initial_work_focus();
                self.task_query.clear();
                self.planner.visible = false;
                self.refresh(runtime);
                return Ok(());
            }
            "/chat" => {
                self.set_visible(false);
                return Ok(());
            }
            "/refresh" => {
                self.refresh(runtime);
                return Ok(());
            }
            _ => {}
        }
        if text.trim().starts_with("/dispatch") {
            let intent = self
                .prepare_command(text, model)?
                .ok_or("Dispatch control unavailable")?;
            return self.dispatch_prepared(runtime, &intent);
        }
        if self.pending {
            return Err("Wait for the pending task receipt".into());
        }
        if let Some(id) = text.trim().strip_prefix("/architect-revise ") {
            let task = id.parse().map_err(|_| "Usage: /architect-revise ID")?;
            let provider = self
                .provider
                .clone()
                .ok_or("Planner provider unavailable")?;
            self.planner
                .revise_architecture(runtime, provider, task, self.store.clone())?;
            self.scroll = 0;
            return Ok(());
        }
        if text.trim() == "/plan-cancel" {
            self.planner.cancel();
            self.planner.draft = None;
            self.planner.notice = "Plan cancelled · no action taken".into();
            return Ok(());
        }
        if text.trim() == "/plan" {
            if self.planner.notice.is_empty() {
                return Err("Usage: /plan REQUEST".into());
            }
            self.planner.visible = true;
            self.scroll = 0;
            return Ok(());
        }
        if let Some(prompt) = text.trim().strip_prefix("/plan ") {
            let revision = self
                .snapshot
                .as_ref()
                .ok_or("Refresh task state first")?
                .revision;
            let provider = self
                .provider
                .clone()
                .ok_or("Planner provider unavailable")?;
            self.evidence = None;
            self.scroll = 0;
            self.planner.start(
                runtime,
                provider,
                prompt,
                model,
                revision,
                self.store.clone(),
            )?;
            return Ok(());
        }
        if text.trim() == "/plan-revise" {
            return Err("Usage: /plan-revise REQUEST".into());
        }
        if let Some(request) = text.trim().strip_prefix("/plan-revise ") {
            let revision = self
                .snapshot
                .as_ref()
                .ok_or("Refresh task state first")?
                .revision;
            let provider = self
                .provider
                .clone()
                .ok_or("Planner provider unavailable")?;
            self.planner
                .revise(runtime, provider, request, revision, self.store.clone())?;
            self.evidence = None;
            self.scroll = 0;
            return Ok(());
        }
        if text.trim() == "/plan-save" {
            if self.planner.active() {
                return Err("Wait for the complete plan before saving".into());
            }
            let plan = self
                .planner
                .draft
                .clone()
                .ok_or("No complete validated plan to save")?;
            self.sequence += 1;
            let request = Request {
                correlation: format!("{}-{}", self.incarnation, self.sequence),
                expected_revision: self.planner.revision,
                action: Action::Plan { plan },
            };
            self.retry = Some(request.clone());
            self.launch(runtime, Some(request));
            return Ok(());
        }

        if let Some(id) = text.trim().strip_prefix("/branch ") {
            let task = id.parse().map_err(|_| "Usage: /branch ID")?;
            let revision = self
                .snapshot
                .as_ref()
                .ok_or("Refresh task state first")?
                .revision;
            self.sequence += 1;
            let correlation = format!("{}-{}", self.incarnation, self.sequence);
            self.pending = true;
            self.writing = true;
            self.retry = None;
            self.notice = format!("Verifying review branch for task #{task}");
            let store = self.store.clone();
            let sender = self.sender.clone();
            runtime.spawn(async move {
                let result = crate::branch::publish(store, task, revision, correlation)
                    .await
                    .map(Projection::from);
                let _ = sender.send(result.map_err(Refusal::from)).await;
            });
            return Ok(());
        }
        if let Some(id) = text.trim().strip_prefix("/recover ") {
            let task: u64 = id.parse().map_err(|_| "Usage: /recover ID")?;
            self.pending = true;
            self.writing = true;
            let store = self.store.clone();
            let sender = self.sender.clone();
            runtime.spawn(async move {
                let result =
                    tokio::task::spawn_blocking(move || store.recover(task).map(Projection::from))
                        .await
                        .unwrap_or_else(|_| {
                            Err("Recovery stopped; refresh before retrying /recover".into())
                        });
                let _ = sender.send(result.map_err(Refusal::from)).await;
            });
            return Ok(());
        }
        if let Some(id) = text.trim().strip_prefix("/evidence ") {
            let task: u64 = id.parse().map_err(|_| "Usage: /evidence ID")?;
            if self.pending {
                return Err("Wait for the pending acknowledgment before opening evidence".into());
            }
            self.pending = true;
            self.writing = false;
            self.sequence += 1;
            let evidence_request = self.sequence;
            self.pending_evidence = Some(evidence_request);
            self.scroll = 0;
            let store = self.store.clone();
            let sender = self.sender.clone();
            runtime.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let snapshot = store.snapshot()?;
                    let evidence =
                        crate::review::View::from_verified(task, &store.evidence(task)?)?
                            .with_acceptance(snapshot.acceptance_for_task(task))
                            .with_review(snapshot.review_summary_for_task(task).as_deref())
                            .with_inputs(
                                snapshot
                                    .tasks
                                    .iter()
                                    .find(|item| item.id == task)
                                    .and_then(|item| item.run.as_ref())
                                    .map(|run| run.inputs.as_slice())
                                    .unwrap_or_default(),
                            );
                    Ok(Projection {
                        snapshot,
                        notice: format!(
                            "Verified evidence for task #{task}; /tasks returns to queue"
                        ),
                        evidence: Some((evidence_request, evidence)),
                        scope: None,
                        canonical_scope: None,
                        gate: None,
                        run_observations: None,
                    })
                })
                .await
                .unwrap_or_else(|_| Err("Evidence reader stopped".into()));
                let _ = sender.send(result).await;
            });
            return Ok(());
        }
        if let Some(id) = text.trim().strip_prefix("/run ") {
            return self.start_worker(runtime, id.parse().map_err(|_| "Usage: /run ID")?);
        }

        if text.trim() == "/retry-task" {
            let request = self
                .retry
                .clone()
                .ok_or("No failed task request to retry")?;
            self.launch(runtime, Some(request));
            return Ok(());
        }
        let action = parse(text, model)?;
        self.planner.visible = false;
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or("Task queue unavailable; /refresh first")?;
        self.sequence += 1;
        let request = Request {
            correlation: format!("{}-{}", self.incarnation, self.sequence),
            expected_revision: snapshot.revision,
            action,
        };
        self.architecture_launch =
            matches!(request.action, Action::ReviewArchitecture { .. }).then(|| request.clone());
        self.retry = Some(request.clone());
        self.launch(runtime, Some(request));
        Ok(())
    }
}

pub fn parse(text: &str, model: &str) -> Result<Action, String> {
    if let Some(arguments) = text.trim().strip_prefix("/assign ") {
        let (id, model) = arguments.split_once(' ').ok_or("Usage: /assign ID MODEL")?;
        let task = id.parse().map_err(|_| "Use a numeric task id")?;
        let model = model.trim();
        if model.is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
            return Err("Invalid worker model identity".into());
        }
        return Ok(Action::Assign {
            task,
            model: model.into(),
        });
    }

    let text = text.trim();
    if let Some(id) = text.strip_prefix("/resolve-repair ") {
        return Ok(Action::ResolveRepair {
            task: id.trim().parse().map_err(|_| "Usage: /resolve-repair ID")?,
        });
    }
    if let Some(arguments) = text.strip_prefix("/repair ") {
        let (id, reason) = arguments
            .split_once(' ')
            .ok_or("Usage: /repair ID reason")?;
        return Ok(Action::Repair {
            task: id.parse().map_err(|_| "Invalid task id")?,
            reason: reason.trim().into(),
        });
    }
    if let Some(arguments) = text.strip_prefix("/permit ") {
        let (id, json) = arguments
            .split_once(' ')
            .ok_or("Usage: /permit ID {\"files\":[\"path\"],\"check\":[\"program\",\"arg\"]}")?;
        let task = id.parse().map_err(|_| "Invalid task id")?;
        let policy: crate::tasks::WorkPolicy = serde_json::from_str(json)
            .map_err(|_| "Policy must be JSON with exact files and check argv")?;
        policy.validate()?;
        return Ok(Action::Permit { task, policy });
    }
    if let Some(arguments) = text.strip_prefix("/review ") {
        let (id, json) = arguments.split_once(' ').ok_or(
            "Usage: /review ID JSON (outcome, reason, criteria; limitations for limited approval)",
        )?;
        let task = id.parse().map_err(|_| "Invalid task id")?;
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum ReviewInput {
            Decision(crate::assessment::Decision),
            Assessment(crate::assessment::Assessment),
        }
        match serde_json::from_str::<ReviewInput>(json).map_err(|_| "Review JSON needs outcome (or legacy accept), reason, criteria; limited approval also needs limitations")? {
            ReviewInput::Decision(decision) => {
                decision.validate()?;
                return Ok(if decision.proposes_repair() && decision.failure == Some(crate::assessment::FailureKind::Architecture) { Action::ReviewArchitecture { task, decision } } else if decision.proposes_repair() { Action::ReviewAndRepair { task, decision } } else { Action::Decide { task, decision } });
            }
            ReviewInput::Assessment(assessment) => { assessment.validate()?; return Ok(Action::Assess { task, assessment }); }
        }
    }
    for (prefix, accept) in [("/accept ", true), ("/reject ", false)] {
        if let Some(id) = text.strip_prefix(prefix) {
            return Ok(Action::Review {
                task: id.parse().map_err(|_| "Invalid task id")?,
                accept,
            });
        }
    }
    if let Some(title) = text.strip_prefix("/task ") {
        if title.trim().is_empty() {
            return Err("Usage: /task description".into());
        }
        return Ok(Action::Propose {
            title: title.trim().into(),
            model: model.into(),
            dependencies: vec![],
        });
    }
    if let Some(arguments) = text.strip_prefix("/after ") {
        let (ids, title) = arguments
            .split_once(' ')
            .ok_or("Usage: /after 1,2 description")?;
        let dependencies = ids
            .split(',')
            .map(|id| {
                id.parse::<u64>()
                    .map_err(|_| "Invalid dependency id".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if title.trim().is_empty() {
            return Err("Task description is required".into());
        }
        return Ok(Action::Propose {
            title: title.trim().into(),
            model: model.into(),
            dependencies,
        });
    }
    for (prefix, approve) in [("/approve ", true), ("/cancel-task ", false)] {
        if let Some(id) = text.strip_prefix(prefix) {
            let task = id.trim().parse().map_err(|_| "Use a numeric task id")?;
            return Ok(if approve {
                Action::Approve { task }
            } else {
                Action::Cancel { task }
            });
        }
    }
    Err("Commands: /task description · /permit ID JSON · /approve ID · /run ID · /cancel-task ID · /evidence ID · /recover ID · /accept ID · /reject ID · /tasks · /activity [query] · /chat · /refresh · /retry-task".into())
}

#[cfg(test)]
mod evidence_refresh_tests {
    use super::*;
    use crate::{
        assessment::{Decision, Outcome as ReviewOutcome},
        tasks::{TaskStatus, WorkPolicy},
        worker::Evidence,
    };
    use sha2::{Digest, Sha256};
    use std::{fs, path::PathBuf};

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn transact(store: &TaskStore, action: Action) -> Snapshot {
        let revision = store.snapshot().unwrap().revision;
        store
            .transact(Request {
                correlation: format!("review-refresh-{revision}"),
                expected_revision: revision,
                action,
            })
            .unwrap()
            .0
    }

    fn review(reason: &str, outcome: ReviewOutcome) -> Action {
        Action::Decide {
            task: 1,
            decision: Decision {
                failure: None,
                risk: None,
                outcome,
                reason: reason.into(),
                criteria: vec![],
                limitations: vec![],
            },
        }
    }

    #[test]
    fn delayed_evidence_uses_admitted_review_after_newer_background_snapshot() {
        let fixture = Fixture(std::env::temp_dir().join(format!(
            "alfredo-evidence-refresh-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        )));
        fs::create_dir_all(fixture.0.join("workspace")).unwrap();
        let store = TaskStore::new(
            &fixture.0.join("state"),
            &fixture.0.join("workspace"),
            "review-refresh",
        )
        .unwrap();
        for action in [
            Action::Propose {
                title: "Review refresh race".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
            Action::Permit {
                task: 1,
                policy: WorkPolicy {
                    files: vec!["result.txt".into()],
                    check: vec!["true".into()],
                },
            },
            Action::Approve { task: 1 },
            Action::Start {
                task: 1,
                baseline: "a".repeat(40),
                inputs: vec![],
            },
        ] {
            transact(&store, action);
        }
        let started = store.snapshot().unwrap();
        let run = started.tasks[0].run.as_ref().unwrap();
        let evidence = serde_json::to_vec(&Evidence {
            agent: None,
            candidate_commit: None,
            model_metrics: None,
            generation: None,
            run: run.id.clone(),
            baseline: run.baseline.clone(),
            status: TaskStatus::Failed,
            detail: "Exact retained run body".into(),
            patch: String::new(),
            check: None,
        })
        .unwrap();
        let directory = store.run_directory(&run.id).unwrap();
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("evidence.json"), &evidence).unwrap();
        transact(
            &store,
            Action::Finish {
                task: 1,
                run: run.id.clone(),
                status: TaskStatus::Failed,
                evidence_sha256: format!("{:x}", Sha256::digest(&evidence)),
                detail: "Exact retained run body".into(),
            },
        );
        let older = transact(
            &store,
            review(
                "Earlier review awaits discussion",
                ReviewOutcome::NeedsHumanReview,
            ),
        );
        let view = crate::review::View::from_verified(1, &store.evidence(1).unwrap())
            .unwrap()
            .with_review(older.review_summary_for_task(1).as_deref());
        let newer = transact(
            &store,
            review("Current review resolves the hold", ReviewOutcome::Rejected),
        );
        let canonical =
            fs::read(store.conversation_directory().unwrap().join("tasks.json")).unwrap();

        let mut control = TaskControl::new(store.clone());
        control.snapshot = Some(older.clone());
        control.visible = true;
        control.pending = true;
        control.pending_evidence = Some(41);
        assert!(control
            .background_sender
            .try_send(Ok(
                (newer.clone(), "Background review refreshed".into()).into()
            ))
            .is_ok());
        let mut delayed = Projection::from((older, "Verified retained evidence".into()));
        delayed.evidence = Some((41, view));
        assert!(control.sender.try_send(Ok(delayed)).is_ok());
        assert!(control.poll());

        assert_eq!(control.snapshot.as_ref().unwrap().revision, newer.revision);
        assert_eq!(control.selected_task().unwrap().id, 1);
        let opened = control.evidence.as_ref().unwrap();
        assert_eq!(opened.task, 1);
        let text = opened
            .lines()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Current review resolves the hold"), "{text}");
        assert!(!text.contains("Earlier review awaits discussion"), "{text}");
        assert!(text.contains(&run.id), "{text}");
        assert!(text.contains("Exact retained run body"), "{text}");
        assert_eq!(store.evidence(1).unwrap().as_bytes(), evidence);
        assert_eq!(
            fs::read(store.conversation_directory().unwrap().join("tasks.json")).unwrap(),
            canonical
        );
    }
}
