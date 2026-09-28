use alfredo_tui::{
    task_control::TaskControl,
    tasks::{Action, Request, Snapshot, TaskStore},
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
            "alfredo-chronology-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("workspace")).unwrap();
        let store =
            TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
        Self { root, store }
    }
    fn propose(&self) -> (Snapshot, Request) {
        let revision = self.store.snapshot().unwrap().revision;
        let request = Request {
            correlation: format!("proposal-{revision}"),
            expected_revision: revision,
            action: Action::Propose {
                title: format!("Task {revision}"),
                model: "fixture".into(),
                dependencies: vec![],
            },
        };
        (self.store.transact(request.clone()).unwrap().0, request)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn receipt_observation_excludes_initial_history_and_deduplicates_canonical_updates() {
    let fixture = Fixture::new();
    fixture.propose();
    let (historical, _) = fixture.propose();
    let mut control = TaskControl::new(fixture.store.clone());
    assert!(control.newly_observed_receipts().is_empty());
    control.snapshot = Some(historical);
    assert!(control.newly_observed_receipts().is_empty());
    let (third, request) = fixture.propose();
    control.snapshot = Some(third.clone());
    let observed = control.newly_observed_receipts();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].revision, 3);
    assert_eq!(observed[0].correlation, request.correlation);
    assert_eq!(observed[0].task, 3);
    assert_eq!(observed[0].after_messages, 0);
    assert!(control.newly_observed_receipts().is_empty());
    // Exact transaction retry does not duplicate the presentation event.
    control.snapshot = Some(fixture.store.transact(request.clone()).unwrap().0);
    assert!(control.newly_observed_receipts().is_empty());
    let mut conflict = request;
    conflict.action = Action::Propose {
        title: "Changed request".into(),
        model: "fixture".into(),
        dependencies: vec![],
    };
    assert!(fixture.store.transact(conflict).is_err());
    control.snapshot = Some(fixture.store.snapshot().unwrap());
    assert!(control.newly_observed_receipts().is_empty());
    fixture.propose();
    let (fifth, _) = fixture.propose();
    control.snapshot = Some(fifth);
    assert_eq!(
        control
            .newly_observed_receipts()
            .iter()
            .map(|r| r.revision)
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
    // A stale observation cannot rewind the cursor and replay old receipts later.
    control.snapshot = Some(third);
    assert!(control.newly_observed_receipts().is_empty());
    let (sixth, _) = fixture.propose();
    control.snapshot = Some(sixth.clone());
    let observed = control.newly_observed_receipts();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].revision, 6);
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let bytes = fs::read(&path).unwrap();
    let mut restarted = TaskControl::new(fixture.store.clone());
    restarted.snapshot = Some(sixth);
    assert!(restarted.newly_observed_receipts().is_empty());
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn receipt_observation_after_empty_initial_state_keeps_every_new_acknowledgment() {
    let fixture = Fixture::new();
    let mut control = TaskControl::new(fixture.store.clone());
    control.snapshot = Some(fixture.store.snapshot().unwrap());
    assert!(control.newly_observed_receipts().is_empty());
    // Callers can defer draining during streaming without losing the new prefix.
    fixture.propose();
    fixture.propose();
    let (snapshot, _) = fixture.propose();
    control.snapshot = Some(snapshot);
    let observed = control.newly_observed_receipts();
    assert_eq!(
        observed.iter().map(|r| r.revision).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(observed.iter().all(|r| r.after_messages == 0));
    assert!(control.newly_observed_receipts().is_empty());
}
