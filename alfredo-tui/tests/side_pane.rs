//! Side pane projection, keys, mission discovery and the open seam; no rendering.
use alfredo_tui::{
    conversations::TaskView,
    mission_work::NodeId,
    model::{App, Update},
    planner::{Plan, Step},
    side_pane::{
        self, MissionEntry, MissionProgress, OpenTarget, PaneAction, PaneKey, RowKey, RowKind,
        Section,
    },
    task_control::TaskControl,
    tasks::{Action, Receipt, Request, Snapshot, Task, TaskRun, TaskStatus, TaskStore, WorkPolicy},
    theme::{Record, RowStatus},
    worker::Progress,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    control: TaskControl,
    app: App,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn task(id: u64, title: &str, status: TaskStatus, repair_of: Option<u64>) -> Task {
    Task {
        id,
        title: title.into(),
        model: "qwen2.5-coder:14b".into(),
        dependencies: vec![],
        status: status.clone(),
        policy: Some(WorkPolicy {
            files: vec!["textutil.py".into()],
            check: vec!["python3".into(), "-c".into(), "import textutil".into()],
        }),
        repair_of,
        run: matches!(status, TaskStatus::Running | TaskStatus::Failed).then(|| TaskRun {
            id: format!("task-{id}-run-1"),
            baseline: "a".repeat(40),
            inputs: vec![],
            evidence_sha256: None,
            detail: "Recorded".into(),
        }),
    }
}

fn plan(prompt: &str, count: usize) -> Plan {
    Plan {
        prompt: prompt.into(),
        planner: "qwen2.5-coder:14b".into(),
        tasks: (0..count)
            .map(|index| Step {
                title: format!("Step {index}"),
                acceptance: vec!["Works".into()],
                model: "qwen2.5-coder:14b".into(),
                dependencies: vec![],
                policy: WorkPolicy {
                    files: vec!["textutil.py".into()],
                    check: vec!["true".into()],
                },
            })
            .collect(),
        context: None,
        scope: None,
        architecture: None,
    }
}

fn fixture(tasks: Vec<Task>, plans: Vec<(u64, Plan)>) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "alfredo-side-pane-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let store = TaskStore::new(&root.join("state"), &workspace, "default").unwrap();
    let mut control = TaskControl::new(store);
    let mut snapshot = Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace,
        mission: "default".into(),
        revision: 0,
        tasks,
        receipts: vec![],
    };
    for (first, plan) in plans {
        snapshot.revision += 1;
        snapshot.receipts.push(Receipt {
            task: first,
            revision: snapshot.revision,
            request: Request {
                correlation: format!("plan-{first}"),
                expected_revision: snapshot.revision - 1,
                action: Action::Plan { plan },
            },
        });
    }
    control.snapshot = Some(snapshot);
    control.visible = true;
    Fixture {
        root,
        control,
        app: App::new("qwen2.5-coder:14b".into()),
    }
}

fn textutil() -> Fixture {
    fixture(
        vec![
            task(1, "Create textutil", TaskStatus::Accepted, None),
            task(2, "Create tests", TaskStatus::Running, None),
            task(3, "Repair of #2", TaskStatus::Failed, Some(2)),
        ],
        vec![(1, plan("textutil module", 2))],
    )
}

fn live(
    control: &mut TaskControl,
    task: u64,
    stage: &'static str,
) -> tokio::sync::watch::Sender<Progress> {
    let (sender, receiver) = tokio::sync::watch::channel(Progress {
        stage,
        started: Instant::now() - Duration::from_secs(9),
        ..Default::default()
    });
    control.attach_progress(task, receiver, Arc::new(AtomicBool::new(false)));
    sender
}

