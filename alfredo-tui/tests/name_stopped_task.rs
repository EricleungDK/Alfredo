//! Issue #584: only the run-level `autopilot` entry with a null id receives the id of
//! the task that stopped; every other failed entry is left as it was.
use alfredo_tui::autopilot::name_stopped_task;
use serde_json::json;

#[test]
fn only_the_null_id_autopilot_entry_gets_the_stopped_task_id() {
    let mut result = json!({"failed": [
        {"id": null, "title": "other", "reason": "a"},
        {"id": 3, "title": "autopilot", "reason": "b"},
        {"id": null, "title": "autopilot", "reason": "c"},
    ]});
    name_stopped_task(&mut result, 7);
    assert_eq!(
        result["failed"],
        json!([
            {"id": null, "title": "other", "reason": "a"},
            {"id": 3, "title": "autopilot", "reason": "b"},
            {"id": 7, "title": "autopilot", "reason": "c"},
        ])
    );
}

#[test]
fn no_autopilot_entry_with_a_null_id_leaves_the_list_unchanged() {
    let original = json!({"failed": [
        {"id": null, "title": "other", "reason": "a"},
        {"id": 3, "title": "autopilot", "reason": "b"},
    ]});
    let mut result = original.clone();
    name_stopped_task(&mut result, 7);
    assert_eq!(result, original);
}
