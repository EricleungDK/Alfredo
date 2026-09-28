use alfredo_tui::{
    conversations::{Autosave, ConversationStore, Snapshot},
    model::{App, Status, Update},
    tasks::TaskStore,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    tasks: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-conversations-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("workspace")).unwrap();
        let tasks =
            TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
        Self { root, tasks }
    }
    fn file(&self) -> PathBuf {
        fs::read_dir(self.tasks.conversation_directory().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn restart_restores_models_selection_drafts_and_partial_text_without_replay() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("first-model".into());
    app.sessions[0].insert("question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("partial answer".into()));
    app.add_session();
    app.sessions[1].model = "second-model".into();
    app.sessions[1].insert("A界Z");
    app.sessions[1].left();
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    drop(store);
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut restored = store.load().unwrap().unwrap().restore();
    assert_eq!(restored.selected, 1);
    assert_eq!(restored.sessions[1].model, "second-model");
    restored.sessions[1].insert("🦀");
    assert_eq!(restored.sessions[1].draft, "A界🦀Z");
    assert!(matches!(restored.sessions[0].status, Status::Failed(_)));
    assert_eq!(restored.sessions[0].messages[1].content, "partial answer");
    assert_eq!(restored.sessions[0].attempt, 1);
    assert!(restored.sessions[0].begin().is_err());
    let request = restored.sessions[0].retry().unwrap();
    assert_eq!(request.len(), 1);
    assert_eq!(request[0].content, "question");
    assert_eq!(restored.sessions[0].attempt, 2);
}

#[test]
fn conversation_owners_cannot_overwrite_each_other_and_names_are_isolated() {
    let fixture = Fixture::new();
    let first = ConversationStore::open(&fixture.tasks, "default").unwrap();
    assert!(ConversationStore::open(&fixture.tasks, "default").is_err());
    let other = ConversationStore::open(&fixture.tasks, "other").unwrap();
    assert!(other.load().unwrap().is_none());
    first
        .save(&Snapshot::capture(&App::new("one".into()), "default"))
        .unwrap();
    assert!(other.load().unwrap().is_none());
    drop(first);
    assert!(ConversationStore::open(&fixture.tasks, "default")
        .unwrap()
        .load()
        .unwrap()
        .is_some());
}

#[cfg(unix)]
#[test]
fn fork_inherited_descriptor_does_not_extend_released_conversation_ownership() {
    let fixture = Fixture::new();
    let owner = ConversationStore::open(&fixture.tasks, "default").unwrap();
    owner
        .save(&Snapshot::capture(&App::new("fixture".into()), "default"))
        .unwrap();
    let mut pipe = [-1; 2];
    // Fork captures the descriptor before the owner's orderly release. The child
    // uses only async-signal-safe syscalls and retains it until explicitly released.
    assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0);
    if pid == 0 {
        unsafe {
            libc::close(pipe[1]);
            let mut byte = 0u8;
            while libc::read(pipe[0], (&mut byte as *mut u8).cast(), 1) < 0 {}
            libc::_exit(0);
        }
    }
    unsafe {
        libc::close(pipe[0]);
    }
    struct Child {
        pid: libc::pid_t,
        release: libc::c_int,
    }
    impl Drop for Child {
        fn drop(&mut self) {
            unsafe {
                let byte = 1u8;
                libc::write(self.release, (&byte as *const u8).cast(), 1);
                libc::close(self.release);
                while libc::waitpid(self.pid, std::ptr::null_mut(), 0) < 0 {
                    if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                        break;
                    }
                }
            }
        }
    }
    let child = Child {
        pid,
        release: pipe[1],
    };
    assert!(ConversationStore::open(&fixture.tasks, "default").is_err());
    drop(owner);
    let reopened = ConversationStore::open(&fixture.tasks, "default");
    // Observe the result while the inherited descriptor is still definitely open.
    drop(child);
    let reopened = reopened
        .unwrap_or_else(|error| panic!("Released owner still blocks conversation reopen: {error}"));
    assert_eq!(
        reopened.load().unwrap().unwrap().sessions[0].model,
        "fixture"
    );
    assert!(ConversationStore::open(&fixture.tasks, "default").is_err());
}

#[test]
fn malformed_future_or_invalid_cursor_state_is_preserved() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("界");
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    let path = fixture.file();
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut future = original.clone();
    future["schema_version"] = 99.into();
    let mut cursor = original.clone();
    cursor["sessions"][0]["cursor"] = 1.into();
    let mut selection = original;
    selection["selected"] = 20.into();
    for bytes in [
        b"{broken".to_vec(),
        serde_json::to_vec(&future).unwrap(),
        serde_json::to_vec(&cursor).unwrap(),
        serde_json::to_vec(&selection).unwrap(),
    ] {
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn final_save_follows_pending_autosave_and_keeps_the_latest_draft() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut save = Autosave::new(store);
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("old");
    save.checkpoint(&runtime, &app, Default::default(), None)
        .unwrap();
    app.sessions[0].insert(" latest");
    save.finish(&runtime, &app, Default::default(), None)
        .unwrap();
    drop(save);
    let restored = ConversationStore::open(&fixture.tasks, "default")
        .unwrap()
        .load()
        .unwrap()
        .unwrap()
        .restore();
    assert_eq!(restored.sessions[0].draft, "old latest");
}

#[test]
fn rejected_snapshot_preserves_the_last_valid_file() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let valid = Snapshot::capture(&App::new("fixture".into()), "default");
    store.save(&valid).unwrap();
    let original = fs::read(fixture.file()).unwrap();
    let mut invalid = valid.clone();
    invalid.sessions[0]
        .messages
        .push(alfredo_tui::model::Message {
            role: "assistant".into(),
            content: "fabricated reply".into(),
        });
    assert!(store.save(&invalid).is_err());
    assert_eq!(fs::read(fixture.file()).unwrap(), original);
    assert_eq!(store.load().unwrap().unwrap(), valid);
}

#[cfg(unix)]
#[test]
fn symlink_conversation_target_is_not_read_or_overwritten() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let snapshot = Snapshot::capture(&App::new("fixture".into()), "default");
    store.save(&snapshot).unwrap();
    let path = fixture.file();
    let outside = fixture.root.join("untouched");
    fs::write(&outside, b"private").unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, path).unwrap();
    assert!(store.load().is_err());
    assert!(store.save(&snapshot).is_err());
    assert_eq!(fs::read(outside).unwrap(), b"private");
}

#[test]
fn legacy_view_migration_preserves_exact_bytes_and_refuses_conflicting_backup() {
    use alfredo_tui::conversations::TaskView;
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut legacy = Snapshot::capture(&App::new("fixture".into()), "default");
    legacy.schema_version = 1;
    store.save(&legacy).unwrap();
    let path = fixture.file();
    let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(store.load().unwrap().unwrap().task_view.is_none());
    let mut latest = legacy.clone();
    latest.schema_version = 2;
    latest.task_view = Some(TaskView {
        visible: true,
        selected: Some(7),
        query: "#7".into(),
    });
    let backup = path.with_extension("v1-backup");
    fs::write(&backup, b"different").unwrap();
    assert!(store.save(&latest).unwrap_err().contains("backup differs"));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    fs::remove_file(&backup).unwrap();
    store.save(&latest).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), bytes);
    assert_eq!(store.load().unwrap().unwrap(), latest);
    store.save(&latest).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), bytes);
    for view in [
        TaskView {
            query: "x".repeat(201),
            ..Default::default()
        },
        TaskView {
            selected: Some(0),
            ..Default::default()
        },
    ] {
        let mut invalid = latest.clone();
        invalid.task_view = Some(view);
        assert!(store.save(&invalid).is_err());
        assert_eq!(store.load().unwrap().unwrap(), latest);
    }
    legacy.task_view = latest.task_view;
    assert!(store.save(&legacy).is_err());
}

