use alfredo_tui::tasks::{Action, Request, TaskStore, WorkPolicy};
use std::fs;

/// A save from before `/approve` required a policy: approved, no policy. The store
/// refuses to create one now, so rewrite a permit+approve history without the permit.
fn forge_legacy_approval(store: &TaskStore) {
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

#[test]
fn legacy_upgrade_retains_exact_bytes_and_does_not_infer_execution_permission() {
    for version in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16] {
        let root =
            std::env::temp_dir().join(format!("alfredo-task-upgrade-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("workspace")).unwrap();
        let store =
            TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
        store
            .transact(Request {
                correlation: "propose".into(),
                expected_revision: 0,
                action: Action::Propose {
                    title: "legacy task".into(),
                    model: "worker".into(),
                    dependencies: vec![],
                },
            })
            .unwrap();
        for request in [
            Request {
                correlation: "permit".into(),
                expected_revision: 1,
                action: Action::Permit {
                    task: 1,
                    policy: WorkPolicy {
                        files: vec!["src/main.rs".into()],
                        check: vec!["/bin/true".into()],
                    },
                },
            },
            Request {
                correlation: "approve".into(),
                expected_revision: 2,
                action: Action::Approve { task: 1 },
            },
        ] {
            store.transact(request).unwrap();
        }
        forge_legacy_approval(&store);
        let namespace = fs::read_dir(root.join("state/rust-tasks-v1"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let path = namespace.join("tasks.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["schema_version"] = version.into();
        let legacy = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&path, &legacy).unwrap();
        assert_eq!(store.snapshot().unwrap().schema_version, version);
        assert!(store
            .transact(Request {
                correlation: "run".into(),
                expected_revision: 2,
                action: Action::Start {
                    inputs: vec![],
                    task: 1,
                    baseline: "a".repeat(40)
                }
            })
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), legacy);
        let (upgraded, _) = store
            .transact(Request {
                correlation: "permit".into(),
                expected_revision: 2,
                action: Action::Permit {
                    task: 1,
                    policy: WorkPolicy {
                        files: vec!["src/main.rs".into()],
                        check: vec!["/bin/true".into()],
                    },
                },
            })
            .unwrap();
        assert_eq!(upgraded.schema_version, 17);
        assert_eq!(
            upgraded.tasks[0].status,
            alfredo_tui::tasks::TaskStatus::Proposed
        );
        assert_eq!(
            fs::read(namespace.join(format!("tasks-v{version}-backup.json"))).unwrap(),
            legacy
        );
        assert!(store
            .transact(Request {
                correlation: "still-not-approved".into(),
                expected_revision: 3,
                action: Action::Start {
                    inputs: vec![],
                    task: 1,
                    baseline: "a".repeat(40)
                }
            })
            .is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
