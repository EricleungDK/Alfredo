use alfredo_tui::{
    selection_command::{Choice, MissionChoice, Origin, Outcome, Phase, Request, WorkspaceChoice},
    selection_store::Store,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "alfredo-selection-store-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        )))
    }
    fn request(&self) -> Request {
        Request::new(
            Origin::Startup,
            Choice {
                workspace: WorkspaceChoice::Create {
                    parent: self.0.clone(),
                    name: "target".into(),
                },
                mission: MissionChoice::StartNew {
                    name: "mission".into(),
                },
            },
            "default".into(),
        )
        .unwrap()
    }
    fn store(&self) -> Store {
        Store::new(&self.0.join("state")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn synced_admission_is_exact_one_use_and_restart_never_grants_replay() {
    let f = Fixture::new();
    let store = f.store();
    let request = f.request();
    let token = store.admit(request.clone()).unwrap();
    assert!(!f.0.join("target").exists());
    let first = store.snapshot().unwrap();
    assert_eq!(first[0].request, request);
    assert!(!first[0].dispatched);
    assert!(store
        .admit(request.clone())
        .unwrap_err()
        .contains("cannot be replayed"));
    let mut changed = request.clone();
    changed.choice.mission = MissionChoice::StartNew {
        name: "other".into(),
    };
    assert!(store
        .admit(changed)
        .unwrap_err()
        .contains("different input"));
    let operation = store.begin(token).unwrap();
    store
        .record(&operation, Outcome::at(Phase::RepositoryReady))
        .unwrap();
    assert!(store
        .record(&operation, Outcome::at(Phase::Admitted))
        .is_err());
    drop(operation);
    drop(store);
    let restored = f.store();
    let records = restored.snapshot().unwrap();
    assert!(records[0].dispatched);
    assert_eq!(records[0].outcome.phase, Phase::RepositoryReady);
    assert!(restored.admit(request).is_err());
    assert!(!f.0.join("target").exists());
}
#[test]
fn journal_conflict_and_malformed_history_preserve_bytes_without_effects() {
    let f = Fixture::new();
    let store = f.store();
    let request = f.request();
    store.admit(request.clone()).unwrap();
    let file = f.0.join("state/rust-selection-v1/selections.json");
    let bytes = fs::read(&file).unwrap();
    assert!(store.admit(request).is_err());
    assert_eq!(fs::read(&file).unwrap(), bytes);
    fs::write(&file, b"{broken history").unwrap();
    assert!(store.admit(f.request()).is_err());
    assert_eq!(fs::read(file).unwrap(), b"{broken history");
    assert!(!f.0.join("target").exists());
}
#[cfg(unix)]
#[test]
fn dangling_journal_and_symlink_ancestor_are_refused_without_replacement() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let root = f.0.join("state/rust-selection-v1");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("selections.json");
    let absent = f.0.join("missing");
    symlink(&absent, &file).unwrap();
    assert!(f.store().admit(f.request()).is_err());
    assert_eq!(fs::read_link(&file).unwrap(), absent);
    fs::remove_file(&file).unwrap();
    fs::remove_dir_all(&root).unwrap();
    let actual = f.0.join("actual");
    fs::create_dir(&actual).unwrap();
    symlink(&actual, &root).unwrap();
    assert!(f.store().snapshot().is_err());
    assert!(f.store().admit(f.request()).is_err());
    assert!(fs::read_dir(actual).unwrap().next().is_none());
}

#[test]
fn full_journal_refuses_before_dispatch_and_preserves_history() {
    use alfredo_tui::selection_store::Record;
    let f = Fixture::new();
    let root = f.0.join("state/rust-selection-v1");
    fs::create_dir_all(&root).unwrap();
    let records: Vec<Record> = (0..512)
        .map(|_| Record {
            request: f.request(),
            outcome: Outcome::at(Phase::Admitted),
            dispatched: false,
        })
        .collect();
    let file = root.join("selections.json");
    let bytes =
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"records":records})).unwrap();
    fs::write(&file, &bytes).unwrap();
    assert!(f
        .store()
        .admit(f.request())
        .unwrap_err()
        .contains("capacity"));
    assert_eq!(fs::read(file).unwrap(), bytes);
    assert!(!f.0.join("target").exists());
}

#[test]
fn journal_inside_uncreated_target_is_refused_before_any_directory_creation() {
    let f = Fixture::new();
    fs::create_dir_all(&f.0).unwrap();
    let request = f.request();
    let target = request.choice.target();
    let store = Store::new(&target.join("state")).unwrap();
    assert!(store.admit(request).unwrap_err().contains("outside"));
    assert!(!target.exists());
    assert!(fs::read_dir(&f.0).unwrap().next().is_none());
}