#[test]
fn restored_task_preferences_select_only_current_visible_tasks_without_effects() {
    use alfredo_tui::{
        conversations::TaskView,
        task_control::TaskControl,
        tasks::{Action, Request},
    };
    let fixture = Fixture::new();
    let (snapshot, _) = fixture
        .tasks
        .transact(Request {
            correlation: "propose".into(),
            expected_revision: 0,
            action: Action::Propose {
                title: "resume me".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        })
        .unwrap();
    let mut tasks = TaskControl::new(fixture.tasks.clone());
    tasks
        .restore_view(TaskView {
            visible: true,
            selected: Some(1),
            query: "#1".into(),
        })
        .unwrap();
    assert!(tasks.selected_task().is_none()); // Loading is not an acknowledged task.
    tasks.snapshot = Some(snapshot.clone());
    assert_eq!(tasks.selected_task().unwrap().id, 1);
    assert!(tasks.visible);
    assert_eq!(tasks.view_preferences().query, "#1");
    tasks
        .restore_view(TaskView {
            visible: true,
            selected: Some(999),
            query: "#999".into(),
        })
        .unwrap();
    assert!(tasks.selected_task().is_none());
    assert!(!tasks.dispatch.enabled);
    assert!(tasks.evidence.is_none());
    assert_eq!(
        serde_json::to_value(fixture.tasks.snapshot().unwrap()).unwrap(),
        serde_json::to_value(snapshot).unwrap()
    );
}

#[test]
fn source_migration_preserves_v1_and_v2_bytes_without_inventing_legacy_authorship() {
    use alfredo_tui::model::ScopeReceiptRef;
    for version in [1, 2] {
        let fixture = Fixture::new();
        let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
        let mut app = App::new("fixture".into());
        app.sessions[0].insert("Legacy question");
        app.sessions[0].begin().unwrap();
        app.sessions[0].apply(
            1,
            Update::Token("Wayfinder · receipt: legacy-looking prose".into()),
        );
        app.sessions[0].apply(1, Update::Done);
        let mut legacy = serde_json::to_value(Snapshot::capture(&app, "default")).unwrap();
        legacy["schema_version"] = version.into();
        legacy["sessions"][0]
            .as_object_mut()
            .unwrap()
            .remove("sources");
        store
            .save(&serde_json::from_value(legacy.clone()).unwrap())
            .unwrap();
        let original = serde_json::to_vec_pretty(&legacy).unwrap();
        let path = fixture.file();
        fs::write(&path, &original).unwrap();
        let mut restored = store.load().unwrap().unwrap().restore();
        assert!(restored.sessions[0].source(1).is_none());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| alfredo_tui::ui::draw(frame, &restored))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Assistant · source unrecorded"), "{text}");
        restored.sessions[0].insert("New scope request");
        restored.sessions[0].begin().unwrap();
        restored.sessions[0].wayfinder_reply(
            2,
            "Application acknowledgment".into(),
            Some(ScopeReceiptRef {
                correlation: "native-scope".into(),
                revision: 3,
            }),
        );
        let current = Snapshot::capture(&restored, "default");
        let backup = path.with_extension(format!("v{version}-backup"));
        fs::write(&backup, b"different original").unwrap();
        assert!(store.save(&current).unwrap_err().contains("backup differs"));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_file(&backup).unwrap();
        store.save(&current).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), original);
        let saved = store.load().unwrap().unwrap();
        assert_eq!(saved.schema_version, 17);
        assert!(saved.sessions[0].source(1).is_none());
        assert_eq!(saved.sessions[0].source(3), restored.sessions[0].source(3));
        assert!(store
            .save(&serde_json::from_value(legacy).unwrap())
            .unwrap_err()
            .contains("downgrade"));
    }
}

#[test]
fn invalid_source_indices_kinds_and_old_schema_metadata_are_refused_unchanged() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("hello");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("reply".into()));
    app.sessions[0].apply(1, Update::Done);
    let snapshot = Snapshot::capture(&app, "default");
    store.save(&snapshot).unwrap();
    let path = fixture.file();
    let good = serde_json::to_value(&snapshot).unwrap();
    for invalid in [
        serde_json::json!({"0":{"kind":"model","model":"fixture"}}),
        serde_json::json!({"3":{"kind":"model","model":"fixture"}}),
        serde_json::json!({"1":{"kind":"wayfinder","receipt":{"correlation":"x","revision":0}}}),
        serde_json::json!({"1":{"kind":"administrator"}}),
        serde_json::json!({"1":{"kind":"model","model":"bad\nmodel"}}),
    ] {
        let mut bad = good.clone();
        bad["sessions"][0]["sources"] = invalid;
        let bytes = serde_json::to_vec(&bad).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    let mut old = good;
    old["schema_version"] = 2.into();
    fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    assert!(store.load().unwrap_err().contains("response sources"));
}

#[test]
fn saved_reading_anchor_survives_unseen_growth_restart_and_geometry_change() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("History");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(
        1,
        Update::Token((0..60).map(|n| format!("HISTORY_{n:03}\n")).collect()),
    );
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Current turn");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(2, Update::Token("Tail\n".into()));
    let render = |app: &App, width, height| {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| alfredo_tui::ui::draw(frame, app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        text.find("HISTORY_")
            .map(|index| text[index..index + 11].to_owned())
    };
    render(&app, 100, 24);
    app.sessions[0].scroll_rows(-20);
    let before = render(&app, 100, 24);
    assert!(before.is_some());
    // Hidden conversation continues receiving output before autosave; no redraw.
    app.sessions[0].apply(2, Update::Token("New output\n".repeat(40)));
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    drop(store);
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let restored = store.load().unwrap().unwrap().restore();
    assert!(matches!(restored.sessions[0].status, Status::Failed(_)));
    assert_eq!(render(&restored, 70, 28), before);
}

#[test]
fn v3_reading_migration_backs_up_exact_bytes_and_refuses_invalid_anchors() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("History");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("saved line\n".repeat(80)));
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].scroll.set(20);
    let mut legacy = Snapshot::capture(&app, "default");
    legacy.schema_version = 3;
    store.save(&legacy).unwrap();
    let path = fixture.file();
    let original = serde_json::to_vec_pretty(&legacy).unwrap();
    fs::write(&path, &original).unwrap();
    let restored = store.load().unwrap().unwrap().restore();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &restored))
        .unwrap();
    let current = Snapshot::capture(&restored, "default");
    let good = serde_json::to_value(&current).unwrap();
    assert!(good["sessions"][0]["reading"]["anchor"].is_object());
    let backup = path.with_extension("v3-backup");
    fs::write(&backup, b"conflict").unwrap();
    assert!(store.save(&current).unwrap_err().contains("backup differs"));
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&current).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert!(store.save(&legacy).unwrap_err().contains("downgrade"));
    for (field, value) in [
        ("line", serde_json::json!(1_000_000)),
        ("row", serde_json::json!(1_000_000)),
        ("row", serde_json::json!(-1)),
    ] {
        let mut bad = good.clone();
        bad["sessions"][0]["reading"]["anchor"][field] = value;
        let bytes = serde_json::to_vec(&bad).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    for version in [1, 2, 3] {
        let mut bad = good.clone();
        bad["schema_version"] = version.into();
        let bytes = serde_json::to_vec(&bad).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

fn saved_plan() -> alfredo_tui::planner::SavedDraft {
    alfredo_tui::planner::SavedDraft {
        origin: None,
        revision: 0,
        plan: alfredo_tui::planner::Plan {
            architecture: None,
            prompt: "Implement calculation".into(),
            planner: "fixture".into(),
            context: None,
            scope: None,
            tasks: vec![alfredo_tui::planner::Step {
                acceptance: vec![],
                title: "Implement calculation".into(),
                model: "fixture".into(),
                dependencies: vec![],
                policy: alfredo_tui::tasks::WorkPolicy {
                    files: vec!["calc.py".into()],
                    check: vec!["true".into()],
                },
            }],
        },
    }
}

#[test]
fn v5_draft_migration_preserves_v4_bytes_and_rejects_invalid_or_old_version_data() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut old = Snapshot::capture(&App::new("fixture".into()), "default");
    old.schema_version = 4;
    store.save(&old).unwrap();
    let file = fixture.file();
    let original = fs::read(&file).unwrap();
    let mut current = old.clone();
    current.schema_version = 5;
    current.plan_draft = Some(saved_plan());
    let backup = file.with_extension("v4-backup");
    fs::write(&backup, b"conflict").unwrap();
    assert!(store.save(&current).is_err());
    assert_eq!(fs::read(&file).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&current).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(
        store.load().unwrap().unwrap().plan_draft,
        Some(saved_plan())
    );
    assert!(store.save(&old).is_err());
    let good = fs::read(&file).unwrap();
    for version in 1..=4 {
        let mut invalid = current.clone();
        invalid.schema_version = version;
        assert!(store.save(&invalid).is_err());
        assert_eq!(fs::read(&file).unwrap(), good);
    }
    for bad in [serde_json::json!([]), serde_json::json!([{"title":"fake"}])] {
        let mut value = serde_json::to_value(&current).unwrap();
        value["plan_draft"]["plan"]["tasks"] = bad;
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(&file, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&file).unwrap(), bytes);
    }
    fs::write(&file, good).unwrap();
    let mut invalid = current;
    invalid.plan_draft.as_mut().unwrap().revision = u64::MAX;
    assert!(store.save(&invalid).is_err());
}

#[test]
fn v6_acceptance_draft_upgrade_preserves_v5_bytes_and_refuses_false_provenance() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut old = Snapshot::capture(&App::new("fixture".into()), "default");
    old.schema_version = 5;
    old.plan_draft = Some(saved_plan());
    store.save(&old).unwrap();
    let file = fixture.file();
    let original = fs::read(&file).unwrap();
    let mut new = old.clone();
    new.schema_version = 6;
    new.plan_draft.as_mut().unwrap().plan.tasks[0].acceptance = vec!["Observable result".into()];
    let backup = file.with_extension("v5-backup");
    fs::write(&backup, b"conflict").unwrap();
    assert!(store.save(&new).is_err());
    assert_eq!(fs::read(&file).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&new).unwrap();
    assert_eq!(fs::read(backup).unwrap(), original);
    assert_eq!(store.load().unwrap().unwrap().plan_draft, new.plan_draft);
    let committed = fs::read(&file).unwrap();
    new.schema_version = 5;
    assert!(store.save(&new).is_err());
    assert_eq!(fs::read(&file).unwrap(), committed);
    let forged_old = serde_json::to_vec(&new).unwrap();
    fs::write(&file, &forged_old).unwrap();
    assert!(store.load().unwrap_err().contains("schema v6"));
    assert_eq!(fs::read(file).unwrap(), forged_old);
}

#[test]
fn architect_draft_requires_v7_and_preserves_exact_v6_backup() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut old = Snapshot::capture(&App::new("fixture".into()), "default");
    old.schema_version = 6;
    old.plan_draft = Some(saved_plan());
    store.save(&old).unwrap();
    let original = fs::read(fixture.file()).unwrap();
    let mut new = old.clone();
    new.schema_version = 7;
    let plan = &mut new.plan_draft.as_mut().unwrap().plan;
    plan.tasks.truncate(1);
    plan.tasks[0].dependencies.clear();
    plan.tasks[0].acceptance = vec!["Revised observable contract".into()];
    plan.architecture = Some(alfredo_tui::architecture::Origin {
        task: 2,
        review_revision: 9,
        run: "task-2-run-7".into(),
        evidence_sha256: "a".repeat(64),
    });
    store.save(&new).unwrap();
    assert_eq!(
        fs::read(fixture.file().with_extension("v6-backup")).unwrap(),
        original
    );
    assert_eq!(store.load().unwrap().unwrap(), new);
    new.schema_version = 6;
    let bytes = serde_json::to_vec(&new).unwrap();
    fs::write(fixture.file(), &bytes).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
}