#[test]
fn work_rows_are_the_mission_tree_with_status_icons_and_live_second_line() {
    let mut fixture = textutil();
    let _sender = live(&mut fixture.control, 2, "Running approved check");
    let projection = side_pane::project(&fixture.app, Some(&fixture.control), Instant::now());
    let rows: Vec<_> = projection
        .work
        .iter()
        .map(|row| (row.kind, row.status, row.depth, row.label.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            (RowKind::Group, None, 0, "textutil module"),
            (
                RowKind::Record(Record::Task),
                Some(RowStatus::Complete),
                1,
                "#1 Create textutil"
            ),
            (
                RowKind::Record(Record::Task),
                Some(RowStatus::Working),
                1,
                "#2 Create tests"
            ),
            (
                RowKind::Record(Record::Repair),
                Some(RowStatus::Failed),
                2,
                "#3 Repair of #2"
            ),
            (
                RowKind::Record(Record::Agent),
                Some(RowStatus::Idle),
                0,
                "chat 1"
            ),
        ]
    );
    assert_eq!(projection.work[0].right, "3");
    assert!(projection.work[0].expanded);
    let running = &projection.work[2];
    let second = running.second.as_deref().unwrap();
    assert!(second.starts_with("check"), "{second}");
    assert!(second.contains("qwen2.5-coder:14b"), "{second}");
    assert!(second.ends_with("0:09"), "{second}");
    assert_eq!(running.target, OpenTarget::Node(NodeId::Task(2)));
    assert!(projection.work[1].second.is_none());
    assert_eq!(projection.work[4].right, "ready");
    assert_eq!(projection.work[4].target, OpenTarget::Chat(0));
    assert!(projection.working());
    assert!(side_pane::any_working(&fixture.app, Some(&fixture.control)));
    assert_eq!(
        (
            projection.summary.done,
            projection.summary.total,
            projection.summary.repairs
        ),
        (1, 2, 1)
    );
}

#[test]
fn recorded_running_without_a_live_worker_is_not_shown_as_working() {
    let fixture = textutil();
    let projection = side_pane::project(&fixture.app, Some(&fixture.control), Instant::now());
    assert_eq!(projection.work[2].status, Some(RowStatus::Unverified));
    assert!(projection.work[2].second.is_none());
    assert!(!projection.working());
    assert!(!side_pane::any_working(
        &fixture.app,
        Some(&fixture.control)
    ));
}

#[test]
fn queued_worker_waits_for_model_capacity() {
    let mut fixture = textutil();
    let _sender = live(
        &mut fixture.control,
        2,
        "Waiting for shared Alfredo capacity",
    );
    let projection = side_pane::project(&fixture.app, Some(&fixture.control), Instant::now());
    assert_eq!(projection.work[2].status, Some(RowStatus::Queued));
    assert!(projection.work[2]
        .second
        .as_deref()
        .unwrap()
        .starts_with("queued"));
}

#[test]
fn architect_draft_row_sits_above_the_tree_and_chats_below() {
    let mut fixture = textutil();
    fixture.control.planner.draft = Some(plan("textutil module", 2));
    fixture.app.add_session();
    fixture.app.sessions[1].insert("hello");
    fixture.app.sessions[1].begin().unwrap();
    fixture.app.sessions[1].apply(1, Update::Token("partial".into()));
    let projection = side_pane::project(&fixture.app, Some(&fixture.control), Instant::now());
    let first = &projection.work[0];
    assert_eq!(first.key, RowKey::Architect);
    assert_eq!(first.kind, RowKind::Record(Record::Agent));
    assert_eq!(first.label, "architect");
    assert_eq!(first.right, "draft");
    assert_eq!(first.status, Some(RowStatus::Decision));
    let chats: Vec<_> = projection
        .work
        .iter()
        .filter(|row| matches!(row.key, RowKey::Chat(_)))
        .map(|row| (row.label.as_str(), row.status, row.right.as_str()))
        .collect();
    assert_eq!(
        chats,
        [
            ("chat 1", Some(RowStatus::Idle), "ready"),
            ("chat 2", Some(RowStatus::Working), "streaming"),
        ]
    );
    assert!(side_pane::any_working(&fixture.app, Some(&fixture.control)));
}

