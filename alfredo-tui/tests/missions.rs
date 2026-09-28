use alfredo_tui::{
    conversations::{ConversationStore, Snapshot},
    missions::{discover, remember},
    model::App,
    tasks::{Action, Request, TaskStore},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    state: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-missions-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        Self {
            state: root.join("state"),
            root,
            workspace,
        }
    }
    fn store(&self, name: &str) -> TaskStore {
        TaskStore::new(&self.state, &self.workspace, name).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn discovery_restores_conversation_only_and_legacy_task_names_without_mutation() {
    let f = Fixture::new();
    assert!(discover(&f.state, &f.workspace).unwrap().names.is_empty());
    assert!(!f.state.exists());
    let chat = f.store("Chat only 界");
    let conversations = ConversationStore::open(&chat, "default").unwrap();
    conversations
        .save(&Snapshot::capture(&App::new("fixture".into()), "default"))
        .unwrap();
    remember(&chat, &f.workspace, "Chat only 界").unwrap();
    remember(&chat, &f.workspace, "Chat only 界").unwrap();
    assert!(!chat
        .conversation_directory()
        .unwrap()
        .join("tasks.json")
        .exists());
    let legacy = f.store("Legacy tasks");
    legacy
        .transact(Request {
            correlation: "create".into(),
            expected_revision: 0,
            action: Action::Propose {
                title: "untouched".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        })
        .unwrap();
    let path = legacy.conversation_directory().unwrap().join("tasks.json");
    let bytes = fs::read(&path).unwrap();
    legacy.select_mission(false).unwrap();
    assert!(legacy.select_mission(true).is_err());
    let other = f.root.join("other");
    fs::create_dir(&other).unwrap();
    remember(
        &TaskStore::new(&f.state, &other, "Other workspace").unwrap(),
        &other,
        "Other workspace",
    )
    .unwrap();
    let found = discover(&f.state, &f.workspace).unwrap();
    assert_eq!(found.names, ["Chat only 界", "Legacy tasks"]);
    assert_eq!(found.skipped, 0);
    assert!(!found.limited);
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert!(!legacy
        .conversation_directory()
        .unwrap()
        .join("identity.json")
        .exists());
}
#[test]
fn corrupt_or_misplaced_discovery_records_are_skipped_and_preserved() {
    let f = Fixture::new();
    let store = f.store("real");
    remember(&store, &f.workspace, "real").unwrap();
    let path = store
        .conversation_directory()
        .unwrap()
        .join("identity.json");
    let bytes = fs::read(&path).unwrap();
    let wrong = f.state.join("rust-tasks-v1/wrong");
    fs::create_dir(&wrong).unwrap();
    fs::write(wrong.join("identity.json"), &bytes).unwrap();
    assert_eq!(discover(&f.state, &f.workspace).unwrap().names, ["real"]);
    fs::write(&path, b"{broken").unwrap();
    let found = discover(&f.state, &f.workspace).unwrap();
    assert!(found.names.is_empty());
    assert_eq!(found.skipped, 2);
    assert!(remember(&store, &f.workspace, "real").is_err());
    assert_eq!(fs::read(path).unwrap(), b"{broken");
    assert!(store.snapshot().unwrap().tasks.is_empty());
}
#[test]
fn discovery_reports_its_scan_limit_without_creating_or_loading_tasks() {
    let f = Fixture::new();
    let root = f.state.join("rust-tasks-v1");
    fs::create_dir_all(&root).unwrap();
    for n in 0..1025 {
        fs::create_dir(root.join(n.to_string())).unwrap();
    }
    let found = discover(&f.state, &f.workspace).unwrap();
    assert!(found.limited);
    assert_eq!(found.skipped, 1024);
    assert!(found.names.is_empty());
    assert_eq!(fs::read_dir(root).unwrap().count(), 1025);
}

#[test]
fn discovery_stops_at_aggregate_byte_budget() {
    let f = Fixture::new();
    let root = f.state.join("rust-tasks-v1");
    let oversized_legacy = vec![b' '; 4 * 1024 * 1024];
    for n in 0..5 {
        let directory = root.join(n.to_string());
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("tasks.json"), &oversized_legacy).unwrap();
    }
    let found = discover(&f.state, &f.workspace).unwrap();
    assert!(found.limited);
    assert_eq!(found.skipped, 4);
    assert!(found.names.is_empty());
}

#[cfg(unix)]
#[test]
fn discovery_does_not_follow_state_directory_symlinks() {
    let f = Fixture::new();
    fs::create_dir(&f.state).unwrap();
    let outside = f.root.join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, f.state.join("rust-tasks-v1")).unwrap();
    assert!(discover(&f.state, &f.workspace).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn workspace_hints_deduplicate_and_do_not_require_or_create_missing_repositories() {
    use alfredo_tui::missions::discover_workspaces;
    let f = Fixture::new();
    for mission in ["one", "two"] {
        remember(&f.store(mission), &f.workspace, mission).unwrap();
    }
    let other = f.root.join("other");
    fs::create_dir(&other).unwrap();
    let store = TaskStore::new(&f.state, &other, "another").unwrap();
    remember(&store, &other, "another").unwrap();
    fs::remove_dir(&other).unwrap();
    let found = discover_workspaces(&f.state).unwrap();
    let mut expected = vec![f.workspace.clone(), other.clone()];
    expected.sort();
    assert_eq!(found.workspaces, expected);
    assert!(found.names.is_empty());
    assert!(!other.exists());
    assert_eq!(
        discover(&f.state, &f.workspace).unwrap().names,
        ["one", "two"]
    );
}

#[test]
fn mission_start_and_resume_are_distinct_and_preserve_existing_identity() {
    let f = Fixture::new();
    let store = f.store("mission");
    assert!(store
        .select_mission(false)
        .unwrap_err()
        .contains("does not exist"));
    assert!(!f.state.exists());
    store.select_mission(true).unwrap();
    let path = store.conversation_directory().unwrap().join("mission.json");
    let bytes = fs::read(&path).unwrap();
    assert!(store
        .select_mission(true)
        .unwrap_err()
        .contains("already exists"));
    store.select_mission(false).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(!store
        .conversation_directory()
        .unwrap()
        .join("tasks.json")
        .exists());
    f.store("distinct").select_mission(true).unwrap();
    assert_eq!(
        discover(&f.state, &f.workspace).unwrap().names,
        ["distinct", "mission"]
    );
    fs::write(&path, b"{broken").unwrap();
    assert!(store.select_mission(false).is_err());
    assert!(store.select_mission(true).is_err());
    assert_eq!(fs::read(path).unwrap(), b"{broken");
}

#[test]
fn simultaneous_start_new_requests_create_only_one_mission() {
    let f = Fixture::new();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs: Vec<_> = (0..2)
        .map(|_| {
            let store = f.store("race");
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.select_mission(true)
            })
        })
        .collect();
    let results: Vec<_> = jobs.into_iter().map(|j| j.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    f.store("race").select_mission(false).unwrap();
    assert_eq!(discover(&f.state, &f.workspace).unwrap().names, ["race"]);
}

#[test]
fn advisory_identity_cannot_create_a_resumable_mission_and_legacy_data_is_preserved() {
    let f = Fixture::new();
    let store = f.store("legacy chat");
    remember(&store, &f.workspace, "legacy chat").unwrap();
    assert!(store.select_mission(false).is_err());
    let conversations = ConversationStore::open(&store, "default").unwrap();
    conversations
        .save(&Snapshot::capture(&App::new("fixture".into()), "default"))
        .unwrap();
    store.select_mission(false).unwrap();
    assert!(store.select_mission(true).is_err());
    assert!(!store
        .conversation_directory()
        .unwrap()
        .join("mission.json")
        .exists());
    let restored = conversations.load().unwrap().unwrap();
    assert_eq!(restored.sessions[0].model, "fixture");
}
