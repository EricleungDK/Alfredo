//! Read-only fixtures exercise projection without granting executable authority.
use alfredo_tui::{
    assessment::{Decision, FailureKind, Outcome},
    mission_work::{project, NodeId, Row, Tree},
    planner::{Plan, Step},
    task_view::ScopeStatus,
    tasks::{Action, Receipt, Request, Snapshot, Task, TaskRun, TaskStatus, WorkPolicy},
};
use std::{collections::BTreeSet, path::PathBuf};

fn task(id: u64, title: &str, dependencies: &[u64], repair_of: Option<u64>) -> Task {
    Task {
        id,
        title: title.into(),
        model: "fixture-worker".into(),
        dependencies: dependencies.into(),
        status: TaskStatus::Proposed,
        policy: Some(WorkPolicy {
            files: vec!["source.rs".into()],
            check: vec!["true".into()],
        }),
        run: None,
        repair_of,
    }
}

fn snapshot(tasks: Vec<Task>) -> Snapshot {
    Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace: PathBuf::from("/projection-fixture"),
        mission: "Mission Work fixture".into(),
        revision: 0,
        tasks,
        receipts: vec![],
    }
}

fn receipt(snapshot: &mut Snapshot, revision: u64, task: u64, action: Action) {
    snapshot.receipts.push(Receipt {
        request: Request {
            correlation: format!("projection-{revision}"),
            expected_revision: revision - 1,
            action,
        },
        revision,
        task,
    });
    snapshot.revision = revision;
}

fn plan(snapshot: &mut Snapshot, revision: u64, first: u64, count: usize, prompt: &str) {
    let tasks = snapshot
        .tasks
        .iter()
        .filter(|task| task.id >= first)
        .take(count)
        .map(|task| Step {
            title: task.title.clone(),
            model: task.model.clone(),
            acceptance: vec![],
            dependencies: task.dependencies.clone(),
            policy: task.policy.clone().unwrap(),
        })
        .collect();
    receipt(
        snapshot,
        revision,
        first,
        Action::Plan {
            plan: Plan {
                prompt: prompt.into(),
                planner: "fixture-planner".into(),
                tasks,
                context: None,
                scope: None,
                architecture: None,
            },
        },
    );
}

fn scope() -> ScopeStatus {
    ScopeStatus {
        revision: Some(0),
        blocked: false,
        label: "Scope observed".into(),
    }
}

fn ids(tree: &Tree) -> Vec<NodeId> {
    tree.rows.iter().map(|row| row.id).collect()
}

fn row(tree: &Tree, id: NodeId) -> &Row {
    tree.rows.iter().find(|row| row.id == id).unwrap()
}

fn diamond() -> Snapshot {
    let mut state = snapshot(vec![
        task(1, "Shared input", &[], None),
        task(2, "Left branch", &[1], None),
        task(3, "Right branch", &[1], None),
        task(4, "Diamond consumer", &[2, 3], None),
        task(5, "Manual investigation", &[], None),
        task(6, "Repair left", &[1], Some(2)),
        task(7, "Needle repair", &[1], Some(6)),
    ]);
    plan(&mut state, 11, 1, 4, "Deliver the planned feature");
    state
}

#[test]
fn plan_diamond_and_manual_repair_family_each_appear_once() {
    let state = diamond();
    let before = serde_json::to_vec(&state).unwrap();
    let tree = project(&state, &scope(), "", &BTreeSet::new());
    assert_eq!(
        ids(&tree),
        vec![
            NodeId::Plan(11),
            NodeId::Task(1),
            NodeId::Task(2),
            NodeId::Task(6),
            NodeId::Task(7),
            NodeId::Task(3),
            NodeId::Task(4),
            NodeId::Manual,
            NodeId::Task(5),
        ]
    );
    assert_eq!((tree.total_tasks, tree.matched_tasks), (7, 7));
    let group = row(&tree, NodeId::Plan(11));
    assert_eq!(group.task_count, 6);
    assert_eq!(group.status, "6 tasks");
    assert!(group.label.contains("Deliver the planned feature"));
    assert!(group.detail.contains("4 original steps"));
    assert_eq!(row(&tree, NodeId::Manual).task_count, 1);
    assert_eq!(row(&tree, NodeId::Task(2)).task_count, 3);
    assert_eq!(row(&tree, NodeId::Task(7)).depth, 3);
    for (id, edges) in [(2, "#1"), (3, "#1"), (4, "#2, #3")] {
        let item = row(&tree, NodeId::Task(id));
        assert_eq!(item.parent, Some(NodeId::Plan(11)));
        assert!(item.detail.contains(&format!("Dependencies: {edges}")));
    }
    assert_eq!(serde_json::to_vec(&state).unwrap(), before);
}