#[test]
fn completed_group_collapses_by_default_while_another_group_is_active() {
    let mut fixture = fixture(
        vec![
            task(1, "Old step", TaskStatus::Accepted, None),
            task(2, "New step", TaskStatus::Proposed, None),
        ],
        vec![(1, plan("old goal", 1)), (2, plan("new goal", 1))],
    );
    let labels = |fixture: &Fixture| -> Vec<String> {
        side_pane::project(&fixture.app, Some(&fixture.control), Instant::now())
            .work
            .iter()
            .map(|row| row.label.clone())
            .collect()
    };
    assert_eq!(
        labels(&fixture),
        ["old goal", "new goal", "#2 New step", "chat 1"]
    );
    // The user can still expand it explicitly.
    fixture
        .control
        .restore_view(TaskView {
            visible: true,
            selected: None,
            query: String::new(),
        })
        .unwrap();
    fixture.control.focus_node(NodeId::Plan(1));
    assert!(fixture.control.expand_work_node());
    assert_eq!(
        labels(&fixture),
        [
            "old goal",
            "#1 Old step",
            "new goal",
            "#2 New step",
            "chat 1"
        ]
    );
    // Without another active group, a finished group stays open.
    let single = self::fixture(
        vec![task(1, "Old step", TaskStatus::Accepted, None)],
        vec![(1, plan("old goal", 1))],
    );
    assert_eq!(labels(&single), ["old goal", "#1 Old step", "chat 1"]);
}

#[test]
fn group_title_is_the_goal_without_planner_retry_text() {
    let fixture = fixture(
        vec![task(1, "Check README", TaskStatus::Accepted, None)],
        vec![(
            1,
            plan(
                "i want to check docs | The previous plan was rejected by validation: Task 1 check must be an argv array. Return a corrected complete plan.",
                1,
            ),
        )],
    );
    let projection = side_pane::project(&fixture.app, Some(&fixture.control), Instant::now());
    assert_eq!(projection.work[0].label, "i want to check docs");
}

#[test]
fn missions_list_current_first_with_progress_then_others() {
    let mut fixture = textutil();
    fixture.app.pane.missions = vec![
        MissionEntry {
            name: "docs-cleanup".into(),
            progress: MissionProgress::Idle,
        },
        MissionEntry {
            name: "broken".into(),
            progress: MissionProgress::Unknown,
        },
        MissionEntry {
            name: "release".into(),
            progress: MissionProgress::State("running".into()),
        },
        MissionEntry {
            name: "todo-cli".into(),
            progress: MissionProgress::Counted {
                done: 4,
                total: 7,
                word: "running".into(),
            },
        },
        MissionEntry {
            name: "slugify".into(),
            progress: MissionProgress::Counted {
                done: 2,
                total: 3,
                word: "paused".into(),
            },
        },
    ];
    let projection = side_pane::project(&fixture.app, Some(&fixture.control), Instant::now());
    let missions: Vec<_> = projection
        .missions
        .iter()
        .map(|row| (row.name.as_str(), row.current, row.progress.as_str()))
        .collect();
    assert_eq!(
        missions,
        [
            ("default", true, "1/2"),
            ("docs-cleanup", false, "idle"),
            ("broken", false, "?"),
            ("release", false, "running"),
            ("todo-cli", false, "4/7   running"),
            ("slugify", false, "2/3   paused"),
        ]
    );
    assert_eq!(
        projection.missions[1].target,
        OpenTarget::Mission("docs-cleanup".into())
    );
    fixture.control.autopilot = Some(alfredo_tui::autopilot::Status {
        state: alfredo_tui::autopilot::RunState::Running,
        goal: "textutil".into(),
        done: 1,
        total: 2,
        failed: 0,
        repairs: 0,
        elapsed: Duration::from_secs(12),
        branch: None,
    });
    let projection = side_pane::project(&fixture.app, Some(&fixture.control), Instant::now());
    assert_eq!(projection.missions[0].progress, "1/2   00:12");
    // Without a task controller only chats are projected.
    let bare = side_pane::project(&fixture.app, None, Instant::now());
    assert!(bare.missions.is_empty());
    assert_eq!(bare.work.len(), 1);
}

