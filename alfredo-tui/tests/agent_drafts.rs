//! Agent-view unsent drafts persist per conversation: quit/restart restores
//! each target's draft; clearing or sending removes it; bad files start empty.
use alfredo_tui::{
    agent_drafts,
    agent_view::{self, Target},
    model::App,
    task_control::TaskControl,
    tasks::TaskStore,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    store: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-agent-drafts-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let store = TaskStore::new(&root.join("state"), &workspace, "default").unwrap();
        Self { root, store }
    }
    /// A fresh process: new control and app over the same state.
    fn launch(&self, conversation: &str) -> (App, TaskControl, Option<String>) {
        let mut control = TaskControl::new(self.store.clone());
        let notice = control.load_agent_drafts(conversation);
        (App::new("m".into()), control, notice)
    }
    fn file(&self, conversation: &str) -> PathBuf {
        agent_drafts::state_path(&self.store.conversation_directory().unwrap(), conversation)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn type_in(app: &mut App, control: &mut TaskControl, target: Target, text: &str) {
    agent_view::open(app, control, target);
    app.sessions[0].clear_draft();
    app.sessions[0].insert(text);
}

#[test]
fn quit_and_restart_restore_each_targets_draft_and_leave_the_chat_draft_alone() {
    let f = Fixture::new();
    let (mut app, mut control, notice) = f.launch("default");
    assert_eq!(notice, None, "missing file is a clean start");
    app.sessions[0].insert("chat line");
    type_in(&mut app, &mut control, Target::Task(1), "note for one");
    agent_view::open(&mut app, &mut control, Target::Architect);
    app.sessions[0].insert("note for architect");
    // Quit closes the open view, as main does.
    agent_view::close(&mut app, &mut control, false);
    assert_eq!(app.sessions[0].draft, "chat line");
    drop((app, control));

    let (mut app, mut control, notice) = f.launch("default");
    assert_eq!(notice, None);
    app.sessions[0].insert("chat line");
    agent_view::open(&mut app, &mut control, Target::Task(1));
    assert_eq!(app.sessions[0].draft, "note for one");
    agent_view::open(&mut app, &mut control, Target::Architect);
    assert_eq!(app.sessions[0].draft, "note for architect");
    agent_view::close(&mut app, &mut control, false);
    assert_eq!(app.sessions[0].draft, "chat line");
}

#[test]
fn an_open_views_draft_survives_a_crash_through_the_periodic_persist() {
    let f = Fixture::new();
    let (mut app, mut control, _) = f.launch("default");
    type_in(
        &mut app,
        &mut control,
        Target::Task(1),
        "typed, never closed",
    );
    agent_view::persist(&app, &mut control);
    drop((app, control)); // crash: no close

    let (mut app, mut control, _) = f.launch("default");
    agent_view::open(&mut app, &mut control, Target::Task(1));
    assert_eq!(app.sessions[0].draft, "typed, never closed");
}

#[test]
fn clearing_or_sending_a_draft_removes_its_entry_and_the_file() {
    let f = Fixture::new();
    let (mut app, mut control, _) = f.launch("default");
    type_in(&mut app, &mut control, Target::Task(1), "keep me");
    agent_view::open(&mut app, &mut control, Target::Architect);
    app.sessions[0].insert("send me");
    agent_view::persist(&app, &mut control);
    let saved = fs::read_to_string(f.file("default")).unwrap();
    assert!(
        saved.contains("keep me") && saved.contains("send me"),
        "{saved}"
    );
    // Sending clears the prompt, as main does after a successful instruction.
    app.sessions[0].clear_draft();
    agent_view::persist(&app, &mut control);
    let saved = fs::read_to_string(f.file("default")).unwrap();
    assert!(
        saved.contains("keep me") && !saved.contains("send me"),
        "{saved}"
    );
    // Clearing the last draft leaves no entry at all.
    agent_view::close(&mut app, &mut control, false);
    agent_view::open(&mut app, &mut control, Target::Task(1));
    app.sessions[0].clear_draft();
    agent_view::close(&mut app, &mut control, false);
    assert!(!f.file("default").exists(), "no drafts, no file");

    let (mut app, mut control, _) = f.launch("default");
    agent_view::open(&mut app, &mut control, Target::Task(1));
    assert_eq!(app.sessions[0].draft, "");
}

#[test]
fn drafts_are_scoped_to_their_conversation() {
    let f = Fixture::new();
    let (mut app, mut control, _) = f.launch("one");
    type_in(&mut app, &mut control, Target::Task(1), "only in one");
    agent_view::close(&mut app, &mut control, false);
    let (mut app, mut control, _) = f.launch("two");
    agent_view::open(&mut app, &mut control, Target::Task(1));
    assert_eq!(app.sessions[0].draft, "");
    assert_ne!(f.file("one"), f.file("two"));
}

#[test]
fn corrupt_state_starts_empty_is_kept_aside_and_never_blocks() {
    for bad in [
        b"not json".to_vec(),
        br#"{"version":1,"drafts":[],"extra":true}"#.to_vec(),
        br#"{"version":9,"drafts":[]}"#.to_vec(),
        vec![b'x'; 300 * 1024],
    ] {
        let f = Fixture::new();
        let path = f.file("default");
        fs::write(&path, &bad).unwrap();
        let (mut app, mut control, notice) = f.launch("default");
        let notice = notice.expect("corruption is reported");
        assert!(notice.contains("drafts start empty"), "{notice}");
        let aside = path.with_extension("json.corrupt");
        assert_eq!(fs::read(&aside).unwrap(), bad, "original bytes are kept");
        agent_view::open(&mut app, &mut control, Target::Task(1));
        assert_eq!(app.sessions[0].draft, "");
        app.sessions[0].insert("fresh");
        agent_view::close(&mut app, &mut control, false);
        assert_eq!(fs::read(&aside).unwrap(), bad, "kept after new writes");
        let (mut app, mut control, notice) = f.launch("default");
        assert_eq!(notice, None);
        agent_view::open(&mut app, &mut control, Target::Task(1));
        assert_eq!(app.sessions[0].draft, "fresh");
    }
}

#[test]
fn the_conversation_snapshot_saves_the_chat_draft_while_an_agent_view_is_open() {
    let snapshot_draft = |app: &App| {
        alfredo_tui::conversations::Snapshot::capture(app, "default")
            .restore()
            .sessions[0]
            .draft
            .clone()
    };
    let f = Fixture::new();
    let (mut app, mut control, _) = f.launch("default");
    app.sessions[0].insert("real chat text");
    type_in(&mut app, &mut control, Target::Task(1), "agent note");
    assert_eq!(snapshot_draft(&app), "real chat text");
    // Moving to another target keeps the same chat draft aside.
    agent_view::open(&mut app, &mut control, Target::Architect);
    app.sessions[0].insert("other note");
    assert_eq!(snapshot_draft(&app), "real chat text");
    agent_view::persist(&app, &mut control);
    agent_view::close(&mut app, &mut control, false);
    assert_eq!(snapshot_draft(&app), "real chat text");
    // After close the snapshot follows what is typed in the chat again.
    app.sessions[0].insert(" more");
    assert_eq!(snapshot_draft(&app), "real chat text more");
    drop((app, control));

    let (mut app, mut control, _) = f.launch("default");
    agent_view::open(&mut app, &mut control, Target::Task(1));
    assert_eq!(app.sessions[0].draft, "agent note");
    agent_view::open(&mut app, &mut control, Target::Architect);
    assert_eq!(app.sessions[0].draft, "other note");
}

#[test]
fn a_second_corruption_keeps_the_first_quarantined_copy() {
    let f = Fixture::new();
    let path = f.file("default");
    fs::write(&path, "first bad").unwrap();
    assert!(f.launch("default").2.is_some());
    fs::write(&path, "second bad").unwrap();
    let notice = f.launch("default").2.unwrap();
    assert_eq!(
        fs::read(path.with_extension("json.corrupt")).unwrap(),
        b"first bad"
    );
    assert_eq!(
        fs::read(path.with_extension("json.corrupt.1")).unwrap(),
        b"second bad"
    );
    assert!(notice.contains("corrupt.1"), "{notice}");
}

#[test]
fn an_unwritable_state_location_reports_once_and_keeps_the_draft_in_memory() {
    let f = Fixture::new();
    let (mut app, mut control, _) = f.launch("default");
    // A directory where the file belongs makes every write fail.
    fs::create_dir_all(f.file("default")).unwrap();
    type_in(&mut app, &mut control, Target::Task(1), "still here");
    assert!(agent_view::persist(&app, &mut control).is_some());
    assert_eq!(agent_view::persist(&app, &mut control), None, "no repeat");
    agent_view::close(&mut app, &mut control, false);
    agent_view::open(&mut app, &mut control, Target::Task(1));
    assert_eq!(app.sessions[0].draft, "still here");
}

mod workstation {
    use alfredo_tui::{
        agent_view::{self, Target},
        provider::Ollama,
        workstation::Workstation,
    };
    use std::{fs, path::PathBuf, time::Duration};
    use tokio::runtime::Runtime;

    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "alfredo-agent-drafts-ws-{}-{}",
                std::process::id(),
                super::NEXT.fetch_add(1, super::Ordering::Relaxed)
            ));
            for name in ["a", "b"] {
                fs::create_dir_all(root.join(name)).unwrap();
                assert!(std::process::Command::new("git")
                    .args(["init", "-q", "--template="])
                    .arg(root.join(name))
                    .status()
                    .unwrap()
                    .success());
            }
            for (name, mission) in [("a", "alpha"), ("b", "beta")] {
                alfredo_tui::tasks::TaskStore::new(&root.join("state"), &root.join(name), mission)
                    .unwrap()
                    .select_mission(true)
                    .unwrap();
            }
            Self { root }
        }
        fn open(&self) -> Workstation {
            Workstation::open(
                &self.root.join("state"),
                &self.root.join("a"),
                "alpha",
                "default",
                "fixture",
                Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_reopened_workstation_restores_agent_drafts_and_a_bad_file_only_warns() {
        let f = Fixture::new();
        let mut work = f.open();
        agent_view::open(&mut work.app, &mut work.tasks, Target::Task(7));
        work.app.sessions[0].insert("half a note");
        agent_view::close(&mut work.app, &mut work.tasks, false);
        drop(work);

        let mut work = f.open();
        agent_view::open(&mut work.app, &mut work.tasks, Target::Task(7));
        assert_eq!(work.app.sessions[0].draft, "half a note");
        drop(work);

        let directory =
            alfredo_tui::tasks::TaskStore::new(&f.root.join("state"), &f.root.join("a"), "alpha")
                .unwrap()
                .conversation_directory()
                .unwrap();
        fs::write(
            alfredo_tui::agent_drafts::state_path(&directory, "default"),
            "{",
        )
        .unwrap();
        let work = f.open();
        assert!(
            work.app.notice.contains("drafts start empty"),
            "{}",
            work.app.notice
        );
        assert!(work.tasks.agent_drafts.is_empty());
    }

    #[test]
    fn a_mission_switch_with_the_view_open_keeps_the_chat_draft_and_the_agent_note_apart() {
        let f = Fixture::new();
        let runtime = Runtime::new().unwrap();
        let mut work = f.open();
        work.app.sessions[0].insert("real chat text");
        agent_view::open(&mut work.app, &mut work.tasks, Target::Task(7));
        work.app.sessions[0].insert("agent note");
        assert!(work.switch_to(&runtime, &f.root.join("b"), "beta").unwrap());
        while work.tasks.pending {
            work.tasks.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(work
            .switch_to(&runtime, &f.root.join("a"), "alpha")
            .unwrap());
        assert_eq!(work.app.sessions[0].draft, "real chat text");
        assert_eq!(
            work.tasks
                .agent_drafts
                .get(&Target::Task(7))
                .map(String::as_str),
            Some("agent note")
        );
    }

    #[test]
    fn switching_missions_saves_the_open_agent_view_draft() {
        let f = Fixture::new();
        let runtime = Runtime::new().unwrap();
        let mut work = f.open();
        agent_view::open(&mut work.app, &mut work.tasks, Target::Task(7));
        work.app.sessions[0].insert("typed before switching");
        assert!(work.switch_to(&runtime, &f.root.join("b"), "beta").unwrap());
        while work.tasks.pending {
            work.tasks.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            work.tasks.agent_drafts.is_empty(),
            "beta has its own drafts"
        );
        assert!(work
            .switch_to(&runtime, &f.root.join("a"), "alpha")
            .unwrap());
        assert_eq!(
            work.tasks
                .agent_drafts
                .get(&Target::Task(7))
                .map(String::as_str),
            Some("typed before switching")
        );
    }
}
