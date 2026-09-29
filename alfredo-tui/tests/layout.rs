use alfredo_tui::{
    model::{App, Update},
    ui,
};
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn terminal_renders_at_wide_narrow_and_tiny_sizes_with_unicode() {
    let mut app = App::new("qwen3:14b".into());
    app.sessions[0].insert("Explain 🦀 中文\nsecond line");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("wide界 🦀\n".repeat(200)));
    for (width, height) in [(140, 40), (88, 24), (60, 18), (32, 10), (20, 5), (1, 1)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        assert_eq!(terminal.backend().buffer().area.width, width);
        if width >= 32 {
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("ALFREDO"));
            assert!(text.contains("Prompt"));
            // Wide: the side pane lists chats; narrow: one summary row with F6.
            // Tiny heights give every body row to the transcript.
            if width >= 88 {
                // A streaming chat shows its spinner between icon and label.
                assert!(text.contains("◈ ") && text.contains(" chat 1 "), "{text}");
            } else if height >= 12 {
                assert!(text.contains("F6 pane"), "{text}");
            }
        }
    }
}

#[test]
fn provider_controls_cannot_inject_terminal_escape_sequences() {
    let mut app = App::new("test".into());
    app.sessions[0].insert("prompt");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("safe\x1b[2J\x07answer".into()));
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!text.contains('\x1b'));
    assert!(!text.contains('\x07'));
    assert!(text.contains("answer"));
}

#[test]
fn model_catalog_has_readable_content_at_minimum_size() {
    let mut app = App::new("test".into());
    app.models_visible = true;
    app.models_notice = "Catalog ready".into();
    let mut terminal = Terminal::new(TestBackend::new(32, 10)).unwrap();
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Catalog ready"), "{text}");
    assert!(text.contains("Prompt"));
}