#[test]
fn collapse_and_search_retain_ancestors_without_changing_task_truth() {
    let state = diamond();
    let before = serde_json::to_vec(&state).unwrap();
    let collapsed = BTreeSet::from([NodeId::Plan(11), NodeId::Task(2), NodeId::Task(6)]);
    let closed = project(&state, &scope(), "", &collapsed);
    assert_eq!(
        ids(&closed),
        vec![NodeId::Plan(11), NodeId::Manual, NodeId::Task(5)]
    );
    assert_eq!((closed.total_tasks, closed.matched_tasks), (7, 7));
    assert!(!row(&closed, NodeId::Plan(11)).expanded);

    let filtered = project(&state, &scope(), " needle ", &collapsed);
    assert_eq!(
        ids(&filtered),
        vec![
            NodeId::Plan(11),
            NodeId::Task(2),
            NodeId::Task(6),
            NodeId::Task(7)
        ]
    );
    assert_eq!((filtered.total_tasks, filtered.matched_tasks), (7, 1));
    for ancestor in &filtered.rows[..3] {
        assert!(!ancestor.matches_filter);
        assert!(ancestor.expandable && ancestor.expanded);
    }
    assert!(row(&filtered, NodeId::Task(7)).matches_filter);
    assert_eq!(row(&filtered, NodeId::Plan(11)).task_count, 6);
    assert_eq!(
        ids(&project(&state, &scope(), "#7", &collapsed)),
        ids(&filtered)
    );
    assert!(project(&state, &scope(), "unknown-query", &collapsed)
        .rows
        .is_empty());
    assert_eq!(serde_json::to_vec(&state).unwrap(), before);
}

#[test]
fn accepted_repair_replacement_preserves_original_lifecycle_and_dependency_edge() {
    let mut state = snapshot(vec![
        task(1, "Original input", &[], None),
        task(2, "Dependent work", &[1], None),
        task(3, "Fixed input", &[], Some(1)),
    ]);
    state.tasks[0].status = TaskStatus::Rejected;
    state.tasks[1].status = TaskStatus::Approved;
    state.tasks[2].status = TaskStatus::Accepted;
    receipt(&mut state, 1, 3, Action::ResolveRepair { task: 3 });
    let tree = project(&state, &scope(), "", &BTreeSet::new());
    let original = row(&tree, NodeId::Task(1));
    assert_eq!(original.status, "Rejected");
    assert!(original.detail.contains("Resolved by accepted repair #3"));
    let dependent = row(&tree, NodeId::Task(2));
    assert_eq!(dependent.parent, Some(NodeId::Manual));
    assert!(dependent.detail.contains("Dependencies: #1"));
    assert!(dependent.detail.contains("#1 via repair #3"));
    assert!(!dependent.detail.contains("blocked by"));
    assert_eq!(row(&tree, NodeId::Task(3)).parent, Some(NodeId::Task(1)));
    assert_eq!(tree.rows.iter().filter(|row| row.task.is_some()).count(), 3);

    state.tasks[1].status = TaskStatus::Accepted;
    let completed = project(&state, &scope(), "", &BTreeSet::new());
    assert!(row(&completed, NodeId::Task(2))
        .detail
        .contains("Dependencies: #1 via repair #3"));
}

#[test]
fn readiness_reuses_scope_approval_and_human_hold_truth() {
    let mut state = snapshot(vec![
        task(1, "Awaiting approval", &[], None),
        task(2, "Held input", &[], None),
        task(3, "Blocked dependent", &[2], None),
        task(4, "Recorded run", &[], None),
    ]);
    state.tasks[1].status = TaskStatus::NeedsHumanReview;
    state.tasks[2].status = TaskStatus::Approved;
    state.tasks[3].status = TaskStatus::Running;
    let unobserved = ScopeStatus::loading();
    let tree = project(&state, &unobserved, "", &BTreeSet::new());
    for task in &state.tasks {
        assert!(row(&tree, NodeId::Task(task.id))
            .detail
            .contains(&unobserved.readiness(&state, task)));
    }
    assert!(row(&tree, NodeId::Task(1))
        .detail
        .contains("Needs approval"));
    assert!(row(&tree, NodeId::Task(3))
        .detail
        .contains("blocked by #2 (Needs human review)"));
    assert!(row(&tree, NodeId::Task(4))
        .detail
        .contains("Run claimed; inspect progress or recovery evidence"));
    assert_eq!(row(&tree, NodeId::Manual).status, "4 tasks");

    plan(&mut state, 3, 1, 1, "Original scope");
    let changed = ScopeStatus {
        revision: Some(9),
        ..scope()
    };
    let changed_tree = project(&state, &changed, "scope changed", &BTreeSet::new());
    assert_eq!(changed_tree.matched_tasks, 1);
    assert!(row(&changed_tree, NodeId::Task(1))
        .detail
        .contains("Plan scope changed"));
}

