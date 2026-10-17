use alfredo_tui::tasks::TaskStore;
use std::fs;

/// Rewrites a saved [propose, permit, approve] history into a pre-rule save: the
/// approval receipt stays but the policy is gone. The store refuses to create that
/// state now, but older saves must still load.
pub fn forge_legacy_approval(store: &TaskStore) {
    let path = store.conversation_directory().unwrap().join("tasks.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let receipts = value["receipts"].as_array_mut().unwrap();
    receipts.remove(1);
    receipts[1]["request"]["expected_revision"] = 1.into();
    receipts[1]["revision"] = 2.into();
    value["revision"] = 2.into();
    value["tasks"][0].as_object_mut().unwrap().remove("policy");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