#[test]
fn short_model_and_stage_words() {
    assert_eq!(side_pane::short_model("qwen2.5-coder:14b"), "qwen2.5-coder");
    assert_eq!(side_pane::short_model("llama3"), "llama3");
    for (stage, word) in [
        ("Running approved check", "check"),
        ("Receiving model plan", "generating"),
        ("Thinking", "thinking"),
        ("Waiting for shared Alfredo capacity", "queued"),
        ("Waiting for another Alfredo process", "queued"),
        ("Waiting for model server", "waiting"),
        ("Writing approved files", "writing"),
        ("Preparing worktree", "preparing"),
        ("Saving evidence and receipt", "saving"),
    ] {
        assert_eq!(side_pane::stage_word(stage), word, "{stage}");
    }
    assert_eq!(side_pane::elapsed(Duration::from_secs(9)), "0:09");
    assert_eq!(side_pane::elapsed(Duration::from_secs(754)), "12:34");
}

#[test]
fn pane_keys_move_switch_sections_open_and_return_to_the_prompt() {
    let mut fixture = textutil();
    fixture.app.pane.missions = vec![MissionEntry {
        name: "docs-cleanup".into(),
        progress: MissionProgress::Idle,
    }];
    fixture.app.sessions[0].insert("draft stays");
    let projection = side_pane::project(&fixture.app, Some(&fixture.control), Instant::now());
    let pane = &mut fixture.app.pane;
    pane.toggle_focus(false);
    assert_eq!(pane.focus, Some(Section::Work));
    assert!(!pane.overlay);
    // The cursor starts on the row shown in the right pane (#1).
    assert_eq!(
        pane.key(PaneKey::Down, &projection),
        PaneAction::Moved(RowKey::Node(NodeId::Task(2)))
    );
    assert_eq!(
        pane.key(PaneKey::Down, &projection),
        PaneAction::Moved(RowKey::Node(NodeId::Task(3)))
    );
    assert_eq!(
        pane.key(PaneKey::Up, &projection),
        PaneAction::Moved(RowKey::Node(NodeId::Task(2)))
    );
    assert_eq!(
        pane.key(PaneKey::Enter, &projection),
        PaneAction::Open(OpenTarget::Node(NodeId::Task(2)))
    );
    assert_eq!(pane.focus, None);
    pane.toggle_focus(true);
    assert!(pane.overlay);
    assert_eq!(pane.key(PaneKey::Tab, &projection), PaneAction::None);
    assert_eq!(pane.focus, Some(Section::Missions));
    assert_eq!(pane.key(PaneKey::Down, &projection), PaneAction::None);
    assert_eq!(
        pane.key(PaneKey::Enter, &projection),
        PaneAction::Open(OpenTarget::Mission("docs-cleanup".into()))
    );
    assert!(!pane.overlay);
    pane.toggle_focus(false);
    assert_eq!(pane.key(PaneKey::Esc, &projection), PaneAction::Exit);
    assert_eq!(pane.focus, None);
    pane.toggle_focus(false);
    pane.toggle_focus(false);
    assert_eq!(pane.focus, None);
    assert_eq!(fixture.app.sessions[0].draft, "draft stays");
}

#[test]
fn opening_rows_uses_agent_views_group_detail_and_chat() {
    use alfredo_tui::agent_view::Target;
    let mut fixture = textutil();
    fixture.app.add_session();
    fixture.app.selected = 0;
    fixture.control.set_visible(false);
    // A task opens the agent view of its repair lineage.
    side_pane::open_work_target(
        &mut fixture.app,
        &mut fixture.control,
        &OpenTarget::Node(NodeId::Task(2)),
    );
    assert!(fixture.control.visible);
    assert_eq!(fixture.control.selected_task().unwrap().id, 2);
    assert_eq!(
        fixture.control.agent_shown().map(|view| view.target),
        Some(Target::Task(2))
    );
    // A group still opens its detail.
    side_pane::open_work_target(
        &mut fixture.app,
        &mut fixture.control,
        &OpenTarget::Node(NodeId::Plan(1)),
    );
    assert!(fixture.control.agent.is_none());
    assert!(fixture.control.visible);
    side_pane::open_work_target(&mut fixture.app, &mut fixture.control, &OpenTarget::Chat(1));
    assert!(!fixture.control.visible);
    assert_eq!(fixture.app.selected, 1);
    fixture.control.planner.draft = Some(plan("textutil module", 2));
    side_pane::open_work_target(
        &mut fixture.app,
        &mut fixture.control,
        &OpenTarget::Architect,
    );
    assert!(fixture.control.visible);
    assert_eq!(
        fixture.control.agent_shown().map(|view| view.target),
        Some(Target::Architect)
    );
}

