use alfredo_tui::{
    activity::{self, Entry},
    assessment::{Decision, FailureKind, Outcome},
    planner::{Plan, Step},
    tasks::{Action, Receipt, Request, Snapshot, Task, TaskRun, TaskStatus, TaskStore, WorkPolicy},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    store: TaskStore,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-activity-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("workspace")).unwrap();
        let store =
            TaskStore::new(&root.join("state"), &root.join("workspace"), "activity").unwrap();
        Self { root, store }
    }

    fn action(&self, correlation: &str, action: Action) -> Snapshot {
        self.store
            .transact(Request {
                correlation: correlation.into(),
                expected_revision: self.store.snapshot().unwrap().revision,
                action,
            })
            .unwrap()
            .0
    }

    fn path(&self) -> PathBuf {
        self.store
            .conversation_directory()
            .unwrap()
            .join("tasks.json")
    }

    fn validate_history(&self, snapshot: &Snapshot) -> Snapshot {
        let bytes = serde_json::to_vec(snapshot).unwrap();
        assert!(bytes.len() < 4 * 1024 * 1024);
        fs::write(self.path(), &bytes).unwrap();
        let loaded = self.store.snapshot().unwrap();
        assert_eq!(serde_json::to_vec(&loaded).unwrap(), bytes);
        loaded
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn policy() -> WorkPolicy {
    WorkPolicy {
        files: vec!["source.rs".into()],
        check: vec!["true".into()],
    }
}

fn proposal(title: &str) -> Action {
    Action::Propose {
        title: title.into(),
        model: "fixture".into(),
        dependencies: vec![],
    }
}

fn fields(entries: &[Entry]) -> Vec<(u64, u64, &str, &str, &str)> {
    entries
        .iter()
        .map(|entry| {
            (
                entry.revision,
                entry.task,
                entry.correlation.as_str(),
                entry.summary.as_str(),
                entry.detail.as_str(),
            )
        })
        .collect()
}

fn assert_prefixes(snapshot: &Snapshot, query: &str) {
    let all = activity::entries(snapshot, query);
    for limit in [0, 1, 3, all.len(), all.len() + 1, usize::MAX] {
        let recent: Vec<_> = activity::iter_entries(snapshot, query)
            .take(limit)
            .collect();
        assert_eq!(
            fields(&recent),
            fields(&all[..limit.min(all.len())]),
            "query {query:?}, limit {limit}"
        );
    }
}

fn assert_revisions(snapshot: &Snapshot, query: &str, revisions: &[u64]) {
    assert_eq!(
        activity::entries(snapshot, query)
            .iter()
            .map(|entry| entry.revision)
            .collect::<Vec<_>>(),
        revisions,
        "query {query:?}"
    );
    assert_prefixes(snapshot, query);
}

#[test]
fn plan_membership_search_and_interleaved_history_keep_exact_entry_prefixes() {
    let fixture = Fixture::new();
    fixture.action("manual", proposal("Background literal #3 marker"));
    fixture.action(
        "plan-identity",
        Action::Plan {
            plan: Plan {
                prompt: "Keep #draft input text".into(),
                planner: "architect".into(),
                context: None,
                scope: None,
                architecture: None,
                tasks: ["Plan first member", "Æble parser", "Leaf-only title"]
                    .into_iter()
                    .map(|title| Step {
                        title: title.into(),
                        acceptance: vec!["Preserve required behavior".into()],
                        model: "fixture".into(),
                        dependencies: vec![],
                        policy: policy(),
                    })
                    .collect(),
            },
        },
    );
    fixture.action("other-proposal", proposal("Other task"));
    fixture.action(
        "assign-member",
        Action::Assign {
            task: 3,
            model: "second-worker".into(),
        },
    );
    fixture.action(
        "noise-#18446744073709551616",
        Action::Assign {
            task: 1,
            model: "noise".into(),
        },
    );
    fixture.action(
        "permit-member",
        Action::Permit {
            task: 3,
            policy: policy(),
        },
    );
    fixture.action("approve-member", Action::Approve { task: 3 });
    let snapshot = fixture.action(
        "latest-unrelated",
        Action::Assign {
            task: 5,
            model: "noise".into(),
        },
    );
    let original = fs::read(fixture.path()).unwrap();

    for (query, revisions) in [
        ("", &[8, 7, 6, 5, 4, 3, 2, 1][..]),
        ("#1", &[5, 1]),
        ("#2", &[2]),
        ("#3", &[7, 6, 4, 2]),
        ("#03", &[7, 6, 4, 2]),
        ("#4", &[2]),
        ("#5", &[8, 3]),
        ("#999", &[]),
        ("APPROVE-MEMBER", &[7]),
        ("PLAN-IDENTITY", &[2]),
        ("æBLE", &[7, 6, 4]),
        ("second-WORKER", &[4]),
        ("approved", &[7]),
        ("#draft", &[2]),
        ("#18446744073709551616", &[5]),
        ("#3 ", &[5, 1]),
        // Free-text lookup uses the receipt's canonical task title, even when
        // another member's exact task filter includes that Plan receipt.
        ("Leaf-only title", &[]),
    ] {
        assert_revisions(&snapshot, query, revisions);
    }
    let member = activity::entries(&snapshot, "#3");
    assert_eq!(
        fields(&member),
        vec![
            (
                7,
                3,
                "approve-member",
                "Task approved",
                "Approval does not mean execution has started"
            ),
            (
                6,
                3,
                "permit-member",
                "Policy set · approval required",
                "Files [\"source.rs\"] · check [\"true\"]"
            ),
            (
                4,
                3,
                "assign-member",
                "Worker assigned",
                "Local Agent model second-worker · fresh approval required"
            ),
            (
                2,
                2,
                "plan-identity",
                "Plan proposed · 3 tasks",
                "Frontier Architect architect · Keep #draft input text · tasks require approval"
            ),
        ]
    );
    assert_eq!(fs::read(fixture.path()).unwrap(), original);
    assert_eq!(
        serde_json::to_vec(&fixture.store.snapshot().unwrap()).unwrap(),
        serde_json::to_vec(&snapshot).unwrap()
    );
}

fn record(snapshot: &mut Snapshot, correlation: &str, task: u64, action: Action) {
    let expected_revision = snapshot.revision;
    snapshot.revision += 1;
    snapshot.receipts.push(Receipt {
        revision: snapshot.revision,
        task,
        request: Request {
            correlation: correlation.into(),
            expected_revision,
            action,
        },
    });
}

// These histories use synthetic worker digests and never execute work. Every
// receipt and final task is checked by the real store's full replay validator.
fn complete(snapshot: &mut Snapshot, task: u64, status: TaskStatus) {
    record(
        snapshot,
        &format!("approve-{task}"),
        task,
        Action::Approve { task },
    );
    let run = format!("task-{task}-run-{}", snapshot.revision + 1);
    record(
        snapshot,
        &format!("start-{task}"),
        task,
        Action::Start {
            task,
            baseline: "a".repeat(40),
            inputs: vec![],
        },
    );
    record(
        snapshot,
        &format!("finish-{task}"),
        task,
        Action::Finish {
            task,
            run: run.clone(),
            status: status.clone(),
            evidence_sha256: "b".repeat(64),
            detail: "Retained fixture outcome".into(),
        },
    );
    let item = &mut snapshot.tasks[task as usize - 1];
    item.status = status;
    item.run = Some(TaskRun {
        id: run,
        baseline: "a".repeat(40),
        inputs: vec![],
        evidence_sha256: Some("b".repeat(64)),
        detail: "Retained fixture outcome".into(),
    });
}

fn decision(outcome: Outcome, architecture: bool) -> Decision {
    Decision {
        outcome,
        reason: "Revise parsing boundary".into(),
        criteria: vec![],
        limitations: vec![],
        risk: None,
        failure: architecture.then_some(FailureKind::Architecture),
    }
}

fn routed_repair(snapshot: &mut Snapshot, task: u64, architecture: bool) {
    let child = snapshot.tasks.len() as u64 + 1;
    let action = if architecture {
        Action::ReviewArchitecture {
            task,
            decision: decision(Outcome::NeedsRepair, true),
        }
    } else {
        Action::ReviewAndRepair {
            task,
            decision: decision(Outcome::NeedsRepair, false),
        }
    };
    record(snapshot, &format!("route-{task}"), child, action);
    let parent = &mut snapshot.tasks[task as usize - 1];
    parent.status = TaskStatus::Rejected;
    let repair = Task {
        id: child,
        title: format!("Repair #{task}: Revise parsing boundary"),
        model: parent.model.clone(),
        dependencies: parent.dependencies.clone(),
        status: TaskStatus::Proposed,
        policy: parent.policy.clone(),
        run: None,
        repair_of: Some(task),
    };
    snapshot.tasks.push(repair);
}

fn noise(snapshot: &mut Snapshot) {
    record(
        snapshot,
        &format!("noise-{}", snapshot.revision + 1),
        2,
        Action::Assign {
            task: 2,
            model: "fixture".into(),
        },
    );
}

fn repair_history(fixture: &Fixture, resolved: bool) -> Snapshot {
    fixture.action("original", proposal("Initial parsing"));
    fixture.action(
        "original-policy",
        Action::Permit {
            task: 1,
            policy: policy(),
        },
    );
    let mut snapshot = fixture.action("unrelated", proposal("Independent task"));
    complete(&mut snapshot, 1, TaskStatus::Failed);
    routed_repair(&mut snapshot, 1, false);
    noise(&mut snapshot);
    complete(&mut snapshot, 3, TaskStatus::Failed);
    routed_repair(&mut snapshot, 3, true);
    noise(&mut snapshot);
    complete(&mut snapshot, 4, TaskStatus::ReviewReady);
    if resolved {
        record(
            &mut snapshot,
            "accepted-repair",
            4,
            Action::Decide {
                task: 4,
                decision: decision(Outcome::Approved, false),
            },
        );
        snapshot.tasks[3].status = TaskStatus::Accepted;
        noise(&mut snapshot);
        record(
            &mut snapshot,
            "resolution",
            4,
            Action::ResolveRepair { task: 4 },
        );
    } else {
        record(
            &mut snapshot,
            "architect-required",
            4,
            Action::ReviewArchitecture {
                task: 4,
                decision: decision(Outcome::NeedsRepair, true),
            },
        );
        snapshot.tasks[3].status = TaskStatus::Rejected;
    }
    noise(&mut snapshot);
    fixture.validate_history(&snapshot)
}

#[test]
fn compound_reviews_and_resolution_ancestors_keep_canonical_receipt_identity() {
    let fixture = Fixture::new();
    let snapshot = repair_history(&fixture, true);
    let before = fs::read(fixture.path()).unwrap();
    assert_eq!(snapshot.resolved_by(1), Some(4));
    assert_eq!(snapshot.resolved_by(3), Some(4));
    assert_eq!(snapshot.resolved_by(2), None);
    for (query, revisions) in [
        ("#1", &[19, 7, 6, 5, 4, 2, 1][..]),
        ("#3", &[19, 12, 11, 10, 9, 7]),
        ("#4", &[19, 17, 16, 15, 14, 12]),
        ("#2", &[20, 18, 13, 8, 3]),
        ("resolution", &[19]),
        ("architecture", &[12]),
    ] {
        assert_revisions(&snapshot, query, revisions);
    }
    let original = activity::entries(&snapshot, "#1");
    assert_eq!(original[0].task, 4);
    assert_eq!(original[0].correlation, "resolution");
    assert_eq!(original[0].summary, "Repair resolution recorded");
    assert_eq!(
        original[0].detail,
        "Accepted repair #4 supplies its unsuccessful ancestors for future dependencies"
    );
    assert_eq!(original[1].task, 3);
    assert_eq!(original[1].correlation, "route-1");
    assert_eq!(
        original[1].summary,
        "Review #1: Needs repair · repair #3 proposed"
    );
    assert!(original[1]
        .detail
        .ends_with("Repair #3 inherits policy; fresh approval required"));
    let parent = activity::entries(&snapshot, "#3");
    assert_eq!(parent[1].task, 4);
    assert_eq!(
        parent[1].summary,
        "Architecture review · repair #4 proposed"
    );
    assert_eq!(fs::read(fixture.path()).unwrap(), before);
}

#[test]
fn architecture_escalation_stays_on_its_source_after_interleaved_receipts() {
    let fixture = Fixture::new();
    let snapshot = repair_history(&fixture, false);
    assert!(snapshot.architecture_required(4));
    assert_eq!(snapshot.tasks.len(), 4);
    assert_revisions(&snapshot, "#4", &[17, 16, 15, 14, 12]);
    assert_revisions(&snapshot, "#3", &[12, 11, 10, 9, 7]);
    assert_revisions(&snapshot, "#1", &[7, 6, 5, 4, 2, 1]);
    let newest = activity::entries(&snapshot, "#4");
    assert_eq!(newest[0].task, 4);
    assert_eq!(newest[0].correlation, "architect-required");
    assert_eq!(newest[0].summary, "Architect revision required · #4");
}

#[test]
fn maximum_receipt_history_preserves_recent_prefix_and_complete_activity() {
    let fixture = Fixture::new();
    fixture.action("proposal", proposal("Bounded history"));
    let mut snapshot = fixture.action(
        "policy",
        Action::Permit {
            task: 1,
            policy: policy(),
        },
    );
    for revision in 3..=4096 {
        record(
            &mut snapshot,
            &format!("policy-{revision}"),
            1,
            Action::Permit {
                task: 1,
                policy: policy(),
            },
        );
    }
    let snapshot = fixture.validate_history(&snapshot);
    let before = fs::read(fixture.path()).unwrap();
    assert_eq!(snapshot.receipts.len(), 4096);
    assert_prefixes(&snapshot, "#1");
    assert_prefixes(&snapshot, "");
    assert_revisions(&snapshot, "proposal", &[1]);
    assert_revisions(&snapshot, "#2", &[]);
    let all = activity::entries(&snapshot, "#1");
    assert_eq!(all.len(), 4096);
    assert_eq!(all[0].revision, 4096);
    assert_eq!(all[0].correlation, "policy-4096");
    assert_eq!(all[4095].revision, 1);
    assert_eq!(all[4095].summary, "Task proposed");
    assert_eq!(fs::read(fixture.path()).unwrap(), before);
}