#[test]
fn observed_task_references_survive_restart_without_entering_model_requests() {
    use alfredo_tui::model::TaskReceiptRef;
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    let before = TaskReceiptRef {
        sequence: 0,
        after_messages: 0,
        revision: 1,
        task: 1,
        correlation: "TASK_RECEIPT_ONLY_SENTINEL_1".into(),
    };
    let between = TaskReceiptRef {
        sequence: 0,
        after_messages: 2,
        revision: 2,
        task: 1,
        correlation: "TASK_RECEIPT_ONLY_SENTINEL_2".into(),
    };
    assert!(app.sessions[0].observe_task_receipt(before.clone()));
    assert!(!app.sessions[0].observe_task_receipt(before.clone()));
    app.sessions[0].insert("First question");
    let first_request = app.sessions[0].begin().unwrap();
    assert_eq!(first_request.len(), 1);
    app.sessions[0].apply(1, Update::Token("First answer".into()));
    app.sessions[0].apply(1, Update::Done);
    assert!(app.sessions[0].observe_task_receipt(between.clone()));
    app.sessions[0].insert("Second question");
    let request = app.sessions[0].begin().unwrap();
    assert_eq!(request.len(), 3);
    assert!(!serde_json::to_string(&request)
        .unwrap()
        .contains("TASK_RECEIPT_ONLY_SENTINEL"));
    app.sessions[0].apply(2, Update::Token("Partial reply".into()));
    let refs = app.sessions[0].task_receipts().to_vec();
    assert_eq!(
        refs.iter()
            .map(|reference| reference.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    drop(store);
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.schema_version, 17);
    let mut restored = loaded.restore();
    assert_eq!(restored.sessions[0].task_receipts(), refs);
    assert_eq!(restored.sessions[0].messages.len(), 4);
    assert!(matches!(restored.sessions[0].status, Status::Failed(_)));
    let retried = restored.sessions[0].retry().unwrap();
    assert_eq!(retried, request);
    assert_eq!(restored.sessions[0].task_receipts(), refs);
    assert!(!serde_json::to_string(&retried)
        .unwrap()
        .contains("TASK_RECEIPT_ONLY_SENTINEL"));
    assert!(
        fixture.tasks.snapshot().unwrap().tasks.is_empty(),
        "Presentation references never replay task actions"
    );
}

#[test]
fn task_reference_schema_v8_preserves_exact_v7_backup_and_refuses_old_version_payloads() {
    use alfredo_tui::model::TaskReceiptRef;
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    let mut legacy = Snapshot::capture(&app, "default");
    legacy.schema_version = 7;
    store.save(&legacy).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    assert!(store.load().unwrap().unwrap().sessions[0]
        .task_receipts()
        .is_empty());
    assert!(app.sessions[0].observe_task_receipt(TaskReceiptRef {
        sequence: 0,
        after_messages: 0,
        revision: 3,
        task: 2,
        correlation: "observed-new-receipt".into()
    }));
    let mut historical = serde_json::to_value(Snapshot::capture(&app, "default")).unwrap();
    historical["schema_version"] = 8.into();
    historical["sessions"][0]["task_receipts"][0]["sequence"] = 0.into();
    let current: Snapshot = serde_json::from_value(historical).unwrap();
    let backup = path.with_extension("v7-backup");
    fs::write(&backup, b"conflicting backup").unwrap();
    assert!(store.save(&current).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&current).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(store.load().unwrap().unwrap(), current);
    for version in 1..=7 {
        let mut downgraded = current.clone();
        downgraded.schema_version = version;
        let bytes = serde_json::to_vec(&downgraded).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(
            store.load().is_err(),
            "Schema {version} cannot claim observed references"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn invalid_task_reference_anchors_order_identity_and_capacity_preserve_saved_bytes() {
    use alfredo_tui::model::TaskReceiptRef;
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("Question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Done);
    for reference in [
        TaskReceiptRef {
            sequence: 0,
            after_messages: 0,
            revision: 1,
            task: 1,
            correlation: "first".into(),
        },
        TaskReceiptRef {
            sequence: 0,
            after_messages: 2,
            revision: 2,
            task: 1,
            correlation: "second".into(),
        },
    ] {
        assert!(app.sessions[0].observe_task_receipt(reference));
    }
    let snapshot = Snapshot::capture(&app, "default");
    store.save(&snapshot).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    let valid: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let mut invalid = Vec::new();
    for (index, field, value) in [
        (0, "after_messages", serde_json::json!(1)),
        (1, "after_messages", serde_json::json!(4)),
        (0, "revision", serde_json::json!(0)),
        (0, "task", serde_json::json!(0)),
        (1, "revision", serde_json::json!(1)),
        (1, "correlation", serde_json::json!("first")),
        (0, "correlation", serde_json::json!("")),
        (0, "correlation", serde_json::json!("bad\ncorrelation")),
        (0, "correlation", serde_json::json!("x".repeat(1024))),
        (0, "unknown", serde_json::json!(true)),
    ] {
        let mut value_with_error = valid.clone();
        value_with_error["sessions"][0]["task_receipts"][index][field] = value;
        invalid.push(value_with_error);
    }
    let mut descending_anchor = valid.clone();
    descending_anchor["sessions"][0]["task_receipts"][0]["after_messages"] = 2.into();
    descending_anchor["sessions"][0]["task_receipts"][1]["after_messages"] = 0.into();
    invalid.push(descending_anchor);
    let mut descending_revision = valid.clone();
    descending_revision["sessions"][0]["task_receipts"][0]["revision"] = 3.into();
    invalid.push(descending_revision);
    let mut overflow = valid.clone();
    overflow["sessions"][0]["task_receipts"] = serde_json::Value::Array((1..=4097).map(|revision| serde_json::json!({"after_messages":0,"revision":revision,"task":1,"correlation":format!("receipt-{revision}")})).collect());
    invalid.push(overflow);
    for bad in invalid {
        let bytes = serde_json::to_vec(&bad).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, &original).unwrap();
    assert_eq!(store.load().unwrap().unwrap(), snapshot);
}

#[test]
fn keyed_receipt_anchor_survives_unrendered_retry_growth_and_restart() {
    use alfredo_tui::{
        model::TaskReceiptRef,
        reading::{Block, BlockKey},
    };
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    let session = &mut app.sessions[0];
    session.insert("Question");
    session.begin().unwrap();
    session.apply(1, Update::Failed("disconnect".into()));
    for revision in 1..=20 {
        assert!(session.observe_task_receipt(TaskReceiptRef {
            sequence: 0,
            after_messages: 2,
            revision,
            task: 1,
            correlation: format!("receipt-{revision}")
        }));
    }
    session.retry().unwrap();
    let blocks = |assistant_len| {
        let mut blocks = vec![
            Block {
                key: BlockKey::Message(0),
                start: 0,
                len: 3,
            },
            Block {
                key: BlockKey::Message(1),
                start: 3,
                len: assistant_len,
            },
        ];
        blocks.extend((1..=20).map(|revision| Block {
            key: BlockKey::TaskReceipt(revision),
            start: 3 + assistant_len + (revision as usize - 1) * 3,
            len: 3,
        }));
        blocks
    };
    session.scroll.set(20);
    let before = session.reading_position_blocks(&[1; 65], 10, &blocks(2));
    assert_eq!(before.line, 35);
    let key = serde_json::to_value(&*session).unwrap()["reading"]["block_anchor"].clone();
    assert_eq!(
        key["key"],
        serde_json::json!({"kind":"task-receipt","id":11})
    );
    session.apply(
        2,
        Update::Token("long hidden streaming output ".repeat(300)),
    );
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    drop(store);
    let reopened = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let restored = reopened.load().unwrap().unwrap().restore();
    let mut heights = vec![1; 66];
    heights[4] = 200;
    let after = restored.sessions[0].reading_position_blocks(&heights, 13, &blocks(3));
    assert_eq!(after.line, 36);
    assert_eq!(after.row, before.row);
    assert_eq!(
        serde_json::to_value(&restored.sessions[0]).unwrap()["reading"]["block_anchor"],
        key
    );
    assert!(matches!(restored.sessions[0].status, Status::Failed(_)));
}

#[test]
fn keyed_reading_schema_v9_preserves_v8_bytes_and_rejects_invalid_block_identity() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("History");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("HISTORY_LINE\n".repeat(80)));
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].scroll.set(20);
    app.sessions[0].reading_position(&[1; 85], 10);
    let mut old = Snapshot::capture(&app, "default");
    old.schema_version = 8;
    store.save(&old).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    let restored = store.load().unwrap().unwrap().restore();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &restored))
        .unwrap();
    let mut current = Snapshot::capture(&restored, "default");
    current.schema_version = 9;
    assert_eq!(current.schema_version, 9);
    let good = serde_json::to_value(&current).unwrap();
    assert_eq!(
        good["sessions"][0]["reading"]["block_anchor"]["key"],
        serde_json::json!({"kind":"message","id":1})
    );
    let backup = path.with_extension("v8-backup");
    fs::write(&backup, b"conflict").unwrap();
    assert!(store.save(&current).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&current).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    for invalid in [
        serde_json::json!({"key":{"kind":"message","id":99},"line":0,"row":0}),
        serde_json::json!({"key":{"kind":"task-receipt","id":1},"line":0,"row":0}),
        serde_json::json!({"key":{"kind":"unknown","id":1},"line":0,"row":0}),
        serde_json::json!({"key":{"kind":"message","id":1},"line":999999,"row":0}),
        serde_json::json!({"key":{"kind":"message","id":1},"line":0,"row":-1}),
        serde_json::json!({"key":{"kind":"message","id":1},"line":0,"row":999999}),
        serde_json::json!({"key":{"kind":"message","id":1,"extra":true},"line":0,"row":0}),
        serde_json::json!({"line":0,"row":0}),
    ] {
        let mut bad = good.clone();
        bad["sessions"][0]["reading"]["block_anchor"] = invalid;
        let bytes = serde_json::to_vec(&bad).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    for version in 1..=8 {
        let mut bad = good.clone();
        bad["schema_version"] = version.into();
        let bytes = serde_json::to_vec(&bad).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

fn command_intent(correlation: &str) -> alfredo_tui::command_intent::Intent {
    alfredo_tui::command_intent::Intent::Task {
        request: alfredo_tui::tasks::Request {
            correlation: correlation.into(),
            expected_revision: 0,
            action: alfredo_tui::tasks::Action::Propose {
                title: "Proposed command task".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        },
    }
}

#[test]
fn older_completed_autosave_cannot_release_newer_or_changed_command_intent() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut save = Autosave::new(store);
    let mut app = App::new("fixture".into());
    app.add_session();
    app.sessions[0]
        .submit_command("/task First".into(), command_intent("first-request"))
        .unwrap();
    let first = app.sessions[0].commands()[0].clone();
    assert!(!save.contains_saved_command(0, &first));
    assert!(!save
        .checkpoint(&runtime, &app, Default::default(), None)
        .unwrap());
    app.sessions[1]
        .submit_command("/task Second".into(), command_intent("second-request"))
        .unwrap();
    let second = app.sessions[1].commands()[0].clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if save
            .checkpoint(&runtime, &app, Default::default(), None)
            .unwrap()
        {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    // The boolean reports the older save, while a save of the second intent is
    // merely scheduled (even if its worker already finished in the background).
    assert!(save.contains_saved_command(0, &first));
    assert!(!save.contains_saved_command(1, &second));
    assert!(!save.contains_saved_command(0, &second));
    while !save.contains_saved_command(1, &second) {
        save.checkpoint(&runtime, &app, Default::default(), None)
            .unwrap();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(!save.contains_saved_command(0, &second));
    assert!(!save.contains_saved_command(99, &second));
    app.sessions[1].set_command_state(
        &second.id,
        alfredo_tui::console_command::CommandState::Submitted,
    );
    app.sessions[1].retry_command(&second.id).unwrap();
    let retry = app.sessions[1].commands()[0].clone();
    assert_eq!(retry.id, second.id);
    assert_eq!(retry.intent, second.intent);
    assert_eq!(retry.attempt, second.attempt + 1);
    assert!(!save.contains_saved_command(1, &retry));
    save.checkpoint(&runtime, &app, Default::default(), None)
        .unwrap();
    assert!(!save.contains_saved_command(1, &retry));
    while !save.contains_saved_command(1, &retry) {
        save.checkpoint(&runtime, &app, Default::default(), None)
            .unwrap();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(!save.contains_saved_command(1, &second));
    let mut changed = retry.clone();
    changed.intent = command_intent("changed-request");
    assert!(!save.contains_saved_command(1, &changed));
    changed = retry.clone();
    changed.text.push_str(" changed");
    assert!(!save.contains_saved_command(1, &changed));
    changed = retry.clone();
    changed.attempt += 1;
    assert!(
        !save.contains_saved_command(1, &changed),
        "An older saved Pending attempt cannot authorize an explicit retry"
    );
    changed = retry.clone();
    changed.state = alfredo_tui::console_command::CommandState::Submitted;
    assert!(!save.contains_saved_command(1, &changed));
    assert!(
        fixture.tasks.snapshot().unwrap().tasks.is_empty(),
        "Saving intent never dispatches task work"
    );
}

#[test]
fn failed_command_checkpoint_never_crosses_durable_barrier_and_preserves_newer_draft() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut app = App::new("fixture".into());
    let mut save = Autosave::new(store);
    save.finish(&runtime, &app, Default::default(), None)
        .unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    app.sessions[0]
        .submit_command("/task Durable first".into(), command_intent("failed-save"))
        .unwrap();
    let command = app.sessions[0].commands()[0].clone();
    app.sessions[0].insert("Newer unsent draft");
    save.checkpoint(&runtime, &app, Default::default(), None)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match save.checkpoint(&runtime, &app, Default::default(), None) {
            Err(_) => break,
            Ok(_) => {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
    assert!(!save.contains_saved_command(0, &command));
    assert_eq!(app.sessions[0].draft, "Newer unsent draft");
    assert_eq!(app.sessions[0].commands(), std::slice::from_ref(&command));
    fs::remove_dir(&path).unwrap();
    fs::write(&path, original).unwrap();
    save.finish(&runtime, &app, Default::default(), None)
        .unwrap();
    assert!(save.contains_saved_command(0, &command));
}

#[test]
fn pending_and_submitted_commands_restore_unknown_without_replay_or_model_input() {
    use alfredo_tui::console_command::CommandState;
    for state in [CommandState::Pending, CommandState::Submitted] {
        let fixture = Fixture::new();
        let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
        let mut app = App::new("fixture".into());
        app.sessions[0]
            .submit_command(
                "/task COMMAND_TEXT_SENTINEL".into(),
                command_intent("command-intent-sentinel"),
            )
            .unwrap();
        let intent = app.sessions[0].commands()[0].intent.clone();
        let mut value = serde_json::to_value(Snapshot::capture(&app, "default")).unwrap();
        value["sessions"][0]["commands"][0]["state"] = serde_json::to_value(state).unwrap();
        let saved: Snapshot = serde_json::from_value(value).unwrap();
        store.save(&saved).unwrap();
        drop(store);
        let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
        let mut restored = store.load().unwrap().unwrap().restore();
        let command = &restored.sessions[0].commands()[0];
        assert_eq!(command.intent, intent);
        assert!(matches!(command.state, CommandState::Unknown { .. }));
        restored.sessions[0].insert("Ordinary model question");
        let request = restored.sessions[0].begin().unwrap();
        assert_eq!(request.len(), 1);
        assert_eq!(request[0].content, "Ordinary model question");
        assert!(!serde_json::to_string(&request)
            .unwrap()
            .contains("SENTINEL"));
        assert!(fixture.tasks.snapshot().unwrap().tasks.is_empty());
    }
}

#[test]
fn command_schema_v10_preserves_v9_backup_and_refuses_old_or_malformed_commands() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    let mut old = Snapshot::capture(&app, "default");
    old.schema_version = 9;
    store.save(&old).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    app.sessions[0]
        .submit_command(
            "/task Recorded intent".into(),
            command_intent("migration-command"),
        )
        .unwrap();
    let mut current = Snapshot::capture(&app, "default");
    current.schema_version = 10;
    assert_eq!(current.schema_version, 10);
    let backup = path.with_extension("v9-backup");
    fs::write(&backup, b"conflict").unwrap();
    assert!(store.save(&current).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&current).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    let good = serde_json::to_value(&current).unwrap();
    for version in 1..=9 {
        let mut invalid = good.clone();
        invalid["schema_version"] = version.into();
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    let mut bad_values = vec![];
    for (field, value) in [
        ("id", serde_json::json!("bad\nid")),
        ("sequence", serde_json::json!(0)),
        ("after_messages", serde_json::json!(1)),
        ("text", serde_json::json!("x".repeat(128 * 1024))),
        ("state", serde_json::json!({"kind":"acknowledged"})),
        ("unexpected", serde_json::json!(true)),
    ] {
        let mut invalid = good.clone();
        invalid["sessions"][0]["commands"][0][field] = value;
        bad_values.push(invalid);
    }
    let mut duplicate = good.clone();
    let entry = duplicate["sessions"][0]["commands"][0].clone();
    duplicate["sessions"][0]["commands"]
        .as_array_mut()
        .unwrap()
        .push(entry);
    bad_values.push(duplicate);
    for invalid in bad_values {
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn command_reading_identity_survives_restart_and_rejects_unowned_keys() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    for index in 1..=12 {
        app.sessions[0]
            .submit_command(
                format!("/task Command {index}"),
                command_intent(&format!("reading-command-{index}")),
            )
            .unwrap();
    }
    let render = |app: &App| {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| alfredo_tui::ui::draw(frame, app))
            .unwrap();
    };
    render(&app);
    app.sessions[0].scroll_rows(-12);
    render(&app);
    let key =
        serde_json::to_value(&app.sessions[0]).unwrap()["reading"]["block_anchor"]["key"].clone();
    assert_eq!(key["kind"], "command");
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    let path = fixture.file();
    let saved = fs::read(&path).unwrap();
    let restored = store.load().unwrap().unwrap().restore();
    render(&restored);
    assert_eq!(
        serde_json::to_value(&restored.sessions[0]).unwrap()["reading"]["block_anchor"]["key"],
        key
    );
    let mut invalid: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    invalid["sessions"][0]["reading"]["block_anchor"]["key"]["id"] = 99.into();
    let bytes = serde_json::to_vec(&invalid).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn full_command_history_reserves_room_for_long_uncertainty_and_retry_metadata() {
    use alfredo_tui::{console_command::CommandState, model::MAX_DRAFT};
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    let mut next = 0;
    loop {
        let result = app.sessions[0].submit_command(
            "x".repeat(MAX_DRAFT),
            command_intent(&format!("capacity-{next}")),
        );
        if result.is_err() {
            break;
        }
        next += 1;
        assert!(next < 512);
    }
    // Fill the remaining admission budget through the public API, independent
    // of its internal byte accounting. The final tiny command must not fit.
    let mut lower = 0;
    let mut upper = MAX_DRAFT;
    while lower < upper {
        let middle = lower + (upper - lower).div_ceil(2);
        let mut trial = app.sessions[0].clone();
        if trial
            .submit_command(
                "x".repeat(middle),
                command_intent(&format!("capacity-{next}")),
            )
            .is_ok()
        {
            lower = middle;
        } else {
            upper = middle - 1;
        }
    }
    if lower > 0 {
        app.sessions[0]
            .submit_command(
                "x".repeat(lower),
                command_intent(&format!("capacity-{next}")),
            )
            .unwrap();
        next += 1;
    }
    assert!(app.sessions[0]
        .submit_command("x".into(), command_intent(&format!("capacity-{next}")))
        .is_err());
    assert!(app.sessions[0].commands().len() > 20);
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    let ids: Vec<_> = app.sessions[0]
        .commands()
        .iter()
        .map(|command| command.id.clone())
        .collect();
    for id in &ids {
        for _ in 0..9 {
            app.sessions[0].retry_command(id).unwrap();
        }
        assert!(app.sessions[0].set_command_state(
            id,
            CommandState::Unknown {
                reason: "\"".repeat(256)
            }
        ));
    }
    assert!(app.sessions[0]
        .commands()
        .iter()
        .all(|command| command.attempt == 10));
    let expanded = Snapshot::capture(&app, "default");
    store.save(&expanded).unwrap();
    let restored = store.load().unwrap().unwrap().restore();
    assert_eq!(restored.sessions[0].commands(), app.sessions[0].commands());
    assert!(fixture.tasks.snapshot().unwrap().tasks.is_empty());
}

#[test]
fn command_order_and_anchor_order_must_match_saved_chronology() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0]
        .submit_command("/task Before".into(), command_intent("ordered-first"))
        .unwrap();
    app.sessions[0].insert("Question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0]
        .submit_command("/task After".into(), command_intent("ordered-second"))
        .unwrap();
    let snapshot = Snapshot::capture(&app, "default");
    store.save(&snapshot).unwrap();
    let path = fixture.file();
    let good = serde_json::to_value(&snapshot).unwrap();
    let mut reversed_entries = good.clone();
    reversed_entries["sessions"][0]["commands"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let mut reversed_anchors = good.clone();
    reversed_anchors["sessions"][0]["commands"][0]["after_messages"] = 2.into();
    reversed_anchors["sessions"][0]["commands"][1]["after_messages"] = 0.into();
    for invalid in [reversed_entries, reversed_anchors] {
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn same_command_cannot_claim_two_saved_session_origins() {
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0]
        .submit_command("/task Original".into(), command_intent("unique-origin"))
        .unwrap();
    app.add_session();
    let valid = Snapshot::capture(&app, "default");
    store.save(&valid).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    app.sessions[1]
        .submit_command("/task Original".into(), command_intent("unique-origin"))
        .unwrap();
    let duplicated = Snapshot::capture(&app, "default");
    assert!(store.save(&duplicated).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    let bytes = serde_json::to_vec(&duplicated).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn worker_result_reading_line_requires_schema11_and_preserves_exact_v10_backup() {
    use alfredo_tui::{
        command_intent::Intent,
        reading::{Block, BlockKey},
    };
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0]
        .submit_command(
            "/run 1".into(),
            Intent::Run {
                correlation: "reading-run".into(),
                expected_revision: 3,
                task: 1,
            },
        )
        .unwrap();
    app.sessions[0].insert("Question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("History\n".repeat(20)));
    app.sessions[0].apply(1, Update::Done);
    let mut old = Snapshot::capture(&app, "default");
    old.schema_version = 10;
    store.save(&old).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    let sequence = app.sessions[0].commands()[0].sequence;
    app.sessions[0].scroll.set(21);
    let position = app.sessions[0].reading_position_blocks(
        &[1; 30],
        5,
        &[
            Block {
                key: BlockKey::Command(sequence),
                start: 0,
                len: 5,
            },
            Block {
                key: BlockKey::Message(0),
                start: 5,
                len: 3,
            },
            Block {
                key: BlockKey::Message(1),
                start: 8,
                len: 22,
            },
        ],
    );
    assert_eq!(position.line, 4);
    let current = Snapshot::capture(&app, "default");
    assert_eq!(current.schema_version, 17);
    let value = serde_json::to_value(&current).unwrap();
    assert_eq!(value["sessions"][0]["reading"]["block_anchor"]["line"], 4);
    let backup = path.with_extension("v10-backup");
    fs::write(&backup, b"conflict").unwrap();
    assert!(store.save(&current).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&current).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(store.load().unwrap().unwrap(), current);
    let mut downgraded = value;
    downgraded["schema_version"] = 10.into();
    let bytes = serde_json::to_vec(&downgraded).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn planner_schema12_preserves_v11_and_restores_only_exact_saved_draft_origin() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::CommandState,
        planner_command::{Operation, Outcome, Request},
    };
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    let mut old = Snapshot::capture(&app, "default");
    old.schema_version = 11;
    store.save(&old).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    let request = Request {
        correlation: "planner-origin".into(),
        operation: Operation::Generate {
            prompt: "Implement calculation".into(),
            model: "fixture".into(),
            revision: 0,
            base_sha256: None,
        },
    };
    let id = app.sessions[0]
        .submit_command(
            "/plan Implement calculation".into(),
            Intent::Planner {
                request: request.clone(),
            },
        )
        .unwrap();
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    app.add_session();
    let unrelated = Request {
        correlation: "unrelated-planner".into(),
        ..request.clone()
    };
    let other = app.sessions[1]
        .submit_command(
            "/plan Implement calculation".into(),
            Intent::Planner { request: unrelated },
        )
        .unwrap();
    let mut draft = saved_plan();
    draft.origin = Some(request.clone());
    let mut snapshot = Snapshot::capture(&app, "default");
    snapshot.plan_draft = Some(draft.clone());
    assert_eq!(snapshot.schema_version, 17);
    let backup = path.with_extension("v11-backup");
    fs::write(&backup, b"conflict").unwrap();
    assert!(store.save(&snapshot).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&snapshot).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    let mut restored = store.load().unwrap().unwrap().restore();
    assert_eq!(restored.selected, 1);
    assert_eq!(
        restored.sessions[0].commands()[0].state,
        CommandState::Planner {
            outcome: Outcome::Generated {
                draft_sha256: draft.digest().unwrap(),
                tasks: 1
            }
        }
    );
    assert!(matches!(
        restored.sessions[1].commands()[0].state,
        CommandState::Unknown { .. }
    ));
    let terminal = restored.sessions[0].commands()[0].clone();
    assert!(restored.sessions[0].retry_command(&id).is_err());
    assert_eq!(restored.sessions[0].commands()[0], terminal);
    assert!(restored.sessions[1].retry_command(&other).is_ok());
    assert!(restored.sessions.iter().all(|s| s.messages.is_empty()));
    let baseline = fs::read(&path).unwrap();
    let mut downgraded = snapshot.clone();
    downgraded.schema_version = 11;
    assert!(store.save(&downgraded).is_err());
    assert_eq!(fs::read(&path).unwrap(), baseline);
    let mut unmatched = snapshot.clone();
    unmatched
        .plan_draft
        .as_mut()
        .unwrap()
        .origin
        .as_mut()
        .unwrap()
        .correlation = "missing-origin".into();
    assert!(store.save(&unmatched).is_err());
    assert_eq!(fs::read(&path).unwrap(), baseline);
    let mut mismatched_result = snapshot.clone();
    mismatched_result.sessions[0].set_command_state(
        &id,
        CommandState::Planner {
            outcome: Outcome::Generated {
                draft_sha256: "a".repeat(64),
                tasks: 1,
            },
        },
    );
    assert!(store.save(&mismatched_result).is_err());
    assert_eq!(fs::read(&path).unwrap(), baseline);
    let mut no_draft = snapshot.clone();
    no_draft.plan_draft = None;
    let interrupted = no_draft.restore();
    assert!(matches!(
        interrupted.sessions[0].commands()[0].state,
        CommandState::Unknown { .. }
    ));
    let terminal_snapshot = Snapshot::capture(&restored, "default");
    store.save(&terminal_snapshot).unwrap();
    let mut old_outcome = terminal_snapshot.clone();
    old_outcome.schema_version = 11;
    assert!(store.save(&old_outcome).is_err());
    assert_eq!(store.load().unwrap().unwrap(), terminal_snapshot);
}

#[test]
fn controller_schema13_preserves_v12_and_never_reactivates_restored_control() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::CommandState,
        control_command::{Operation, Outcome, Request},
        task_control::TaskControl,
    };
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    let mut old = Snapshot::capture(&app, "default");
    old.schema_version = 12;
    store.save(&old).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    let request = Request {
        correlation: "enable-controller".into(),
        controller: "controller-one".into(),
        operation: Operation::Dispatch {
            enabled: true,
            expected_epoch: 0,
            scope_revision_for_on: Some(0),
        },
    };
    let id = app.sessions[0]
        .submit_command(
            "/dispatch on".into(),
            Intent::Control {
                request: request.clone(),
            },
        )
        .unwrap();
    app.sessions[0].set_command_state(
        &id,
        CommandState::Control {
            outcome: Outcome::DispatchChanged { enabled: true },
        },
    );
    app.add_session();
    for (correlation, state) in [
        ("pending-control", CommandState::Pending),
        ("submitted-control", CommandState::Submitted),
    ] {
        let intent = Intent::Control {
            request: Request {
                correlation: correlation.into(),
                ..request.clone()
            },
        };
        let id = app.sessions[1]
            .submit_command("/dispatch on".into(), intent)
            .unwrap();
        app.sessions[1].set_command_state(&id, state);
    }
    let snapshot = Snapshot::capture(&app, "default");
    assert_eq!(snapshot.schema_version, 17);
    let backup = path.with_extension("v12-backup");
    fs::write(&backup, b"conflict").unwrap();
    assert!(store.save(&snapshot).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&snapshot).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    let mut restored = store.load().unwrap().unwrap().restore();
    assert_eq!(restored.selected, 1);
    assert_eq!(
        restored.sessions[0].commands()[0],
        app.sessions[0].commands()[0]
    );
    assert!(restored.sessions[1]
        .commands()
        .iter()
        .all(|command| matches!(command.state, CommandState::Unknown { .. })));
    assert!(restored
        .sessions
        .iter()
        .all(|session| session.messages.is_empty()));
    assert!(restored.sessions[0].retry_command(&id).is_err());
    let mut restarted = TaskControl::new(fixture.tasks.clone());
    assert!(!restarted.dispatch.enabled);
    assert!(restarted.take_control_events().is_empty());
    let preserved = fs::read(&path).unwrap();
    let mut old = snapshot.clone();
    old.schema_version = 12;
    assert!(store.save(&old).is_err());
    assert_eq!(fs::read(&path).unwrap(), preserved);
    let mut mismatched = snapshot.clone();
    mismatched.sessions[0].set_command_state(
        &id,
        CommandState::Control {
            outcome: Outcome::CancellationRequested,
        },
    );
    assert!(store.save(&mismatched).is_err());
    assert_eq!(fs::read(&path).unwrap(), preserved);
    let mut wrongvalue = snapshot.clone();
    wrongvalue.sessions[0].set_command_state(
        &id,
        CommandState::Control {
            outcome: Outcome::DispatchChanged { enabled: false },
        },
    );
    assert!(store.save(&wrongvalue).is_err());
    assert_eq!(fs::read(&path).unwrap(), preserved);
    let bytes = serde_json::to_vec(&old).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn cancellation_result_anchor_requires_its_schema13_command_and_stays_bounded() {
    use alfredo_tui::{
        command_intent::Intent,
        control_command::{Operation, Request},
        reading::{Block, BlockKey},
    };
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0]
        .submit_command(
            "/cancel-task 1".into(),
            Intent::Control {
                request: Request {
                    correlation: "cancel-reading".into(),
                    controller: "controller-reading".into(),
                    operation: Operation::CancelWorker {
                        task: 1,
                        start_correlation: "start-reading".into(),
                        expected_start_revision: 4,
                    },
                },
            },
        )
        .unwrap();
    app.sessions[0].insert("Question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("History\n".repeat(20)));
    app.sessions[0].apply(1, Update::Done);
    let sequence = app.sessions[0].commands()[0].sequence;
    app.sessions[0].scroll.set(21);
    let position = app.sessions[0].reading_position_blocks(
        &[1; 30],
        5,
        &[
            Block {
                key: BlockKey::Command(sequence),
                start: 0,
                len: 5,
            },
            Block {
                key: BlockKey::Message(0),
                start: 5,
                len: 3,
            },
            Block {
                key: BlockKey::Message(1),
                start: 8,
                len: 22,
            },
        ],
    );
    assert_eq!(position.line, 4);
    let saved = Snapshot::capture(&app, "default");
    store.save(&saved).unwrap();
    assert_eq!(store.load().unwrap().unwrap(), saved);
    let path = fixture.file();
    let valid = serde_json::to_value(&saved).unwrap();
    let mut wrong_schema = valid.clone();
    wrong_schema["schema_version"] = 12.into();
    let mut past_end = valid.clone();
    past_end["sessions"][0]["reading"]["block_anchor"]["line"] = 5.into();
    let mut nonexistent_owner = valid.clone();
    nonexistent_owner["sessions"][0]["reading"]["block_anchor"]["key"]["id"] = 999.into();
    for invalid in [wrong_schema, past_end, nonexistent_owner] {
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, serde_json::to_vec(&valid).unwrap()).unwrap();
}

fn dispatch_source() -> alfredo_tui::control_command::Request {
    alfredo_tui::control_command::Request {
        correlation: "saved-enable-source".into(),
        controller: "live-controller".into(),
        operation: alfredo_tui::control_command::Operation::Dispatch {
            enabled: true,
            expected_epoch: 0,
            scope_revision_for_on: Some(0),
        },
    }
}
fn dispatch_run() -> alfredo_tui::command_intent::Intent {
    alfredo_tui::command_intent::Intent::DispatchRun {
        request: alfredo_tui::dispatch::RunRequest {
            correlation: "automatic-run".into(),
            expected_revision: 3,
            task: 1,
            approval_revision: 3,
            source: dispatch_source(),
        },
    }
}
fn save_dispatch_parent(app: &mut App) -> String {
    use alfredo_tui::{
        command_intent::Intent, console_command::CommandState, control_command::Outcome,
    };
    let id = app.sessions[0]
        .submit_command(
            "/dispatch on".into(),
            Intent::Control {
                request: dispatch_source(),
            },
        )
        .unwrap();
    app.sessions[0].set_command_state(
        &id,
        CommandState::Control {
            outcome: Outcome::DispatchChanged { enabled: true },
        },
    );
    id
}

#[test]
fn automatic_run_schema14_requires_its_exact_earlier_enabled_parent_and_preserves_v13() {
    use alfredo_tui::{
        console_command::{CommandState, ConsoleCommand},
        reading::{Block, BlockKey},
    };
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    save_dispatch_parent(&mut app);
    let mut old = Snapshot::capture(&app, "default");
    old.schema_version = 13;
    store.save(&old).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    let id = app.sessions[0]
        .submit_automatic_command("Dispatch selected task #1".into(), dispatch_run())
        .unwrap();
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    app.sessions[0].insert("Question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("History\n".repeat(20)));
    app.sessions[0].apply(1, Update::Done);
    let sequence = app.sessions[0].commands()[1].sequence;
    app.sessions[0].scroll.set(21);
    app.sessions[0].reading_position_blocks(
        &[1; 34],
        5,
        &[
            Block {
                key: BlockKey::Command(1),
                start: 0,
                len: 4,
            },
            Block {
                key: BlockKey::Command(sequence),
                start: 4,
                len: 5,
            },
            Block {
                key: BlockKey::Message(0),
                start: 9,
                len: 3,
            },
            Block {
                key: BlockKey::Message(1),
                start: 12,
                len: 22,
            },
        ],
    );
    app.add_session();
    let snapshot = Snapshot::capture(&app, "default");
    assert_eq!(snapshot.schema_version, 17);
    let backup = path.with_extension("v13-backup");
    fs::write(&backup, b"conflicting backup").unwrap();
    assert!(store.save(&snapshot).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&snapshot).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(store.load().unwrap().unwrap(), snapshot);
    let mut restored = snapshot.clone().restore();
    assert_eq!(restored.selected, 1);
    assert!(matches!(
        restored.sessions[0].commands()[1].state,
        CommandState::Unknown { .. }
    ));
    let prior = restored.sessions[0].commands()[1].clone();
    assert!(restored.sessions[0].retry_command(&id).is_err());
    assert_eq!(restored.sessions[0].commands()[1], prior);
    let mut controller = alfredo_tui::task_control::TaskControl::new(fixture.tasks.clone());
    assert!(!controller.dispatch.enabled);
    assert!(controller.workers.is_empty());
    assert!(controller.take_control_events().is_empty());
    let good = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(good["sessions"][0]["reading"]["block_anchor"]["line"], 4);
    let mut orphan = good.clone();
    orphan["sessions"][0]["commands"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let mut cross_session = good.clone();
    let child = cross_session["sessions"][0]["commands"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    cross_session["sessions"][1]["commands"] = serde_json::json!([child]);
    cross_session["sessions"][0]
        .as_object_mut()
        .unwrap()
        .remove("reading");
    cross_session["sessions"][0]["scroll"] = 0.into();
    let mut pending_parent = good.clone();
    pending_parent["sessions"][0]["commands"][0]["state"] = serde_json::json!({"kind":"pending"});
    let mut changed_source = good.clone();
    changed_source["sessions"][0]["commands"][1]["intent"]["request"]["source"]["correlation"] =
        "other-enable".into();
    let intent =
        serde_json::from_value(changed_source["sessions"][0]["commands"][1]["intent"].clone())
            .unwrap();
    changed_source["sessions"][0]["commands"][1]["id"] = ConsoleCommand::identity(&intent).into();
    let mut wrong_parent_type = good.clone();
    let wrong_parent = command_intent("saved-enable-source");
    wrong_parent_type["sessions"][0]["commands"][0]["intent"] =
        serde_json::to_value(&wrong_parent).unwrap();
    wrong_parent_type["sessions"][0]["commands"][0]["id"] =
        ConsoleCommand::identity(&wrong_parent).into();
    wrong_parent_type["sessions"][0]["commands"][0]["state"] =
        serde_json::json!({"kind":"submitted"});
    let mut later_parent = good.clone();
    later_parent["sessions"][0]["commands"][0]["sequence"] = 3.into();
    let mut unsupported = good.clone();
    unsupported["schema_version"] = 13.into();
    let mut past_end = good.clone();
    past_end["sessions"][0]["reading"]["block_anchor"]["line"] = 5.into();
    for invalid in [
        orphan,
        cross_session,
        pending_parent,
        changed_source,
        wrong_parent_type,
        later_parent,
        unsupported,
        past_end,
    ] {
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, serde_json::to_vec(&good).unwrap()).unwrap();
}

#[test]
fn automatic_run_admission_preserves_streaming_reading_and_composer_without_inference_input() {
    let mut app = App::new("fixture".into());
    let parent = save_dispatch_parent(&mut app);
    app.sessions[0].insert("Explain the plan");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("A long reply\n".repeat(20)));
    app.sessions[0].insert("Unfinished next question");
    app.sessions[0].left();
    app.sessions[0].scroll.set(10);
    app.sessions[0].reading_position_blocks(
        &[1; 29],
        5,
        &[
            alfredo_tui::reading::Block {
                key: alfredo_tui::reading::BlockKey::Command(1),
                start: 0,
                len: 4,
            },
            alfredo_tui::reading::Block {
                key: alfredo_tui::reading::BlockKey::Message(0),
                start: 4,
                len: 3,
            },
            alfredo_tui::reading::Block {
                key: alfredo_tui::reading::BlockKey::Message(1),
                start: 7,
                len: 22,
            },
        ],
    );
    let before = serde_json::to_value(&app.sessions[0]).unwrap();
    app.sessions[0]
        .submit_automatic_command("Dispatch selected task #1".into(), dispatch_run())
        .unwrap();
    let after = serde_json::to_value(&app.sessions[0]).unwrap();
    for field in [
        "reading", "scroll", "draft", "cursor", "messages", "status", "attempt",
    ] {
        assert_eq!(
            before[field], after[field],
            "Automatic entry changed {field}"
        );
    }
    assert_eq!(app.sessions[0].commands()[1].after_messages, 2);
    assert_eq!(app.sessions[0].commands()[0].id, parent);
    app.sessions[0].apply(1, Update::Token("More streamed text".into()));
    assert_eq!(app.sessions[0].commands()[1].after_messages, 2);
    let held = app.sessions[0].commands().to_vec();
    assert!(app.sessions[0]
        .submit_automatic_command("/task Wrong".into(), command_intent("wrong-kind"))
        .is_err());
    assert_eq!(app.sessions[0].commands(), held);
    app.sessions[0]
        .submit_command(
            "/task Deliberate new command".into(),
            command_intent("user-new-command"),
        )
        .unwrap();
    assert_eq!(app.sessions[0].scroll.get(), 0);
    assert!(serde_json::to_value(&app.sessions[0])
        .unwrap()
        .get("reading")
        .is_none());
}

fn architect_request() -> alfredo_tui::planner_command::ArchitectRequest {
    use alfredo_tui::{assessment, planner_command, tasks};
    planner_command::ArchitectRequest {
        request: planner_command::Request {
            correlation: "automatic-architect".into(),
            operation: planner_command::Operation::Architect {
                origin: alfredo_tui::architecture::Origin {
                    task: 2,
                    review_revision: 5,
                    run: "task-2-run-3".into(),
                    evidence_sha256: "a".repeat(64),
                },
                revision: 5,
            },
        },
        source: tasks::Request {
            correlation: "review-source".into(),
            expected_revision: 4,
            action: tasks::Action::ReviewArchitecture {
                task: 2,
                decision: assessment::Decision {
                    outcome: assessment::Outcome::NeedsRepair,
                    reason: "Revise the design".into(),
                    criteria: vec![],
                    limitations: vec![],
                    risk: None,
                    failure: Some(assessment::FailureKind::Architecture),
                },
            },
        },
    }
}

#[test]
fn architect_schema15_preserves_v14_and_restores_only_the_saved_draft_owner() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::{CommandState, ConsoleCommand},
        planner_command::Operation,
    };
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let request = architect_request();
    let mut app = App::new("fixture".into());
    let parent_id = app.sessions[0]
        .submit_command(
            "/review 2 architecture".into(),
            Intent::Task {
                request: request.source.clone(),
            },
        )
        .unwrap();
    app.sessions[0].set_command_state(&parent_id, CommandState::Submitted);
    let mut old = Snapshot::capture(&app, "default");
    old.schema_version = 14;
    store.save(&old).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    let automatic = Intent::ArchitectDraft {
        request: request.clone(),
    };
    let id = app.sessions[0]
        .submit_automatic_command("Architect revision for task #2".into(), automatic)
        .unwrap();
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    app.add_session();
    let mut draft = saved_plan();
    draft.revision = 5;
    draft.origin = Some(request.request.clone());
    let Operation::Architect { origin, .. } = &request.request.operation else {
        unreachable!()
    };
    draft.plan.architecture = Some(origin.clone());
    draft.plan.tasks[0].acceptance = vec!["Preserve the calculation contract".into()];
    let mut snapshot = Snapshot::capture(&app, "default");
    snapshot.plan_draft = Some(draft.clone());
    let backup = path.with_extension("v14-backup");
    fs::write(&backup, b"conflicting backup").unwrap();
    assert!(store.save(&snapshot).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&snapshot).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(store.load().unwrap().unwrap(), snapshot);
    let mut restored = snapshot.clone().restore();
    assert_eq!(restored.selected, 1);
    assert!(matches!(
        restored.sessions[0].commands()[0].state,
        CommandState::Unknown { .. }
    ));
    assert_eq!(
        restored.sessions[0].commands()[1].state,
        CommandState::Planner {
            outcome: draft.outcome_for(&request.request).unwrap()
        }
    );
    assert!(restored.sessions[0].retry_command(&id).is_err());
    assert!(restored.sessions[1].commands().is_empty());
    assert!(restored
        .sessions
        .iter()
        .all(|session| session.messages.is_empty()));
    let mut no_draft = snapshot.clone();
    no_draft.plan_draft = None;
    let mut interrupted = no_draft.restore();
    assert!(matches!(
        interrupted.sessions[0].commands()[1].state,
        CommandState::Unknown { .. }
    ));
    assert!(interrupted.sessions[0].retry_command(&id).is_err());
    let good = serde_json::to_value(&snapshot).unwrap();
    let mut orphan = good.clone();
    orphan["sessions"][0]["commands"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let mut cross_session = good.clone();
    let child = cross_session["sessions"][0]["commands"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    cross_session["sessions"][1]["commands"] = serde_json::json!([child]);
    let mut wrong_source = good.clone();
    wrong_source["sessions"][0]["commands"][1]["intent"]["request"]["source"]["correlation"] =
        "unrelated-review".into();
    let changed_intent =
        serde_json::from_value(wrong_source["sessions"][0]["commands"][1]["intent"].clone())
            .unwrap();
    wrong_source["sessions"][0]["commands"][1]["id"] =
        ConsoleCommand::identity(&changed_intent).into();
    let mut wrong_type = good.clone();
    let other = command_intent("review-source");
    wrong_type["sessions"][0]["commands"][0]["intent"] = serde_json::to_value(&other).unwrap();
    wrong_type["sessions"][0]["commands"][0]["id"] = ConsoleCommand::identity(&other).into();
    let mut late_source = good.clone();
    late_source["sessions"][0]["commands"][0]["sequence"] = 3.into();
    let mut old_schema = good.clone();
    old_schema["schema_version"] = 14.into();
    for invalid in [
        orphan,
        cross_session,
        wrong_source,
        wrong_type,
        late_source,
        old_schema,
    ] {
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, serde_json::to_vec(&good).unwrap()).unwrap();
    let mut duplicate = Snapshot::capture(&app, "default").restore();
    assert!(duplicate.sessions[0]
        .submit_command(
            "/architect-revise 2".into(),
            Intent::Planner {
                request: request.request.clone()
            }
        )
        .is_err());
    duplicate.sessions[1]
        .submit_command(
            "/architect-revise 2".into(),
            Intent::Planner {
                request: request.request.clone(),
            },
        )
        .unwrap();
    let duplicate = Snapshot::capture(&duplicate, "default");
    assert!(store.save(&duplicate).is_err());
    let bytes = serde_json::to_vec(&duplicate).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn wayfinder_schema16_binds_delayed_scope_intent_to_its_turn_and_preserves_v15() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::{CommandState, ConsoleCommand},
        understanding::{Action, Flow, Mode, Request},
    };
    let fixture = Fixture::new();
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    let prompt = "Build a new project for tracking calculations";
    app.sessions[0].insert(prompt);
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Explain the existing code");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(2, Update::Token("Existing code details\n".repeat(20)));
    app.sessions[0].apply(2, Update::Done);
    app.sessions[0]
        .submit_command(
            "/task Later unrelated work".into(),
            command_intent("later-unrelated"),
        )
        .unwrap();
    app.sessions[0].insert("Preserve the next draft");
    app.sessions[0].left();
    app.sessions[0].scroll.set(10);
    app.sessions[0].reading_position_blocks(
        &[1; 34],
        5,
        &[
            alfredo_tui::reading::Block {
                key: alfredo_tui::reading::BlockKey::Message(0),
                start: 0,
                len: 3,
            },
            alfredo_tui::reading::Block {
                key: alfredo_tui::reading::BlockKey::Message(1),
                start: 3,
                len: 3,
            },
            alfredo_tui::reading::Block {
                key: alfredo_tui::reading::BlockKey::Message(2),
                start: 6,
                len: 3,
            },
            alfredo_tui::reading::Block {
                key: alfredo_tui::reading::BlockKey::Message(3),
                start: 9,
                len: 21,
            },
            alfredo_tui::reading::Block {
                key: alfredo_tui::reading::BlockKey::Command(1),
                start: 30,
                len: 4,
            },
        ],
    );
    let mut old = Snapshot::capture(&app, "default");
    old.schema_version = 15;
    store.save(&old).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    let before = serde_json::to_value(&app.sessions[0]).unwrap();
    let request = Request {
        correlation: "wayfinder-enter-turn".into(),
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
    let after = serde_json::to_value(&app.sessions[0]).unwrap();
    for field in [
        "messages", "draft", "cursor", "scroll", "reading", "status", "attempt",
    ] {
        assert_eq!(
            before[field], after[field],
            "Late Wayfinder admission changed {field}"
        );
    }
    assert_eq!(app.sessions[0].commands()[0].after_messages, 4);
    assert_eq!(app.sessions[0].commands()[1].after_messages, 2);
    assert_eq!(app.sessions[0].commands()[0].sequence, 1);
    assert_eq!(app.sessions[0].commands()[1].sequence, 2);
    assert!(!app.sessions[0].commands()[1].text.contains(prompt));
    app.add_session();
    let snapshot = Snapshot::capture(&app, "default");
    let backup = path.with_extension("v15-backup");
    fs::write(&backup, b"conflicting backup").unwrap();
    assert!(store.save(&snapshot).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&snapshot).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(store.load().unwrap().unwrap(), snapshot);
    let mut restored = snapshot.clone().restore();
    assert!(matches!(
        restored.sessions[0].commands()[1].state,
        CommandState::Unknown { .. }
    ));
    assert!(restored.sessions[0].retry_command(&id).is_ok());
    assert_eq!(restored.sessions[0].commands()[1].attempt, 2);
    assert!(matches!(
        restored.sessions[0].commands()[1].state,
        CommandState::Pending
    ));
    assert_eq!(
        serde_json::to_value(&restored.sessions[0]).unwrap()["messages"],
        before["messages"]
    );
    let good = serde_json::to_value(&snapshot).unwrap();
    let mut changed_prompt = good.clone();
    changed_prompt["sessions"][0]["messages"][0]["content"] = "A different project".into();
    let mut bad_offset = good.clone();
    bad_offset["sessions"][0]["commands"][1]["after_messages"] = 4.into();
    let mut wrong_turn = good.clone();
    wrong_turn["sessions"][0]["commands"][1]["intent"]["user_message"] = 1.into();
    let intent =
        serde_json::from_value(wrong_turn["sessions"][0]["commands"][1]["intent"].clone()).unwrap();
    wrong_turn["sessions"][0]["commands"][1]["id"] = ConsoleCommand::identity(&intent).into();
    let mut duplicate_turn = good.clone();
    let mut duplicate = duplicate_turn["sessions"][0]["commands"][1].clone();
    duplicate["intent"]["request"]["correlation"] = "second-operation-same-turn".into();
    let intent = serde_json::from_value(duplicate["intent"].clone()).unwrap();
    duplicate["id"] = ConsoleCommand::identity(&intent).into();
    duplicate["sequence"] = 3.into();
    duplicate_turn["sessions"][0]["commands"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    let mut backwards_ordinary = good.clone();
    let mut backwards = backwards_ordinary["sessions"][0]["commands"][0].clone();
    let intent = command_intent("backwards-ordinary");
    backwards["intent"] = serde_json::to_value(&intent).unwrap();
    backwards["id"] = ConsoleCommand::identity(&intent).into();
    backwards["after_messages"] = 0.into();
    backwards["sequence"] = 3.into();
    backwards_ordinary["sessions"][0]["commands"]
        .as_array_mut()
        .unwrap()
        .push(backwards);
    let mut unsupported = good.clone();
    unsupported["schema_version"] = 15.into();
    for invalid in [
        changed_prompt,
        bad_offset,
        wrong_turn,
        duplicate_turn,
        backwards_ordinary,
        unsupported,
    ] {
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, serde_json::to_vec(&good).unwrap()).unwrap();
    assert!(restored.sessions[0]
        .submit_command(
            "/scope duplicate".into(),
            Intent::Scope {
                request: request.clone()
            }
        )
        .is_err());
    restored.sessions[1]
        .submit_command("/scope duplicate".into(), Intent::Scope { request })
        .unwrap();
    let duplicate = Snapshot::capture(&restored, "default");
    assert!(store.save(&duplicate).is_err());
    assert_eq!(fs::read(&path).unwrap(), serde_json::to_vec(&good).unwrap());
    let bytes = serde_json::to_vec(&duplicate).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn selection_schema17_preserves_v16_and_requires_exact_source_identity_without_replay() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::{CommandState, ConsoleCommand},
        selection_command::{
            Choice, MissionChoice, Origin, Outcome, Phase, Request, WorkspaceChoice,
        },
    };
    let fixture = Fixture::new();
    fs::create_dir(fixture.root.join("destination")).unwrap();
    let request = Request {
        correlation: "selection-origin".into(),
        origin: Origin::Conversation {
            workspace: fixture.root.join("workspace").canonicalize().unwrap(),
            mission: "mission".into(),
            conversation: "default".into(),
            session: 0,
        },
        choice: Choice {
            workspace: WorkspaceChoice::Existing {
                path: fixture.root.join("destination").canonicalize().unwrap(),
            },
            mission: MissionChoice::Resume {
                name: "destination-mission".into(),
            },
        },
        conversation: "destination-chat".into(),
    };
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("Previous conversation");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("Retained history\n".repeat(20)));
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Preserve this unfinished draft");
    app.sessions[0].left();
    app.sessions[0].scroll.set(10);
    app.sessions[0].reading_position(&[1; 25], 5);
    let before = serde_json::to_value(&app.sessions[0]).unwrap();
    let mut old = Snapshot::capture(&app, "default");
    old.schema_version = 16;
    store.save(&old).unwrap();
    let path = fixture.file();
    let original = fs::read(&path).unwrap();
    let id = app.sessions[0]
        .submit_selection_command(
            "Select destination workspace and mission".into(),
            request.clone(),
        )
        .unwrap();
    let after = serde_json::to_value(&app.sessions[0]).unwrap();
    for field in [
        "draft", "cursor", "messages", "scroll", "reading", "status", "attempt",
    ] {
        assert_eq!(
            before[field], after[field],
            "Selection admission changed {field}"
        );
    }
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    app.add_session();
    let snapshot = Snapshot::capture(&app, "default");
    assert_eq!(snapshot.schema_version, 17);
    let backup = path.with_extension("v16-backup");
    fs::write(&backup, b"conflicting backup").unwrap();
    assert!(store.save(&snapshot).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&backup).unwrap();
    store.save(&snapshot).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(store.load().unwrap().unwrap(), snapshot);
    let mut restored = snapshot.clone().restore();
    assert_eq!(restored.selected, 1);
    assert!(matches!(
        restored.sessions[0].commands()[0].state,
        CommandState::Unknown { .. }
    ));
    assert!(restored.sessions[0].retry_command(&id).is_err());
    assert_eq!(restored.sessions[0].draft, "Preserve this unfinished draft");
    assert!(fixture.tasks.snapshot().unwrap().tasks.is_empty());
    let mut partial = snapshot.clone();
    partial.sessions[0].set_command_state(
        &id,
        CommandState::Selection {
            outcome: Outcome {
                phase: Phase::RepositoryReady,
                failure: Some("Target mission could not be prepared".into()),
            },
        },
    );
    store.save(&partial).unwrap();
    assert!(matches!(
        partial.clone().restore().sessions[0].commands()[0].state,
        CommandState::Selection { .. }
    ));
    let good = serde_json::to_value(&partial).unwrap();
    let mut invalids = vec![];
    for field in ["workspace", "mission", "conversation", "session"] {
        let mut wrong = good.clone();
        let origin = &mut wrong["sessions"][0]["commands"][0]["intent"]["request"]["origin"];
        origin[field] = match field {
            "workspace" => fixture
                .root
                .join("destination")
                .to_string_lossy()
                .to_string()
                .into(),
            "session" => 1.into(),
            _ => "another-identity".into(),
        };
        let intent =
            serde_json::from_value(wrong["sessions"][0]["commands"][0]["intent"].clone()).unwrap();
        wrong["sessions"][0]["commands"][0]["id"] = ConsoleCommand::identity(&intent).into();
        invalids.push(wrong);
    }
    let mut unsupported = good.clone();
    unsupported["schema_version"] = 16.into();
    invalids.push(unsupported);
    let mut wrong_domain = good.clone();
    let other = command_intent("selection-origin");
    wrong_domain["sessions"][0]["commands"][0]["intent"] = serde_json::to_value(&other).unwrap();
    wrong_domain["sessions"][0]["commands"][0]["id"] = ConsoleCommand::identity(&other).into();
    invalids.push(wrong_domain);
    for invalid in invalids {
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, serde_json::to_vec(&good).unwrap()).unwrap();
    let mut conflicting = request.clone();
    if let Origin::Conversation { session, .. } = &mut conflicting.origin {
        *session = 1;
    }
    restored.sessions[1]
        .submit_selection_command(
            "Another origin with the same correlation".into(),
            conflicting,
        )
        .unwrap();
    assert!(store
        .save(&Snapshot::capture(&restored, "default"))
        .is_err());
    let mut startup = request;
    startup.origin = Origin::Startup;
    assert!(Intent::Selection { request: startup }.validate().is_err());
}
