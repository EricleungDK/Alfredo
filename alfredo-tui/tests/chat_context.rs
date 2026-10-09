use alfredo_tui::{
    chat_context,
    planner::{Plan, Step},
    tasks::{Action, Receipt, Request, Snapshot, Task, TaskRun, TaskStatus, WorkPolicy},
    worker::Evidence,
};

fn policy() -> WorkPolicy {
    WorkPolicy {
        files: vec!["app/utils.py".into()],
        check: vec!["python3".into(), "-m".into(), "pytest".into()],
    }
}

fn snapshot() -> Snapshot {
    let plan = Plan {
        prompt: "check the repo if there is any dead code, remove them".into(),
        planner: "qwen2.5-coder:14b".into(),
        tasks: vec![Step {
            title: "Remove Dead Code".into(),
            acceptance: vec![],
            model: "qwen2.5-coder:14b".into(),
            dependencies: vec![],
            policy: policy(),
        }],
        context: None,
        scope: None,
        architecture: None,
    };
    Snapshot {
        schema_version: 8,
        workspace: "/repo".into(),
        mission: "default".into(),
        revision: 4,
        tasks: vec![Task {
            id: 2,
            title: "Remove Dead Code".into(),
            model: "qwen2.5-coder:14b".into(),
            dependencies: vec![],
            status: TaskStatus::Accepted,
            policy: Some(policy()),
            run: Some(TaskRun {
                id: "run-2".into(),
                baseline: "a".repeat(40),
                inputs: vec![],
                evidence_sha256: Some("b".repeat(64)),
                detail: "check passed".into(),
            }),
            repair_of: None,
        }],
        receipts: vec![Receipt {
            request: Request {
                correlation: "c".into(),
                expected_revision: 0,
                action: Action::Plan { plan },
            },
            revision: 1,
            task: 2,
        }],
    }
}

fn evidence(patch: &str) -> String {
    serde_json::to_string(&Evidence {
        agent: None,
        candidate_commit: None,
        model_metrics: None,
        failure_code: None,
        generation: None,
        run: "run-2".into(),
        baseline: "a".repeat(40),
        status: TaskStatus::ReviewReady,
        detail: "check passed".into(),
        patch: patch.into(),
        check: None,
    })
    .unwrap()
}

#[test]
fn chat_knows_it_is_the_alfredo_harness_without_tasks() {
    let message = chat_context::system_message(None, |_| None);
    assert_eq!(message.role, "system");
    assert!(message.content.contains("Alfredo"), "{}", message.content);
    assert!(message.content.contains("No tasks"), "{}", message.content);
}

#[test]
fn chat_sees_goal_task_status_files_and_the_retained_patch() {
    let patch = "diff --git a/app/utils.py b/app/utils.py\n-def unused_helper():\n-    pass\n";
    let message =
        chat_context::system_message(Some(&snapshot()), |id| (id == 2).then(|| evidence(patch)));
    let text = message.content;
    for needle in [
        "check the repo if there is any dead code",
        "#2",
        "Remove Dead Code",
        "accepted",
        "app/utils.py",
        "python3 -m pytest",
        "-def unused_helper():",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
}

#[test]
fn unreadable_evidence_is_stated_not_invented() {
    let message = chat_context::system_message(Some(&snapshot()), |_| None);
    assert!(message.content.contains("#2"));
    assert!(
        message.content.contains("patch unavailable"),
        "{}",
        message.content
    );
}

#[test]
fn context_stays_bounded_for_huge_patches_and_many_tasks() {
    let mut many = snapshot();
    let template = many.tasks[0].clone();
    many.tasks = (1..=40)
        .map(|id| Task {
            id,
            ..template.clone()
        })
        .collect();
    let huge = format!("diff --git a/x b/x\n{}", "-dead line\n".repeat(20_000));
    let message = chat_context::system_message(Some(&many), |_| Some(evidence(&huge)));
    assert!(
        message.content.len() <= chat_context::MAX_CONTEXT,
        "{}",
        message.content.len()
    );
    // Newest tasks win the budget.
    assert!(message.content.contains("#40"));
    assert!(message.content.contains("truncated"));
}