#[test]
fn plan_identity_survives_refresh_and_identical_prompts() {
    let mut state = snapshot(vec![
        task(1, "First plan", &[], None),
        task(2, "Second plan", &[], None),
    ]);
    plan(&mut state, 4, 1, 1, "Same prompt");
    plan(&mut state, 9, 2, 1, "Same prompt");
    let before = project(&state, &scope(), "", &BTreeSet::new());
    state.tasks[0].title = "Updated visible title".into();
    state.tasks[0].model = "another-worker".into();
    receipt(&mut state, 10, 1, Action::Approve { task: 1 });
    let refreshed = project(&state, &scope(), "", &BTreeSet::new());
    assert_eq!(ids(&before), ids(&refreshed));
    assert_eq!(
        ids(&refreshed),
        vec![
            NodeId::Plan(4),
            NodeId::Task(1),
            NodeId::Plan(9),
            NodeId::Task(2)
        ]
    );
}

#[test]
fn adopted_architect_revision_remains_in_original_repair_family() {
    let mut state = snapshot(vec![
        task(1, "Original work", &[], None),
        task(2, "First repair", &[], Some(1)),
    ]);
    plan(&mut state, 1, 1, 1, "Original delivery");
    for id in 1..=2 {
        let task = &mut state.tasks[(id - 1) as usize];
        task.status = TaskStatus::Rejected;
        task.run = Some(TaskRun {
            id: format!("fixture-run-{id}"),
            baseline: "a".repeat(40),
            inputs: vec![],
            evidence_sha256: Some("b".repeat(64)),
            detail: "Rejected architecture".into(),
        });
        receipt(
            &mut state,
            id + 1,
            id,
            Action::ReviewArchitecture {
                task: id,
                decision: Decision {
                    outcome: Outcome::NeedsRepair,
                    reason: "Revise architecture".into(),
                    criteria: vec![],
                    limitations: vec![],
                    risk: None,
                    failure: Some(FailureKind::Architecture),
                },
            },
        );
    }
    let pending = project(&state, &scope(), "", &BTreeSet::new());
    assert!(row(&pending, NodeId::Task(2))
        .detail
        .contains("Frontier Architect revision required"));
    let origin = state.architecture_origin(2).unwrap();
    state
        .tasks
        .push(task(3, "Revised architecture", &[], Some(2)));
    plan(&mut state, 4, 3, 1, "Architect revision");
    if let Action::Plan { plan } = &mut state.receipts.last_mut().unwrap().request.action {
        plan.architecture = Some(origin);
    }
    let adopted = project(&state, &scope(), "", &BTreeSet::new());
    assert_eq!(
        ids(&adopted),
        vec![
            NodeId::Plan(1),
            NodeId::Task(1),
            NodeId::Task(2),
            NodeId::Task(3)
        ]
    );
    assert_eq!(row(&adopted, NodeId::Plan(1)).task_count, 3);
    assert!(row(&adopted, NodeId::Task(3))
        .detail
        .contains("Architect Plan r4"));
    assert!(row(&adopted, NodeId::Task(1))
        .detail
        .contains("Superseded by adopted Architect revision"));
}

#[test]
fn deep_repair_tree_is_iterative_and_bounds_multibyte_row_text() {
    let mut state = snapshot(
        (1..=256)
            .map(|id| task(id, "Repair", &[], (id > 1).then_some(id - 1)))
            .collect(),
    );
    state.tasks[255].title = "界".repeat(2700);
    let collapsed = BTreeSet::from([NodeId::Manual, NodeId::Task(128)]);
    let tree = project(&state, &scope(), "#256", &collapsed);
    assert_eq!((tree.total_tasks, tree.matched_tasks), (256, 1));
    assert_eq!(tree.rows.len(), 257);
    assert_eq!(row(&tree, NodeId::Task(256)).depth, 256);
    assert_eq!(row(&tree, NodeId::Manual).task_count, 256);
    assert_eq!(row(&tree, NodeId::Task(128)).task_count, 129);
    let leaf = row(&tree, NodeId::Task(256));
    assert!(leaf.label.len() <= 512);
    assert!(leaf.label.ends_with('…'));
    assert!(tree.rows.iter().all(|row| row.detail.len() <= 8192));
}

#[test]
fn empty_snapshot_has_no_fabricated_groups_or_workers() {
    let tree = project(&snapshot(vec![]), &scope(), "", &BTreeSet::new());
    assert_eq!(tree, Tree::default());
}
