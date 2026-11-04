//! Issue #587: dispatch turning itself off leaves one row for the chat transcript.

use alfredo_tui::dispatch::TRANSIENT_LIMIT;
use alfredo_tui::task_control::TaskControl;
use alfredo_tui::tasks::{Refusal, TaskStore};

fn control(on: bool) -> (TaskControl, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "alfredo-dispatch-off-rows-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
    let mut control = TaskControl::new(store);
    control.dispatch.enabled = on;
    (control, root)
}

#[test]
fn a_busy_turn_off_queues_one_chat_row_with_the_refusal_and_the_way_back() {
    let (mut control, root) = control(true);
    for _ in 0..TRANSIENT_LIMIT {
        control.background_refresh_failed(&Refusal::Busy);
    }
    assert!(control.dispatch.enabled);
    assert!(control.take_dispatch_off_rows().is_empty());
    control.background_refresh_failed(&Refusal::Busy);
    // Later failing ticks while dispatch is already off add nothing.
    control.background_refresh_failed(&Refusal::Busy);
    assert!(!control.dispatch.enabled);
    let rows = control.take_dispatch_off_rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0],
        "Task refresh failed 6 times in a row; dispatch off: Task store is busy; refresh and retry\nResume with /dispatch on"
    );
    assert_eq!(control.notice, rows[0].lines().next().unwrap());
    assert!(control.take_dispatch_off_rows().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_non_transient_turn_off_queues_the_same_row_once() {
    let (mut control, root) = control(true);
    control.background_refresh_failed(&Refusal::Denied("disk unreadable".into()));
    control.background_refresh_failed(&Refusal::Denied("disk unreadable".into()));
    let rows = control.take_dispatch_off_rows();
    assert_eq!(
        rows,
        vec!["Task refresh failed; dispatch off: disk unreadable\nResume with /dispatch on"]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_refresh_failure_with_dispatch_already_off_queues_no_row() {
    let (mut control, root) = control(false);
    control.background_refresh_failed(&Refusal::Denied("disk unreadable".into()));
    for _ in 0..=TRANSIENT_LIMIT {
        control.background_refresh_failed(&Refusal::Busy);
    }
    assert!(control.take_dispatch_off_rows().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_later_turn_off_queues_its_own_row_and_the_queue_is_bounded() {
    let (mut control, root) = control(true);
    for round in 0..6 {
        control.dispatch.enabled = true;
        control.background_refresh_failed(&Refusal::Denied(format!("fault {round}")));
    }
    let rows = control.take_dispatch_off_rows();
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(rows[0].contains("fault 0") && rows[3].contains("fault 3"));
    let _ = std::fs::remove_dir_all(root);
}