#[test]
fn other_missions_progress_comes_from_their_autopilot_state_read_only() {
    let root = std::env::temp_dir().join(format!(
        "alfredo-side-pane-missions-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let state = root.join("state");
    for name in [
        "default",
        "docs-cleanup",
        "release",
        "broken",
        "shipped",
        "todo-cli",
        "slugify",
    ] {
        let store = TaskStore::new(&state, &workspace, name).unwrap();
        store.select_mission(true).unwrap();
        alfredo_tui::missions::remember(&store, &workspace.canonicalize().unwrap(), name).unwrap();
    }
    let directory = |name: &str| {
        TaskStore::new(&state, &workspace, name)
            .unwrap()
            .conversation_directory()
            .unwrap()
    };
    let release = alfredo_tui::autopilot::state_path(&directory("release"), "default");
    fs::write(
        &release,
        serde_json::json!({
            "version": 1, "id": "0123456789abcdef", "goal": "ship", "model": "m",
            "max_repairs": 3, "phase": "running", "paused": false, "started": 1
        })
        .to_string(),
    )
    .unwrap();
    let before = fs::read(&release).unwrap();
    // A finished loop reports its outcome from the saved report.
    fs::write(
        alfredo_tui::autopilot::state_path(&directory("shipped"), "default"),
        serde_json::json!({
            "version": 1, "id": "0123456789abcdef", "goal": "ship", "model": "m",
            "max_repairs": 3, "phase": "done", "paused": false, "started": 1,
            "finished": 2, "report": "Autopilot partial: ship"
        })
        .to_string(),
    )
    .unwrap();
    // Counts saved by the other mission's own autopilot; done/total plus state.
    for (name, paused, done, total) in [("todo-cli", false, 4, 7), ("slugify", true, 2, 3)] {
        fs::write(
            alfredo_tui::autopilot::state_path(&directory(name), "default"),
            serde_json::json!({
                "version": 1, "id": "0123456789abcdef", "goal": "ship", "model": "m",
                "max_repairs": 3, "phase": "running", "paused": paused, "started": 1,
                "done": done, "total": total
            })
            .to_string(),
        )
        .unwrap();
    }
    let counted = alfredo_tui::autopilot::state_path(&directory("todo-cli"), "default");
    let counted_before = fs::read(&counted).unwrap();
    fs::write(
        alfredo_tui::autopilot::state_path(&directory("broken"), "default"),
        "not json",
    )
    .unwrap();
    let entries = side_pane::load_missions(
        &state,
        &workspace.canonicalize().unwrap(),
        "default",
        "default",
    );
    let found: Vec<_> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.progress.summary()))
        .collect();
    assert_eq!(
        found,
        [
            ("broken", "?".to_string()),
            ("docs-cleanup", "idle".into()),
            ("release", "running".into()),
            ("shipped", "partial".into()),
            ("slugify", "2/3   paused".into()),
            ("todo-cli", "4/7   running".into()),
        ]
    );
    // A state file written before counts existed shows the state word alone.
    assert_eq!(
        entries[2].progress,
        MissionProgress::State("running".into())
    );
    assert_eq!(fs::read(&release).unwrap(), before);
    assert_eq!(fs::read(&counted).unwrap(), counted_before);
    fs::remove_dir_all(root).unwrap();
}