#[test]
fn chat_keeps_background_work_visible_without_moving_draft_or_reading_position() {
    use alfredo_tui::{
        task_control::TaskControl,
        tasks::{Snapshot, Task, TaskStatus, TaskStore},
    };
    use std::sync::{atomic::AtomicBool, Arc};
    for (width, height) in [(100, 24), (32, 10)] {
        let root =
            std::env::temp_dir().join(format!("alfredo-layout-{}-{width}", std::process::id()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        // Rendering needs only an in-memory projection, never a real task journal.
        let store = TaskStore::new(&root.join("state"), &workspace, "layout").unwrap();
        let mut tasks = TaskControl::new(store);
        tasks.visible = false;
        tasks.snapshot = Some(Snapshot {
            schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
            workspace,
            mission: "layout".into(),
            revision: 0,
            tasks: vec![],
            receipts: vec![],
        });
        let mut app = App::new("chat-fixture".into());
        app.sessions[0].insert("Earlier question");
        app.sessions[0].begin().unwrap();
        app.sessions[0].apply(1, Update::Token("Earlier answer line\n".repeat(80)));
        app.sessions[0].apply(1, Update::Done);
        app.sessions[0].insert("Preserve my unsent draft 🦀");
        app.sessions[0].scroll_rows(-12);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &app, &tasks))
            .unwrap();
        let session_before = serde_json::to_value(&app.sessions[0]).unwrap();
        let selected_before = app.selected;
        tasks.snapshot.as_mut().unwrap().tasks = [
            TaskStatus::Running,
            TaskStatus::ReviewReady,
            TaskStatus::NeedsHumanReview,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, status)| Task {
            id: index as u64 + 1,
            title: format!("Background task {index}"),
            model: "worker-fixture".into(),
            dependencies: vec![],
            status,
            policy: None,
            run: None,
            repair_of: None,
        })
        .collect();
        tasks.workers.insert(1, Arc::new(AtomicBool::new(false)));
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &app, &tasks))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let header: String = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(header.contains("ALFREDO"), "{header}");
        let work = tasks.work_status();
        assert_eq!(work.workers, 1);
        assert_eq!(work.review, 1);
        assert_eq!(work.held, 1);
        assert_eq!(work.attention(), 2);
        // Attention items only, separated by spaces; no zero-value worker counts.
        if width >= 70 {
            assert!(header.contains("   1 review   1 decision"), "{header}");
            assert!(!header.contains("Work 1 local"), "{header}");
        } else {
            // 32x10 has no summary row; the header keeps the mission name.
            assert!(header.contains("layout"), "{header}");
        }
        assert!(text.contains("F1 help"), "{text}");
        assert!(text.contains("Prompt"));
        assert!(!tasks.visible);
        assert_eq!(app.selected, selected_before);
        assert_eq!(
            serde_json::to_value(&app.sessions[0]).unwrap(),
            session_before
        );
        // Completing a local worker updates awareness without changing the chat view.
        tasks.workers.clear();
        tasks.snapshot.as_mut().unwrap().tasks[0].status = TaskStatus::ReviewReady;
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &app, &tasks))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let header: String = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
        assert_eq!(tasks.work_status().workers, 0);
        assert_eq!(tasks.work_status().attention(), 3);
        if width >= 70 {
            assert!(header.contains("   2 review   1 decision"), "{header}");
            assert!(!header.contains("Work 0 local"), "{header}");
        } else {
            assert!(header.contains("ALFREDO"), "{header}");
        }
        assert_eq!(
            serde_json::to_value(&app.sessions[0]).unwrap(),
            session_before
        );
        assert!(!tasks.visible);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn console_interleaves_exact_task_receipts_without_feeding_them_to_model() {
    use alfredo_tui::{
        model::TaskReceiptRef,
        task_control::TaskControl,
        tasks::{Action, Request, Snapshot, TaskStore},
    };
    let root =
        std::env::temp_dir().join(format!("alfredo-console-receipts-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let store = TaskStore::new(&root.join("state"), &workspace, "console").unwrap();
    let mut tasks = TaskControl::new(store);
    tasks.visible = false;
    tasks.snapshot = Some(Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace,
        mission: "console".into(),
        revision: 1,
        tasks: vec![],
        receipts: vec![alfredo_tui::tasks::Receipt {
            revision: 1,
            task: 7,
            request: Request {
                correlation: "exact-proposal".into(),
                expected_revision: 0,
                action: Action::Propose {
                    title: "Never send to model".into(),
                    model: "worker".into(),
                    dependencies: vec![],
                },
            },
        }],
    });
    let mut app = App::new("chat".into());
    app.sessions[0].insert("FIRST_QUESTION");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("FIRST_ANSWER".into()));
    app.sessions[0].apply(1, Update::Done);
    assert!(app.sessions[0].observe_task_receipt(TaskReceiptRef {
        sequence: 0,
        after_messages: 2,
        revision: 1,
        task: 7,
        correlation: "exact-proposal".into(),
    }));
    app.sessions[0].insert("SECOND_QUESTION");
    let request = app.sessions[0].begin().unwrap();
    assert_eq!(request.len(), 3);
    assert!(request.iter().all(
        |m| !m.content.contains("exact-proposal") && !m.content.contains("Never send to model")
    ));
    app.sessions[0].apply(2, Update::Token("SECOND_ANSWER".into()));
    app.sessions[0].apply(2, Update::Done);
    app.sessions[0].insert("Preserve unsent draft");
    let render = |app: &App, tasks: &TaskControl| {
        let mut terminal = Terminal::new(TestBackend::new(140, 32)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, app, tasks))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let before = app.sessions[0].messages.clone();
    let text = render(&app, &tasks);
    assert!(text.find("FIRST_ANSWER").unwrap() < text.find("✓ Task #7 proposed").unwrap());
    assert!(text.find("✓ Task #7 proposed").unwrap() < text.find("SECOND_QUESTION").unwrap());
    // Chat shows the short line; revision and correlation are F4 activity detail.
    assert!(!text.contains("exact-proposal"), "{text}");
    assert!(!text.contains("revision 1"), "{text}");
    tasks.visible = true;
    tasks.activity = Some(String::new());
    let activity = render(&app, &tasks);
    assert!(activity.contains("r1 · task #7"), "{activity}");
    assert!(activity.contains("exact-proposal"), "{activity}");
    tasks.visible = false;
    tasks.activity = None;
    assert_eq!(app.sessions[0].messages, before);
    assert_eq!(app.sessions[0].draft, "Preserve unsent draft");
    tasks.snapshot.as_mut().unwrap().receipts[0].request.action = Action::Decide {
        task: 7,
        decision: alfredo_tui::assessment::Decision {
            outcome: alfredo_tui::assessment::Outcome::NeedsRepair,
            risk: Some(alfredo_tui::assessment::ReviewRisk::Security),
            failure: None,
            reason: "Review risk".into(),
            criteria: vec![],
            limitations: vec![],
        },
    };
    let text = render(&app, &tasks);
    assert!(
        text.contains("Task #7 held · Security risk needs human review"),
        "{text}"
    );
    assert!(!text.contains("review: Needs repair"));
    for mismatch in 0..3 {
        let receipt = &mut tasks.snapshot.as_mut().unwrap().receipts[0];
        receipt.revision = if mismatch == 0 { 2 } else { 1 };
        receipt.task = if mismatch == 1 { 8 } else { 7 };
        receipt.request.correlation = if mismatch == 2 {
            "different"
        } else {
            "exact-proposal"
        }
        .into();
        let text = render(&app, &tasks);
        assert!(text.contains("? Task #7 update not verified"), "{text}");
        assert!(!text.contains("Task #7 proposed"));
    }
    tasks.snapshot = None;
    assert!(render(&app, &tasks).contains("? Task #7 update not verified"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn observed_receipt_append_preserves_history_reading_and_unsent_draft() {
    use alfredo_tui::model::TaskReceiptRef;
    let mut app = App::new("chat".into());
    app.sessions[0].insert("Earlier question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(
        1,
        Update::Token((0..80).map(|n| format!("HISTORY_{n:03}\n")).collect()),
    );
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Unsent draft 🦀");
    let render = |app: &App, width| {
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        text.find("HISTORY_")
            .map(|index| text[index..index + 11].to_owned())
    };
    render(&app, 100);
    app.sessions[0].scroll_rows(-20);
    let before = render(&app, 100);
    assert!(before.is_some());
    let messages = app.sessions[0].messages.clone();
    assert!(app.sessions[0].observe_task_receipt(TaskReceiptRef {
        sequence: 0,
        after_messages: 2,
        revision: 1,
        task: 7,
        correlation: "new-observation".into(),
    }));
    assert_eq!(render(&app, 100), before);
    assert_eq!(render(&app, 60), before);
    assert_eq!(app.sessions[0].messages, messages);
    assert_eq!(app.sessions[0].draft, "Unsent draft 🦀");
}

#[test]
fn command_receipt_binding_preserves_origin_and_never_enters_inference() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::CommandState,
        model::TaskReceiptRef,
        task_control::TaskControl,
        tasks::{Action, Receipt, Request, Snapshot, TaskStore},
    };
    let root = std::env::temp_dir().join(format!("alfredo-command-layout-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let store = TaskStore::new(&root.join("state"), &workspace, "commands").unwrap();
    let mut tasks = TaskControl::new(store);
    tasks.visible = false;
    tasks.snapshot = Some(Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace,
        mission: "commands".into(),
        revision: 0,
        tasks: vec![],
        receipts: vec![],
    });
    let request = Request {
        correlation: "command-exact".into(),
        expected_revision: 0,
        action: Action::Propose {
            title: "Origin work".into(),
            model: "worker".into(),
            dependencies: vec![],
        },
    };
    let mut app = App::new("chat".into());
    let command_id = app.sessions[0]
        .submit_command(
            "/task Origin work".into(),
            Intent::Task {
                request: request.clone(),
            },
        )
        .unwrap();
    app.sessions[0].insert("Keep newer draft 🦀");
    let render = |app: &App, tasks: &TaskControl| {
        let mut terminal = Terminal::new(TestBackend::new(140, 32)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, app, tasks))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let pending = render(&app, &tasks);
    assert!(pending.contains("/task Origin work"));
    assert!(pending.contains("Pending · saving intent"));
    assert!(!pending.contains("Task #7 proposed"));
    for (state, label) in [
        (
            CommandState::Submitted,
            "Submitted · awaiting acknowledgment",
        ),
        (
            CommandState::Unknown {
                reason: "Interrupted".into(),
            },
            "Outcome unconfirmed · Interrupted",
        ),
        (
            CommandState::Refused {
                reason: "Intent save failed".into(),
            },
            "Not dispatched · Intent save failed",
        ),
    ] {
        assert!(app.sessions[0].set_command_state(&command_id, state));
        let text = render(&app, &tasks);
        assert!(text.contains(label));
        assert!(!text.contains("Task #7 proposed"));
    }
    let original = serde_json::to_value(&app.sessions[0]).unwrap();
    app.add_session();
    let other = render(&app, &tasks);
    assert!(!other.contains("/task Origin work"));
    tasks.snapshot.as_mut().unwrap().receipts.push(Receipt {
        request: request.clone(),
        revision: 1,
        task: 7,
    });
    tasks.snapshot.as_mut().unwrap().revision = 1;
    assert!(!render(&app, &tasks).contains("Task #7 proposed"));
    app.selected = 0;
    let acknowledged = render(&app, &tasks);
    assert!(
        acknowledged.contains("✓ Task #7 proposed"),
        "{acknowledged}"
    );
    // The correlation is F4 activity detail, not chat text.
    assert!(!acknowledged.contains("command-exact"));
    tasks.visible = true;
    tasks.activity = Some("#7".into());
    assert!(render(&app, &tasks).contains("command-exact"));
    tasks.visible = false;
    tasks.activity = None;
    assert!(!acknowledged.contains("Pending · saving intent"));
    assert_eq!(serde_json::to_value(&app.sessions[0]).unwrap(), original);
    tasks.snapshot.as_mut().unwrap().receipts[0].request.action = Action::Propose {
        title: "Different request under same correlation".into(),
        model: "worker".into(),
        dependencies: vec![],
    };
    assert!(!render(&app, &tasks).contains("Task #7 proposed"));
    assert!(app.sessions[0].observe_task_receipt(TaskReceiptRef {
        sequence: 0,
        after_messages: 0,
        revision: 1,
        task: 7,
        correlation: "command-exact".into(),
    }));
    let ordered = render(&app, &tasks);
    assert!(
        ordered.find("/task Origin work").unwrap() < ordered.find("✓ Task #7 proposed").unwrap()
    );
    let messages = app.sessions[0].begin().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Keep newer draft 🦀");
    assert!(messages
        .iter()
        .all(|message| !message.content.contains("Origin work")));
    app.sessions[0].apply(1, Update::Done);
    let risk_request = Request {
        correlation: "risk-review".into(),
        expected_revision: 1,
        action: Action::Decide {
            task: 7,
            decision: alfredo_tui::assessment::Decision {
                failure: None,
                risk: Some(alfredo_tui::assessment::ReviewRisk::Security),
                outcome: alfredo_tui::assessment::Outcome::NeedsRepair,
                reason: "Unresolved security finding".into(),
                criteria: vec![],
                limitations: vec![],
            },
        },
    };
    app.sessions[0]
        .submit_command(
            "/review 7 risk decision".into(),
            Intent::Task {
                request: risk_request.clone(),
            },
        )
        .unwrap();
    tasks.snapshot.as_mut().unwrap().receipts.push(Receipt {
        request: risk_request,
        revision: 2,
        task: 7,
    });
    tasks.snapshot.as_mut().unwrap().revision = 2;
    let risk = render(&app, &tasks);
    assert!(
        risk.contains("Task #7 held · Security risk needs human review"),
        "{risk}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn worker_command_keeps_claim_and_result_at_origin_without_moving_later_reading() {
    use alfredo_tui::{
        command_intent::Intent,
        task_control::TaskControl,
        tasks::{Action, Receipt, Request, Snapshot, Task, TaskRun, TaskStatus, TaskStore},
    };
    let root = std::env::temp_dir().join(format!("alfredo-worker-phases-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut tasks =
        TaskControl::new(TaskStore::new(&root.join("state"), &workspace, "phases").unwrap());
    tasks.visible = false;
    tasks.snapshot = Some(Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace,
        mission: "phases".into(),
        revision: 0,
        tasks: vec![],
        receipts: vec![],
    });
    let mut app = App::new("chat".into());
    app.sessions[0]
        .submit_command(
            "/run 7".into(),
            Intent::Run {
                correlation: "worker-origin".into(),
                expected_revision: 0,
                task: 7,
            },
        )
        .unwrap();
    let render = |app: &App, tasks: &TaskControl, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, app, tasks))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let pending = render(&app, &tasks, 140, 32);
    assert!(pending.contains("Pending · saving intent"));
    assert!(pending.contains("no result yet"));
    assert!(!pending.contains("Task #7 started"));
    let snapshot = tasks.snapshot.as_mut().unwrap();
    snapshot.revision = 1;
    snapshot.receipts.push(Receipt {
        revision: 1,
        task: 7,
        request: Request {
            correlation: "worker-origin".into(),
            expected_revision: 0,
            action: Action::Start {
                task: 7,
                baseline: "a".repeat(40),
                inputs: vec![],
            },
        },
    });
    snapshot.tasks.push(Task {
        id: 7,
        title: "Isolated work".into(),
        model: "worker".into(),
        dependencies: vec![],
        status: TaskStatus::Running,
        policy: None,
        repair_of: None,
        run: Some(TaskRun {
            id: "task-7-run-1".into(),
            baseline: "a".repeat(40),
            inputs: vec![],
            evidence_sha256: None,
            detail: "".into(),
        }),
    });
    let claimed = render(&app, &tasks, 140, 32);
    assert!(claimed.contains("✓ Task #7 started"), "{claimed}");
    assert!(claimed.contains("no result yet"));
    app.add_session();
    for status in [
        TaskStatus::ReviewReady,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ] {
        let snapshot = tasks.snapshot.as_mut().unwrap();
        snapshot.receipts.truncate(1);
        snapshot.revision = 2;
        snapshot.tasks[0].status = status.clone();
        snapshot.tasks[0].run.as_mut().unwrap().evidence_sha256 = Some("b".repeat(64));
        snapshot.receipts.push(Receipt {
            revision: 2,
            task: 7,
            request: Request {
                correlation: "finish:task-7-run-1".into(),
                expected_revision: 1,
                action: Action::Finish {
                    task: 7,
                    run: "task-7-run-1".into(),
                    status: status.clone(),
                    evidence_sha256: "b".repeat(64),
                    detail: "Retained result".into(),
                },
            },
        });
        let finished = match status {
            TaskStatus::ReviewReady => "✓ Task #7 check passed · awaiting review",
            TaskStatus::Failed => "✗ Task #7 failed",
            _ => "– Task #7 run cancelled",
        };
        assert!(!render(&app, &tasks, 140, 32).contains(finished));
        app.selected = 0;
        let result = render(&app, &tasks, 140, 32);
        assert!(result.contains("✓ Task #7 started"), "{result}");
        assert!(result.contains(finished), "{result}");
        assert!(!result.contains("no result yet"));
        assert!(!result.contains("Task #7 accepted"));
        app.selected = 1;
    }
    app.selected = 0;
    app.sessions[0].insert("Later chat");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(
        1,
        Update::Token((0..70).map(|n| format!("LATER_{n:03}\n")).collect()),
    );
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Unsent after worker 🦀");
    render(&app, &tasks, 80, 24);
    app.sessions[0].scroll_rows(-20);
    let before = render(&app, &tasks, 80, 24);
    let first_marker = |text: &str| {
        text.find("LATER_")
            .map(|index| text[index..index + 9].to_owned())
    };
    let marker = first_marker(&before);
    assert!(marker.is_some());
    tasks.snapshot.as_mut().unwrap().receipts.truncate(1);
    assert_eq!(first_marker(&render(&app, &tasks, 80, 24)), marker);
    assert_eq!(first_marker(&render(&app, &tasks, 60, 25)), marker);
    assert_eq!(app.sessions[0].draft, "Unsent after worker 🦀");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn planner_outcomes_are_originating_drafts_and_plan_save_is_separate_authority() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::CommandState,
        planner_command::{Operation, Outcome, Request as PlannerRequest},
        task_control::TaskControl,
        tasks::{Action, Receipt, Request, Snapshot, TaskStore},
    };
    let root = std::env::temp_dir().join(format!("alfredo-planner-layout-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut tasks =
        TaskControl::new(TaskStore::new(&root.join("state"), &workspace, "planner").unwrap());
    tasks.visible = false;
    tasks.snapshot = Some(Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace,
        mission: "planner".into(),
        revision: 0,
        tasks: vec![],
        receipts: vec![],
    });
    let mut app = App::new("chat".into());
    let id = app.sessions[0]
        .submit_command(
            "/plan A small change".into(),
            Intent::Planner {
                request: PlannerRequest {
                    correlation: "generation-origin".into(),
                    operation: Operation::Generate {
                        prompt: "A small change".into(),
                        model: "architect".into(),
                        revision: 0,
                        base_sha256: None,
                    },
                },
            },
        )
        .unwrap();
    let render = |app: &App, tasks: &TaskControl, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, app, tasks))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    assert!(render(&app, &tasks, 140, 32).contains("Submitted · planner operation pending"));
    app.add_session();
    app.sessions[0].set_command_state(
        &id,
        CommandState::Planner {
            outcome: Outcome::Generated {
                draft_sha256: "a".repeat(64),
                tasks: 1,
            },
        },
    );
    assert!(!render(&app, &tasks, 140, 32).contains("Draft generated"));
    app.selected = 0;
    for (width, height) in [(140, 32), (60, 24)] {
        let generated = render(&app, &tasks, width, height);
        assert!(generated.contains("Draft generated"), "{generated}");
        assert!(!generated.contains("Task #1 approved"));
        assert!(!generated.contains("Plan saved"));
    }
    let save = Request {
        correlation: "save-plan".into(),
        expected_revision: 0,
        action: Action::Plan {
            plan: alfredo_tui::planner::Plan {
                prompt: "A small change".into(),
                planner: "architect".into(),
                context: None,
                scope: None,
                architecture: None,
                tasks: vec![alfredo_tui::planner::Step {
                    title: "Change file".into(),
                    model: "worker".into(),
                    dependencies: vec![],
                    acceptance: vec!["Check passes".into()],
                    policy: alfredo_tui::tasks::WorkPolicy {
                        files: vec!["file.txt".into()],
                        check: vec!["true".into()],
                    },
                }],
            },
        },
    };
    app.sessions[0]
        .submit_command(
            "/plan-save".into(),
            Intent::Task {
                request: save.clone(),
            },
        )
        .unwrap();
    tasks.snapshot.as_mut().unwrap().receipts.push(Receipt {
        request: save,
        revision: 1,
        task: 1,
    });
    tasks.snapshot.as_mut().unwrap().revision = 1;
    let saved = render(&app, &tasks, 140, 32);
    assert!(saved.contains("Draft generated · 1 step · saving and approval are separate"));
    assert!(saved.contains("✓ Plan saved · task #1"), "{saved}");
    assert!(!saved.contains("Task #1 approved"));
    app.sessions[0].insert("Later discussion");
    let request = app.sessions[0].begin().unwrap();
    assert_eq!(request.len(), 1);
    app.sessions[0].apply(
        1,
        Update::Token(
            (0..70)
                .map(|n| format!("PLANNER_HISTORY_{n:03}\n"))
                .collect(),
        ),
    );
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Keep newer draft 🦀");
    render(&app, &tasks, 100, 24);
    app.sessions[0].scroll_rows(-20);
    let first_marker = |text: String| {
        text.find("PLANNER_HISTORY_")
            .map(|index| text[index..index + 19].to_owned())
    };
    let before = first_marker(render(&app, &tasks, 100, 24));
    assert!(before.is_some());
    app.sessions[0].set_command_state(
        &id,
        CommandState::Planner {
            outcome: Outcome::Failed {
                reason: "Generation interrupted with bounded diagnostic detail ".repeat(20),
            },
        },
    );
    assert_eq!(first_marker(render(&app, &tasks, 100, 24)), before);
    assert_eq!(first_marker(render(&app, &tasks, 60, 25)), before);
    assert_eq!(app.sessions[0].draft, "Keep newer draft 🦀");
    // Inspect only the command so failure and cancellation remain legible at origin.
    app.sessions[0].scroll_rows(-1000);
    let failed = render(&app, &tasks, 140, 32);
    assert!(failed.contains("Draft generation failed"));
    app.sessions[0].set_command_state(
        &id,
        CommandState::Planner {
            outcome: Outcome::Stopped,
        },
    );
    let stopped = render(&app, &tasks, 140, 32);
    assert!(stopped.contains("Draft generation stopped"));
    app.add_session();
    let selected = app.selected;
    let cancel = app.sessions[selected]
        .submit_command(
            "/plan-cancel".into(),
            Intent::Planner {
                request: PlannerRequest {
                    correlation: "discard-draft".into(),
                    operation: Operation::Cancel {
                        generation: None,
                        draft_sha256: Some("a".repeat(64)),
                    },
                },
            },
        )
        .unwrap();
    app.sessions[selected].set_command_state(
        &cancel,
        CommandState::Planner {
            outcome: Outcome::Stopped,
        },
    );
    let discarded = render(&app, &tasks, 100, 24);
    assert!(discarded.contains("Draft discarded"));
    assert!(!discarded.contains("Plan saved"));
    assert!(!discarded.contains("Draft generation stopped"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_request_stays_at_origin_and_does_not_overwrite_actual_worker_result() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::CommandState,
        control_command::{Operation, Outcome, Request as ControlRequest},
        task_control::TaskControl,
        tasks::{Action, Receipt, Request, Snapshot, Task, TaskRun, TaskStatus, TaskStore},
    };
    let root = std::env::temp_dir().join(format!("alfredo-cancel-layout-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut tasks =
        TaskControl::new(TaskStore::new(&root.join("state"), &workspace, "cancel").unwrap());
    tasks.visible = false;
    tasks.snapshot = Some(Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace,
        mission: "cancel".into(),
        revision: 1,
        tasks: vec![Task {
            id: 7,
            title: "Work already in progress".into(),
            model: "worker".into(),
            dependencies: vec![],
            status: TaskStatus::Running,
            policy: None,
            repair_of: None,
            run: Some(TaskRun {
                id: "task-7-run-1".into(),
                baseline: "a".repeat(40),
                inputs: vec![],
                evidence_sha256: None,
                detail: "".into(),
            }),
        }],
        receipts: vec![Receipt {
            revision: 1,
            task: 7,
            request: Request {
                correlation: "worker-origin".into(),
                expected_revision: 0,
                action: Action::Start {
                    task: 7,
                    baseline: "a".repeat(40),
                    inputs: vec![],
                },
            },
        }],
    });
    let mut app = App::new("chat".into());
    let id = app.sessions[0]
        .submit_command(
            "/cancel-task 7".into(),
            Intent::Control {
                request: ControlRequest {
                    correlation: "cancel-worker".into(),
                    controller: "origin-controller".into(),
                    operation: Operation::CancelWorker {
                        task: 7,
                        start_correlation: "worker-origin".into(),
                        expected_start_revision: 1,
                    },
                },
            },
        )
        .unwrap();
    let render = |app: &App, tasks: &TaskControl, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, app, tasks))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let pending = render(&app, &tasks, 140, 32);
    assert!(pending.contains("Pending · saving intent"));
    assert!(pending.contains("no result yet"));
    assert!(!pending.contains("Task #7 started"));
    assert!(!pending.contains("Cancellation requested"));
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    assert!(render(&app, &tasks, 140, 32).contains("Submitted · controller operation pending"));
    app.add_session();
    app.sessions[0].set_command_state(
        &id,
        CommandState::Control {
            outcome: Outcome::CancellationRequested,
        },
    );
    assert!(!render(&app, &tasks, 140, 32).contains("Cancellation requested"));
    app.selected = 0;
    // Neither a local cancellation outcome nor changed task status proves Finish.
    tasks.snapshot.as_mut().unwrap().tasks[0].status = TaskStatus::Cancelled;
    for (width, height) in [(140, 32), (60, 24)] {
        let requested = render(&app, &tasks, width, height);
        assert!(requested.contains("Cancellation requested"), "{requested}");
        assert!(requested.contains("no result yet"), "{requested}");
        assert!(!requested.contains("Task #7 run cancelled"));
        assert!(!requested.contains("Task #7 started"));
    }
    // A late successful result remains truthful even after cancellation was requested.
    let snapshot = tasks.snapshot.as_mut().unwrap();
    snapshot.revision = 2;
    snapshot.tasks[0].status = TaskStatus::ReviewReady;
    snapshot.tasks[0].run.as_mut().unwrap().evidence_sha256 = Some("b".repeat(64));
    snapshot.receipts.push(Receipt {
        revision: 2,
        task: 7,
        request: Request {
            correlation: "finish:task-7-run-1".into(),
            expected_revision: 1,
            action: Action::Finish {
                task: 7,
                run: "task-7-run-1".into(),
                status: TaskStatus::ReviewReady,
                evidence_sha256: "b".repeat(64),
                detail: "Completed before cancellation took effect".into(),
            },
        },
    });
    for (state, primary) in [
        (
            CommandState::Control {
                outcome: Outcome::CancellationRequested,
            },
            "Cancellation requested",
        ),
        (
            CommandState::Unknown {
                reason: "Controller interrupted".into(),
            },
            "Outcome unconfirmed",
        ),
        (
            CommandState::Refused {
                reason: "Worker owner changed".into(),
            },
            "Not dispatched",
        ),
    ] {
        app.sessions[0].set_command_state(&id, state);
        let result = render(&app, &tasks, 140, 32);
        assert!(result.contains(primary), "{result}");
        assert!(
            result.contains("✓ Task #7 check passed · awaiting review"),
            "{result}"
        );
        assert!(!result.contains("Task #7 run cancelled"));
        assert!(!result.contains("Task #7 started"));
    }
    app.sessions[0].insert("Later chat");
    assert_eq!(app.sessions[0].begin().unwrap().len(), 1);
    app.sessions[0].apply(
        1,
        Update::Token(
            (0..70)
                .map(|n| format!("CANCEL_HISTORY_{n:03}\n"))
                .collect(),
        ),
    );
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Keep current draft 🦀");
    render(&app, &tasks, 80, 24);
    app.sessions[0].scroll_rows(-20);
    let marker = |text: String| {
        text.find("CANCEL_HISTORY_")
            .map(|index| text[index..index + 18].to_owned())
    };
    let before = marker(render(&app, &tasks, 80, 24));
    assert!(before.is_some());
    tasks.snapshot.as_mut().unwrap().receipts.truncate(1);
    app.sessions[0].set_command_state(
        &id,
        CommandState::Unknown {
            reason: "Long interrupted controller detail ".repeat(6),
        },
    );
    assert_eq!(marker(render(&app, &tasks, 80, 24)), before);
    assert_eq!(marker(render(&app, &tasks, 60, 25)), before);
    assert_eq!(app.sessions[0].draft, "Keep current draft 🦀");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn restored_dispatch_outcome_is_historical_while_current_dispatch_stays_off() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::CommandState,
        control_command::{Operation, Outcome, Request},
        conversations,
        task_control::TaskControl,
        tasks::TaskStore,
    };
    let root = std::env::temp_dir().join(format!("alfredo-dispatch-layout-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let tasks =
        TaskControl::new(TaskStore::new(&root.join("state"), &workspace, "dispatch").unwrap());
    let mut app = App::new("chat".into());
    let id = app.sessions[0]
        .submit_command(
            "/dispatch on".into(),
            Intent::Control {
                request: Request {
                    correlation: "enable-old-controller".into(),
                    controller: "previous-controller".into(),
                    operation: Operation::Dispatch {
                        enabled: true,
                        expected_epoch: 0,
                        scope_revision_for_on: Some(1),
                    },
                },
            },
        )
        .unwrap();
    app.sessions[0].set_command_state(
        &id,
        CommandState::Control {
            outcome: Outcome::DispatchChanged { enabled: true },
        },
    );
    let snapshot = conversations::Snapshot::capture(&app, "dispatch-test");
    let restored = snapshot.restore();
    assert!(!tasks.dispatch.enabled);
    for (width, height) in [(140, 32), (60, 24)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &restored, &tasks))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("✓ Dispatch on"), "{text}");
        // Current dispatch is off: the header names dispatch only while it is on.
        let header: String = text.chars().take(usize::from(width)).collect();
        assert!(header.contains("ALFREDO"), "{header}");
        assert!(!header.contains("dispatch on"), "{header}");
        assert!(!text.contains("Task #"));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_launch_has_dispatch_actor_and_keeps_origin_reading_and_drafts() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::CommandState,
        control_command::{Operation, Outcome, Request as ControlRequest},
        conversations,
        dispatch::RunRequest,
        task_control::TaskControl,
        tasks::{Action, Receipt, Request, Snapshot, Task, TaskRun, TaskStatus, TaskStore},
    };
    let root =
        std::env::temp_dir().join(format!("alfredo-automatic-layout-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut tasks =
        TaskControl::new(TaskStore::new(&root.join("state"), &workspace, "automatic").unwrap());
    tasks.visible = false;
    tasks.snapshot = Some(Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace,
        mission: "automatic".into(),
        revision: 1,
        tasks: vec![Task {
            id: 7,
            title: "Automatic isolated work".into(),
            model: "worker".into(),
            dependencies: vec![],
            status: TaskStatus::Approved,
            policy: None,
            repair_of: None,
            run: None,
        }],
        receipts: vec![Receipt {
            revision: 1,
            task: 7,
            request: Request {
                correlation: "approved-worker".into(),
                expected_revision: 0,
                action: Action::Approve { task: 7 },
            },
        }],
    });
    let render = |app: &App, tasks: &TaskControl, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, app, tasks))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let mut app = App::new("chat".into());
    let source = ControlRequest {
        correlation: "enable-automatic".into(),
        controller: "origin-controller".into(),
        operation: Operation::Dispatch {
            enabled: true,
            expected_epoch: 0,
            scope_revision_for_on: Some(1),
        },
    };
    let parent = app.sessions[0]
        .submit_command(
            "/dispatch on".into(),
            Intent::Control {
                request: source.clone(),
            },
        )
        .unwrap();
    app.sessions[0].set_command_state(
        &parent,
        CommandState::Control {
            outcome: Outcome::DispatchChanged { enabled: true },
        },
    );
    app.sessions[0].insert("Origin unsent 🦀");
    app.add_session();
    app.sessions[1].insert("Selected unsent 中文");
    app.sessions[0]
        .submit_automatic_command(
            "Automatic /run 7 · dispatch command #1".into(),
            Intent::DispatchRun {
                request: RunRequest {
                    correlation: "automatic-origin".into(),
                    expected_revision: 1,
                    task: 7,
                    approval_revision: 1,
                    source: source.clone(),
                },
            },
        )
        .unwrap();
    assert_eq!(app.selected, 1);
    assert!(!render(&app, &tasks, 140, 32).contains("Dispatch · run task #7"));
    assert_eq!(app.sessions[0].draft, "Origin unsent 🦀");
    assert_eq!(app.sessions[1].draft, "Selected unsent 中文");
    assert!(app.sessions[0].messages.is_empty());
    app.selected = 0;
    for (width, height) in [(140, 32), (60, 24)] {
        let pending = render(&app, &tasks, width, height);
        // The dispatch actor heads its own entry below the originating command.
        let origin = pending.find("› /dispatch on").expect(&pending);
        let launch = pending.find("▶ Dispatch · run task #7").expect(&pending);
        assert!(origin < launch, "{pending}");
        assert!(!pending.contains("› Automatic /run"), "{pending}");
        assert!(pending.contains("Pending · saving intent"), "{pending}");
        assert!(pending.contains("no result yet"), "{pending}");
        assert!(!pending.contains("Task #7 started"), "{pending}");
    }
    let restored = conversations::Snapshot::capture(&app, "automatic-test").restore();
    let unknown = render(&restored, &tasks, 140, 32);
    assert!(unknown.contains("▶ Dispatch · run task #7"), "{unknown}");
    assert!(unknown.contains("Outcome unconfirmed"), "{unknown}");
    // Current dispatch stays off: the header names dispatch only while it is on.
    let header: String = unknown.chars().take(140).collect();
    assert!(header.contains("ALFREDO"), "{header}");
    assert!(!header.contains("dispatch on"), "{header}");
    assert!(!unknown.contains("Task #7 started"), "{unknown}");
    assert!(tasks.workers.is_empty());
    let snapshot = tasks.snapshot.as_mut().unwrap();
    snapshot.revision = 2;
    snapshot.receipts.push(Receipt {
        revision: 2,
        task: 7,
        request: Request {
            correlation: "automatic-origin".into(),
            expected_revision: 1,
            action: Action::Start {
                task: 7,
                baseline: "a".repeat(40),
                inputs: vec![],
            },
        },
    });
    snapshot.tasks[0].status = TaskStatus::Running;
    snapshot.tasks[0].run = Some(TaskRun {
        id: "task-7-run-2".into(),
        baseline: "a".repeat(40),
        inputs: vec![],
        evidence_sha256: None,
        detail: String::new(),
    });
    let claimed = render(&app, &tasks, 140, 32);
    assert!(claimed.contains("✓ Task #7 started"), "{claimed}");
    assert!(claimed.contains("no result yet"), "{claimed}");
    for status in [
        TaskStatus::ReviewReady,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ] {
        let snapshot = tasks.snapshot.as_mut().unwrap();
        snapshot.receipts.truncate(2);
        snapshot.revision = 3;
        snapshot.tasks[0].status = status.clone();
        snapshot.tasks[0].run.as_mut().unwrap().evidence_sha256 = Some("b".repeat(64));
        snapshot.receipts.push(Receipt {
            revision: 3,
            task: 7,
            request: Request {
                correlation: "finish:task-7-run-2".into(),
                expected_revision: 2,
                action: Action::Finish {
                    task: 7,
                    run: "task-7-run-2".into(),
                    status: status.clone(),
                    evidence_sha256: "b".repeat(64),
                    detail: "Retained result".into(),
                },
            },
        });
        let finished = match status {
            TaskStatus::ReviewReady => "✓ Task #7 check passed · awaiting review",
            TaskStatus::Failed => "✗ Task #7 failed",
            _ => "– Task #7 run cancelled",
        };
        app.selected = 1;
        assert!(!render(&app, &tasks, 140, 32).contains(finished));
        app.selected = 0;
        let result = render(&app, &tasks, 140, 32);
        assert!(result.contains("✓ Task #7 started"), "{result}");
        assert!(result.contains(finished), "{result}");
        assert!(!result.contains("no result yet"), "{result}");
        assert!(!result.contains("Task #7 accepted"), "{result}");
    }
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(
        1,
        Update::Token((0..70).map(|n| format!("AUTO_READING_{n:03}\n")).collect()),
    );
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Still unsent 🦀");
    render(&app, &tasks, 80, 24);
    app.sessions[0].scroll_rows(-20);
    let first_marker = |text: &str| {
        text.find("AUTO_READING_")
            .map(|index| text[index..index + 16].to_owned())
    };
    let marker = first_marker(&render(&app, &tasks, 80, 24));
    assert!(marker.is_some());
    app.sessions[0]
        .submit_automatic_command(
            "Automatic /run 8 · dispatch command #1".into(),
            Intent::DispatchRun {
                request: RunRequest {
                    correlation: "automatic-next".into(),
                    expected_revision: 4,
                    task: 8,
                    approval_revision: 4,
                    source,
                },
            },
        )
        .unwrap();
    assert_eq!(first_marker(&render(&app, &tasks, 80, 24)), marker);
    assert_eq!(first_marker(&render(&app, &tasks, 60, 25)), marker);
    let restored = conversations::Snapshot::capture(&app, "automatic-test").restore();
    assert_eq!(first_marker(&render(&restored, &tasks, 60, 25)), marker);
    assert_eq!(restored.sessions[0].draft, "Still unsent 🦀");
    assert_eq!(restored.sessions[1].draft, "Selected unsent 中文");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_architect_draft_stays_with_review_and_preserves_background_reading() {
    use alfredo_tui::{
        architecture::Origin,
        assessment::{Decision, FailureKind, Outcome as ReviewOutcome},
        command_intent::Intent,
        console_command::CommandState,
        conversations,
        planner::{Plan, Step},
        planner_command::{ArchitectRequest, Operation, Outcome, Request as PlannerRequest},
        task_control::TaskControl,
        tasks::{Action, Receipt, Request, Snapshot, TaskStore, WorkPolicy},
    };
    let root =
        std::env::temp_dir().join(format!("alfredo-architect-layout-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut tasks =
        TaskControl::new(TaskStore::new(&root.join("state"), &workspace, "architect").unwrap());
    tasks.visible = false;
    let source = Request {
        correlation: "review-needs-architecture".into(),
        expected_revision: 0,
        action: Action::ReviewArchitecture {
            task: 7,
            decision: Decision {
                failure: Some(FailureKind::Architecture),
                risk: None,
                outcome: ReviewOutcome::NeedsRepair,
                reason: "Revise the approach".into(),
                criteria: vec![],
                limitations: vec![],
            },
        },
    };
    let origin = Origin {
        task: 7,
        review_revision: 1,
        run: "task-7-run-1".into(),
        evidence_sha256: "a".repeat(64),
    };
    tasks.snapshot = Some(Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace,
        mission: "architect".into(),
        revision: 1,
        tasks: vec![],
        receipts: vec![Receipt {
            revision: 1,
            task: 7,
            request: source.clone(),
        }],
    });
    let render = |app: &App, tasks: &TaskControl, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, app, tasks))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let mut app = App::new("chat".into());
    // The source is the exact Task request and its canonical receipt, not a local success state.
    app.sessions[0]
        .submit_command(
            "/review 7 needs-repair architecture".into(),
            Intent::Task {
                request: source.clone(),
            },
        )
        .unwrap();
    app.sessions[0].insert("Later discussion");
    assert_eq!(app.sessions[0].begin().unwrap().len(), 1);
    app.sessions[0].apply(
        1,
        Update::Token(
            (0..70)
                .map(|n| format!("ARCHITECT_HISTORY_{n:03}\n"))
                .collect(),
        ),
    );
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Origin unsent 🦀");
    render(&app, &tasks, 100, 24);
    app.sessions[0].scroll_rows(-20);
    let first_marker = |text: &str| {
        text.find("ARCHITECT_HISTORY_")
            .map(|index| text[index..index + 21].to_owned())
    };
    let marker = first_marker(&render(&app, &tasks, 100, 24));
    assert!(marker.is_some());
    app.add_session();
    app.sessions[1].insert("Selected unsent 中文");
    let intent = Intent::ArchitectDraft {
        request: ArchitectRequest {
            request: PlannerRequest {
                correlation: "automatic-architect".into(),
                operation: Operation::Architect {
                    origin: origin.clone(),
                    revision: 1,
                },
            },
            source,
        },
    };
    let id = app.sessions[0]
        .submit_automatic_command(
            "Automatic /architect-revise 7 · review command #1".into(),
            intent.clone(),
        )
        .unwrap();
    assert_eq!(app.selected, 1);
    assert!(!render(&app, &tasks, 140, 32).contains("Architect · revise task #7"));
    assert_eq!(app.sessions[0].draft, "Origin unsent 🦀");
    assert_eq!(app.sessions[1].draft, "Selected unsent 中文");
    assert_eq!(app.sessions[0].messages.len(), 2);
    assert!(intent.reconcile(tasks.snapshot.as_ref(), None).is_none());
    app.selected = 0;
    assert_eq!(first_marker(&render(&app, &tasks, 100, 24)), marker);
    assert_eq!(first_marker(&render(&app, &tasks, 60, 25)), marker);
    let restored = conversations::Snapshot::capture(&app, "architect-test").restore();
    assert_eq!(first_marker(&render(&restored, &tasks, 60, 25)), marker);
    assert_eq!(restored.sessions[0].draft, "Origin unsent 🦀");
    restored.sessions[0].scroll_rows(1000);
    let unknown = render(&restored, &tasks, 140, 32);
    assert!(
        unknown.contains("◆ Architect · revise task #7"),
        "{unknown}"
    );
    assert!(unknown.contains("Outcome unconfirmed"), "{unknown}");
    assert!(!unknown.contains("Draft generated"), "{unknown}");
    assert!(!tasks.planner.active());
    assert!(tasks.workers.is_empty());
    app.sessions[0].scroll_rows(1000);
    for (width, height) in [(140, 32), (60, 24)] {
        let pending = render(&app, &tasks, width, height);
        assert!(
            pending.contains("◆ Architect · revise task #7"),
            "{pending}"
        );
        assert!(!pending.contains("› Automatic"), "{pending}");
        assert!(!pending.contains("▶ Dispatch"), "{pending}");
        assert!(pending.contains("Pending · saving intent"), "{pending}");
        assert!(!pending.contains("no result yet"), "{pending}");
        assert!(!pending.contains("Task #7 started"), "{pending}");
    }
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    assert!(render(&app, &tasks, 140, 32).contains("Submitted · planner operation pending"));
    app.sessions[0].set_command_state(
        &id,
        CommandState::Planner {
            outcome: Outcome::Generated {
                draft_sha256: "b".repeat(64),
                tasks: 1,
            },
        },
    );
    app.selected = 1;
    assert!(!render(&app, &tasks, 140, 32).contains("Draft generated"));
    app.selected = 0;
    for (width, height) in [(140, 32), (60, 24)] {
        let generated = render(&app, &tasks, width, height);
        assert!(generated.contains("Draft generated"), "{generated}");
        assert!(!generated.contains("Plan saved"), "{generated}");
        assert!(!generated.contains("approved"), "{generated}");
        assert!(!generated.contains("no result yet"), "{generated}");
    }
    assert_eq!(tasks.snapshot.as_ref().unwrap().receipts.len(), 1);
    let save = Request {
        correlation: "save-architect-draft".into(),
        expected_revision: 1,
        action: Action::Plan {
            plan: Plan {
                prompt: "Revise the approach".into(),
                planner: "architect".into(),
                context: None,
                scope: None,
                architecture: Some(origin),
                tasks: vec![Step {
                    title: "Repair implementation".into(),
                    model: "worker".into(),
                    dependencies: vec![],
                    acceptance: vec!["Check passes".into()],
                    policy: WorkPolicy {
                        files: vec!["file.txt".into()],
                        check: vec!["true".into()],
                    },
                }],
            },
        },
    };
    app.sessions[0]
        .submit_command(
            "/plan-save".into(),
            Intent::Task {
                request: save.clone(),
            },
        )
        .unwrap();
    tasks.snapshot.as_mut().unwrap().receipts.push(Receipt {
        revision: 2,
        task: 8,
        request: save,
    });
    tasks.snapshot.as_mut().unwrap().revision = 2;
    let saved = render(&app, &tasks, 140, 32);
    assert!(saved.contains("◆ Architect · revise task #7"), "{saved}");
    assert!(
        saved.contains("Draft generated · 1 step · saving and approval are separate"),
        "{saved}"
    );
    assert!(saved.contains("› /plan-save"), "{saved}");
    assert!(saved.contains("✓ Plan saved · task #8"), "{saved}");
    assert!(!saved.contains("approved"), "{saved}");
    assert_eq!(app.sessions[0].draft, "Origin unsent 🦀");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn wayfinder_scope_action_keeps_turn_origin_after_cancellation_and_restart() {
    use alfredo_tui::{
        console_command::CommandState,
        conversations,
        model::Status,
        task_control::TaskControl,
        tasks::TaskStore,
        understanding::{Action, Flow, Mode, Request},
    };
    let root =
        std::env::temp_dir().join(format!("alfredo-wayfinder-layout-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let store = TaskStore::new(&root.join("state"), &workspace, "wayfinder").unwrap();
    let scope = store.understanding();
    let mut tasks = TaskControl::new(store.clone());
    tasks.visible = false;
    tasks.canonical_scope = Some(scope.snapshot().unwrap());
    let render = |app: &App, tasks: &TaskControl, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, app, tasks))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let prompt = "Build a new project WAYFINDER_ORIGIN_PROMPT";
    let mut app = App::new("chat".into());
    app.sessions[0].insert(prompt);
    let messages = app.sessions[0].begin().unwrap();
    assert_eq!(messages.len(), 1);
    let before = app.sessions[0].messages.clone();
    app.sessions[0].insert("Unsent origin 🦀");
    let request = Request {
        correlation: "wayfinder-origin".into(),
        expected_revision: 0,
        action: Action::Enter {
            flow: Flow {
                mode: Mode::Chart,
                prompt: prompt.into(),
            },
        },
    };
    let id = app.sessions[0]
        .submit_wayfinder_command(0, request.clone())
        .unwrap();
    assert_eq!(app.sessions[0].messages, before);
    assert_eq!(app.sessions[0].draft, "Unsent origin 🦀");
    for (width, height) in [(140, 32), (60, 24)] {
        let pending = render(&app, &tasks, width, height);
        assert!(pending.contains("◇ Wayfinder · turn 1"), "{pending}");
        assert!(pending.contains("Enter shared understanding"), "{pending}");
        assert!(pending.contains("Pending · saving intent"), "{pending}");
        assert!(!pending.contains("› Enter"), "{pending}");
        assert!(!pending.contains("Scope saved"), "{pending}");
        assert!(!pending.contains("Task #"), "{pending}");
    }
    assert_eq!(
        render(&app, &tasks, 140, 32)
            .matches("WAYFINDER_ORIGIN_PROMPT")
            .count(),
        1
    );
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    assert!(render(&app, &tasks, 140, 32).contains("Submitted · awaiting scope receipt"));
    app.sessions[0].cancel();
    let restored = conversations::Snapshot::capture(&app, "wayfinder-test").restore();
    let unknown = render(&restored, &tasks, 140, 32);
    assert!(unknown.contains("Outcome unconfirmed"), "{unknown}");
    assert!(!unknown.contains("Scope saved"), "{unknown}");
    assert_eq!(scope.snapshot().unwrap().revision, 0);
    assert!(tasks.workers.is_empty());
    app.add_session();
    app.sessions[1].insert("Unsent selected 中文");
    tasks.canonical_scope = Some(scope.transact(request.clone()).unwrap());
    assert!(!render(&app, &tasks, 140, 32).contains("Scope saved"));
    assert_eq!(app.selected, 1);
    app.selected = 0;
    let acknowledged = render(&app, &tasks, 140, 32);
    assert!(acknowledged.contains("✓ Scope saved"), "{acknowledged}");
    assert!(!acknowledged.contains("wayfinder-origin"), "{acknowledged}");
    // The scope revision is shown by the scope view itself.
    tasks.visible = true;
    tasks.scope_view = tasks.canonical_scope.clone();
    assert!(render(&app, &tasks, 140, 32).contains("Shared Understanding · revision 1"));
    tasks.visible = false;
    tasks.scope_view = None;
    assert!(matches!(app.sessions[0].status, Status::Cancelled));
    assert!(app.sessions[0].messages[1].content.is_empty());
    assert_eq!(app.sessions[0].draft, "Unsent origin 🦀");
    assert_eq!(app.sessions[1].draft, "Unsent selected 中文");
    assert!(!tasks.canonical_scope.as_ref().unwrap().confirmed);
    assert!(store.snapshot().unwrap().tasks.is_empty());
    assert!(render(&restored, &tasks, 140, 32).contains("✓ Scope saved"));
    // Same correlation and revision with a different action cannot acknowledge this turn.
    tasks.canonical_scope.as_mut().unwrap().receipts[0]
        .request
        .action = Action::Confirm { draft_revision: 1 };
    assert!(!render(&app, &tasks, 140, 32).contains("Scope saved"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn delayed_wayfinder_action_renders_at_its_turn_and_preserves_later_reading() {
    use alfredo_tui::{
        command_intent::Intent,
        conversations,
        tasks::{Action as TaskAction, Request as TaskRequest},
        understanding::{Action, Brief, Request},
    };
    let render = |app: &App, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let mut app = App::new("chat".into());
    app.sessions[0].insert("Destination: ORIGIN_DESTINATION\nScope: Local tasks\nConstraints: Local only\nUncertainty: Latency");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("Reviewing the scope".into()));
    app.sessions[0].apply(1, Update::Done);
    let later = |correlation: &str| Intent::Task {
        request: TaskRequest {
            correlation: correlation.into(),
            expected_revision: 0,
            action: TaskAction::Propose {
                title: "Unrelated proposal".into(),
                model: "worker".into(),
                dependencies: vec![],
            },
        },
    };
    app.sessions[0]
        .submit_command("/task first unrelated proposal".into(), later("later-one"))
        .unwrap();
    app.sessions[0].insert("LATER_DISCUSSION");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(
        2,
        Update::Token(
            (0..70)
                .map(|n| format!("WAYFINDER_READING_{n:03}\n"))
                .collect(),
        ),
    );
    app.sessions[0].apply(2, Update::Done);
    app.sessions[0]
        .submit_command("/task second unrelated proposal".into(), later("later-two"))
        .unwrap();
    app.sessions[0].insert("Keep newest draft 🦀");
    render(&app, 100, 24);
    app.sessions[0].scroll_rows(-20);
    let first_marker = |text: &str| {
        text.find("WAYFINDER_READING_")
            .map(|index| text[index..index + 21].to_owned())
    };
    let marker = first_marker(&render(&app, 100, 24));
    assert!(marker.is_some());
    app.sessions[0]
        .submit_wayfinder_command(
            0,
            Request {
                correlation: "delayed-scope".into(),
                expected_revision: 1,
                action: Action::Draft {
                    brief: Brief {
                        destination: "ORIGIN_DESTINATION".into(),
                        scope: "Local tasks".into(),
                        constraints: "Local only".into(),
                        uncertainty: "Latency".into(),
                    },
                },
            },
        )
        .unwrap();
    assert_eq!(first_marker(&render(&app, 100, 24)), marker);
    assert_eq!(first_marker(&render(&app, 60, 25)), marker);
    assert_eq!(app.sessions[0].draft, "Keep newest draft 🦀");
    let restored = conversations::Snapshot::capture(&app, "delayed-wayfinder-test").restore();
    assert_eq!(first_marker(&render(&restored, 60, 25)), marker);
    assert_eq!(restored.sessions[0].draft, "Keep newest draft 🦀");
    app.sessions[0].scroll_rows(-1000);
    let beginning = render(&app, 140, 32);
    let origin = beginning.find("ORIGIN_DESTINATION").unwrap();
    let earlier_command = beginning.find("› /task first unrelated proposal").unwrap();
    let scope = beginning.find("◇ Wayfinder · turn 1").unwrap();
    let later_turn = beginning.find("LATER_DISCUSSION").unwrap();
    assert!(
        origin < earlier_command && earlier_command < scope && scope < later_turn,
        "{beginning}"
    );
    assert!(
        beginning.contains("Save scope draft for review"),
        "{beginning}"
    );
    assert_eq!(beginning.matches("ORIGIN_DESTINATION").count(), 1);
    app.sessions[0].scroll_rows(1000);
    assert!(render(&app, 140, 32).contains("› /task second unrelated proposal"));
}

#[test]
fn wayfinder_reply_header_requires_current_receipt_proof_and_bound_request() {
    use alfredo_tui::{
        conversations,
        model::ScopeReceiptRef,
        task_control::TaskControl,
        tasks::TaskStore,
        understanding::{Action, Flow, Mode, Request},
    };
    use ratatui::style::Color;
    let root =
        std::env::temp_dir().join(format!("alfredo-wayfinder-header-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let store = TaskStore::new(&root.join("state"), &workspace, "header").unwrap();
    let request = Request {
        correlation: "verified-wayfinder".into(),
        expected_revision: 0,
        action: Action::Enter {
            flow: Flow {
                mode: Mode::Chart,
                prompt: "Build a new project".into(),
            },
        },
    };
    let canonical = store.understanding().transact(request.clone()).unwrap();
    let mut tasks = TaskControl::new(store);
    tasks.visible = false;
    let render = |app: &App, tasks: &TaskControl, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, app, tasks))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        let heading = buffer
            .content
            .windows(9)
            .position(|cells| {
                cells.iter().map(|cell| cell.symbol()).collect::<String>() == "Wayfinder"
            })
            .expect("Wayfinder reply header visible");
        (text, buffer.content[heading].fg)
    };
    for bound in [false, true] {
        let mut app = App::new("chat".into());
        app.sessions[0].insert("Build a new project");
        app.sessions[0].begin().unwrap();
        if bound {
            app.sessions[0]
                .submit_wayfinder_command(0, request.clone())
                .unwrap();
        }
        app.sessions[0].wayfinder_reply(
            1,
            "Retained historical reply".into(),
            Some(ScopeReceiptRef {
                correlation: request.correlation.clone(),
                revision: 1,
            }),
        );
        let app = conversations::Snapshot::capture(&app, "header-test").restore();
        for (width, height) in [(140, 32), (60, 24)] {
            tasks.canonical_scope = Some(canonical.clone());
            let (verified, color) = render(&app, &tasks, width, height);
            assert!(
                verified.contains("Wayfinder · scope receipt 1"),
                "{verified}"
            );
            // Speaker labels are dim; only an unverified claim is coloured.
            assert_eq!(color, Color::DarkGray);
            for fault in [
                "missing-scope",
                "missing-receipt",
                "correlation",
                "revision",
                "actor",
                "bound-request",
            ] {
                if fault == "bound-request" && !bound {
                    continue;
                }
                let mut candidate = canonical.clone();
                match fault {
                    "missing-scope" => {}
                    "missing-receipt" => candidate.receipts.clear(),
                    "correlation" => candidate.receipts[0].request.correlation = "impostor".into(),
                    "revision" => candidate.receipts[0].request.expected_revision = 1,
                    "actor" => candidate.receipts[0].actor = "model".into(),
                    "bound-request" => {
                        candidate.receipts[0].request.action = Action::Enter {
                            flow: Flow {
                                mode: Mode::WorkThrough,
                                prompt: "Another scope action".into(),
                            },
                        };
                    }
                    _ => unreachable!(),
                }
                tasks.canonical_scope = (fault != "missing-scope").then_some(candidate);
                let (unverified, color) = render(&app, &tasks, width, height);
                assert!(
                    unverified.contains("saved receipt 1"),
                    "{fault}: {unverified}"
                );
                assert!(unverified.contains("unverified"), "{fault}: {unverified}");
                assert!(
                    !unverified.contains("Wayfinder · scope receipt 1"),
                    "{fault}: {unverified}"
                );
                assert!(
                    unverified.contains("Retained historical reply"),
                    "{fault}: {unverified}"
                );
                assert_eq!(color, Color::Yellow, "{fault}: {unverified}");
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn fresh_application_notice_is_visible_over_a_selected_failed_session() {
    use alfredo_tui::{model::Status, task_control::TaskControl, tasks::TaskStore};
    let root = std::env::temp_dir().join(format!("alfredo-notice-layout-{}", std::process::id()));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut tasks =
        TaskControl::new(TaskStore::new(&root.join("state"), &workspace, "notice").unwrap());
    tasks.notice = "Retained task notice".into();
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("An earlier model request");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Failed("Retained provider failure".into()));
    let render = |app: &App, tasks: Option<&TaskControl>, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| match tasks {
                Some(tasks) => ui::draw_with_tasks(frame, app, tasks),
                None => ui::draw(frame, app),
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let footer = buffer
            .content
            .chunks(width as usize)
            .last()
            .unwrap()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        let screen = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        (footer, screen)
    };
    for (width, height) in [(140, 32), (60, 24)] {
        tasks.visible = false;
        app.notice = "Wayfinder scope receipt verified".into();
        for panel in [None, Some(&tasks)] {
            let (footer, screen) = render(&app, panel, width, height);
            assert!(
                footer.contains("Wayfinder scope receipt verified"),
                "{screen}"
            );
            assert!(!footer.contains("Retained provider failure"), "{screen}");
            // The failure reason stays in the transcript itself.
            assert!(
                screen.contains("✗ Request failed · Retained provider"),
                "{screen}"
            );
        }
        app.notice.clear();
        let (footer, _) = render(&app, Some(&tasks), width, height);
        assert!(footer.contains("Retained provider failure"), "{footer}");
        assert!(matches!(app.sessions[0].status, Status::Failed(_)));
        tasks.visible = true;
        let (footer, _) = render(&app, Some(&tasks), width, height);
        assert!(footer.contains("Retained task notice"), "{footer}");
        app.notice = "Wayfinder scope receipt verified".into();
        let (footer, _) = render(&app, Some(&tasks), width, height);
        assert!(
            footer.contains("Wayfinder scope receipt verified"),
            "{footer}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

fn normalize_selection_screen(text: String) -> String {
    text.replace(['│', '┌', '┐', '└', '┘', '─'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn shared_capacity_queue_and_upstream_wait_remain_distinct_in_wide_and_narrow_layouts() {
    use alfredo_tui::inference_admission::{Class, Observation};
    let render = |app: &App, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
        normalize_selection_screen(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect(),
        )
    };
    for (width, height, class) in [
        (140, 32, Class::Foreground),
        (60, 24, Class::Foreground),
        (140, 32, Class::Background),
        (60, 24, Class::Background),
    ] {
        let mut app = App::new("fixture".into());
        app.sessions[0].insert("Discuss this change");
        app.sessions[0].begin().unwrap();
        app.sessions[0].insert("Unfinished next prompt 🦀");
        app.sessions[0].apply(1, Update::Queued);
        let generic = render(&app, width, height);
        assert!(generic.contains("Chat 1 · queued"), "{generic}");
        assert!(!generic.contains("queued 2/3"), "{generic}");
        let mut observation = Observation {
            position: 2,
            waiting: 3,
            active: 2,
            capacity: 2,
            class,
        };
        app.sessions[0].apply(1, Update::QueueProgress(observation));
        let queued = render(&app, width, height);
        for expected in ["Chat 1 · queued", "queued 2/3 · 2/2 active"] {
            assert!(queued.contains(expected), "{expected}: {queued}");
        }
        // The queue class stays in the observation, not the compact line.
        assert_eq!(app.sessions[0].queue_observation().unwrap().class, class);
        assert!(!queued.contains("waiting for model"), "{queued}");
        assert!(!queued.contains("loading"), "{queued}");
        observation.position = 1;
        observation.waiting = 2;
        observation.active = 1;
        app.sessions[0].apply(1, Update::QueueProgress(observation));
        let changed = render(&app, width, height);
        for expected in ["queued 1/2", "1/2 active"] {
            assert!(changed.contains(expected), "{expected}: {changed}");
        }
        assert!(!changed.contains("queued 2/3"), "{changed}");
        app.sessions[0].apply(1, Update::Admitted);
        let upstream = render(&app, width, height);
        assert!(upstream.contains("waiting for model"), "{upstream}");
        assert!(!upstream.contains("queued 1/2"), "{upstream}");
        assert!(!upstream.contains("loading"), "{upstream}");
        assert!(!upstream.contains("Server timing"), "{upstream}");
        assert_eq!(app.sessions[0].draft, "Unfinished next prompt 🦀");
        assert!(app.sessions[0].messages[1].content.is_empty());
    }
}

fn selection_layout_request() -> alfredo_tui::selection_command::Request {
    use alfredo_tui::selection_command::{Choice, MissionChoice, Origin, Request, WorkspaceChoice};
    Request {
        correlation: "selection-origin".into(),
        origin: Origin::Conversation {
            workspace: "/repo/source".into(),
            mission: "source".into(),
            conversation: "default".into(),
            session: 0,
        },
        choice: Choice {
            workspace: WorkspaceChoice::Create {
                parent: "/repo".into(),
                name: "created".into(),
            },
            mission: MissionChoice::StartNew {
                name: "next".into(),
            },
        },
        conversation: "default".into(),
    }
}

#[test]
fn selection_history_separates_creation_loading_and_actual_handoff() {
    use alfredo_tui::{
        console_command::CommandState,
        selection_command::{Outcome, Phase},
    };
    use ratatui::style::Color;
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("Source unfinished draft 🦀");
    let id = app.sessions[0]
        .submit_selection_command(
            "Create /repo/created · start mission next".into(),
            selection_layout_request(),
        )
        .unwrap();
    let render = |app: &App, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        assert!(
            !buffer
                .content
                .iter()
                .any(|cell| cell.fg == Color::Green && !cell.symbol().trim().is_empty()),
            "Selection history must not use task acknowledgment styling"
        );
        normalize_selection_screen(
            buffer
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>(),
        )
    };
    let pending = render(&app, 140, 32);
    assert!(pending.contains("› Create /repo/created"), "{pending}");
    assert!(pending.contains("Pending · saving intent"), "{pending}");
    assert!(!pending.contains("Repository created"), "{pending}");
    let restored = alfredo_tui::conversations::Snapshot::capture(&app, "default").restore();
    let interrupted = render(&restored, 60, 24);
    assert!(interrupted.contains("Outcome unconfirmed"), "{interrupted}");
    assert!(!interrupted.contains("Repository created"), "{interrupted}");
    assert_eq!(restored.sessions[0].draft, "Source unfinished draft 🦀");
    for (phase, expected, absent) in [
        (Phase::Admitted, "Selection admitted", "Repository created"),
        (
            Phase::RepositoryReady,
            "Repository created",
            "Mission created",
        ),
        (Phase::MissionReady, "Mission created", "target loaded"),
        (Phase::TargetLoaded, "target loaded", "handoff prepared"),
        (
            Phase::HandoffPrepared,
            "handoff prepared",
            "workspace selected",
        ),
        (
            Phase::Selected,
            "workspace selected",
            "selection not recorded",
        ),
    ] {
        app.sessions[0].set_command_state(
            &id,
            CommandState::Selection {
                outcome: Outcome {
                    phase,
                    failure: None,
                },
            },
        );
        for (width, height) in [(140, 32), (60, 24)] {
            let text = render(&app, width, height);
            assert!(text.contains(expected), "{text}");
            assert!(!text.contains(absent), "{text}");
            assert!(text.contains("/repo/created"), "{text}");
            assert!(!text.contains("Task #"), "{text}");
            assert!(!text.contains("approved"), "{text}");
        }
    }
    app.sessions[0].set_command_state(
        &id,
        CommandState::Selection {
            outcome: Outcome {
                phase: Phase::RepositoryReady,
                failure: Some("Mission creation failed; repository directory retained".into()),
            },
        },
    );
    for (width, height) in [(140, 32), (60, 24)] {
        let text = render(&app, width, height);
        assert!(text.contains("Repository created"), "{text}");
        assert!(text.contains("Mission creation failed"), "{text}");
        assert!(text.contains("directory retained"), "{text}");
        assert!(!text.contains("Mission created"), "{text}");
        assert!(!text.contains("workspace selected"), "{text}");
    }
    assert_eq!(app.sessions[0].draft, "Source unfinished draft 🦀");
    assert!(app.sessions[0].messages.is_empty());
    app.add_session();
    assert!(!render(&app, 140, 32).contains("› Create /repo/created"));
    app.selected = 0;
    app.sessions[0].set_command_state(
        &id,
        CommandState::Unknown {
            reason: "Selection journal unavailable".into(),
        },
    );
    let unknown = render(&app, 60, 24);
    assert!(unknown.contains("Outcome unconfirmed"), "{unknown}");
    assert!(!unknown.contains("workspace selected"), "{unknown}");
}

#[test]
fn selection_arrival_preserves_destination_reading_and_waits_for_actual_selection() {
    use alfredo_tui::{
        console_command::CommandState,
        conversations,
        selection_command::{Outcome, Phase},
    };
    let render = |app: &App, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
        normalize_selection_screen(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>(),
        )
    };
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("Existing destination discussion");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(
        1,
        Update::Token(
            (0..70)
                .map(|n| format!("SELECTION_READING_{n:03}\n"))
                .collect(),
        ),
    );
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Destination unfinished draft 🦀");
    let messages = app.sessions[0].messages.clone();
    render(&app, 100, 24);
    app.sessions[0].scroll_rows(-20);
    let first_marker = |text: &str| {
        text.find("SELECTION_READING_")
            .map(|index| text[index..index + 21].to_owned())
    };
    let marker = first_marker(&render(&app, 100, 24));
    assert!(marker.is_some());
    app.add_session();
    app.sessions[1].insert("Unrelated selected draft 中文");
    let id = app.sessions[0]
        .submit_selection_arrival(
            selection_layout_request(),
            Outcome {
                phase: Phase::HandoffPrepared,
                failure: None,
            },
        )
        .unwrap();
    assert_eq!(app.selected, 1);
    assert!(!render(&app, 140, 32).contains("Workspace /repo/created"));
    assert_eq!(app.sessions[0].messages, messages);
    assert_eq!(app.sessions[0].draft, "Destination unfinished draft 🦀");
    assert_eq!(app.sessions[1].draft, "Unrelated selected draft 中文");
    app.selected = 0;
    assert_eq!(first_marker(&render(&app, 100, 24)), marker);
    assert_eq!(first_marker(&render(&app, 60, 25)), marker);
    let restored = conversations::Snapshot::capture(&app, "default").restore();
    assert_eq!(first_marker(&render(&restored, 60, 25)), marker);
    assert_eq!(
        restored.sessions[0].draft,
        "Destination unfinished draft 🦀"
    );
    app.sessions[0].scroll_rows(1000);
    for (width, height) in [(140, 32), (60, 24)] {
        let prepared = render(&app, width, height);
        // One dim line: destination, then the preparation milestone.
        assert!(
            prepared.contains("· Workspace /repo/created · mission next"),
            "{prepared}"
        );
        assert!(prepared.contains("handoff prepared"), "{prepared}");
        assert!(prepared.contains("selection not recorded"), "{prepared}");
        assert!(!prepared.contains("workspace selected"), "{prepared}");
        assert!(!prepared.contains("Task #"), "{prepared}");
        assert!(!prepared.contains("› Workspace"), "{prepared}");
    }
    app.sessions[0].set_command_state(
        &id,
        CommandState::Selection {
            outcome: Outcome {
                phase: Phase::Selected,
                failure: None,
            },
        },
    );
    let selected = render(&app, 140, 32);
    assert!(selected.contains("mission next · ready"), "{selected}");
    assert!(!selected.contains("selection not recorded"), "{selected}");
    assert_eq!(app.sessions[0].messages, messages);
}
