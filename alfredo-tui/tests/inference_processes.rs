//! Separate OS processes exercise the same production admission boundary.
use alfredo_tui::inference_admission::{Class, Coordinator};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
static ID: AtomicU64 = AtomicU64::new(0);

fn marker(root: &std::path::Path, id: &str, phase: &str, value: &str) {
    let path = root.join(format!("{id}.{phase}"));
    let temporary = path.with_extension(format!("{phase}.tmp"));
    fs::write(&temporary, value).unwrap();
    fs::rename(temporary, path).unwrap();
}

#[test]
fn process_client() {
    let Ok(spec) = std::env::var("ALFREDO_TEST_INFERENCE_PROCESS") else {
        return;
    };
    let spec: serde_json::Value = serde_json::from_str(&spec).unwrap();
    let root = PathBuf::from(spec["root"].as_str().unwrap());
    let id = spec["id"].as_str().unwrap();
    let class = if spec["foreground"].as_bool().unwrap() {
        Class::Foreground
    } else {
        Class::Background
    };
    let coordinator = Coordinator::new(
        spec["origin"].as_str().unwrap().to_owned(),
        spec["capacity"].as_u64().unwrap() as usize,
    )
    .unwrap();
    let mut queue = coordinator.queue(class);
    let deadline = Instant::now() + Duration::from_secs(20);
    let permit = loop {
        match queue.poll() {
            Ok(Some(permit)) => break permit,
            Ok(None) => {
                if queue.observation().is_some() {
                    marker(&root, id, "queued", "queued");
                }
            }
            Err(error) => {
                marker(&root, id, "error", &error);
                return;
            }
        }
        assert!(Instant::now() < deadline, "client {id} admission timed out");
        std::thread::sleep(Duration::from_millis(5));
    };
    marker(&root, id, "granted", "granted");
    while !root.join(format!("{id}.release")).exists() {
        assert!(Instant::now() < deadline, "client {id} release timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(permit);
}

struct Processes {
    root: PathBuf,
    origin: String,
    children: BTreeMap<String, Child>,
}
impl Processes {
    fn new() -> Self {
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(format!("alfredo-inference-processes-{nonce}"));
        fs::create_dir(&root).unwrap();
        Self {
            root,
            origin: format!("http://inference-{nonce}.invalid"),
            children: BTreeMap::new(),
        }
    }
    fn start(&mut self, id: &str, foreground: bool, capacity: usize) {
        let spec = serde_json::json!({"root": self.root, "origin": self.origin, "id":id, "foreground":foreground, "capacity":capacity});
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process_client", "--nocapture"])
            .env("ALFREDO_TEST_INFERENCE_PROCESS", spec.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.children.insert(id.into(), child);
    }
    fn wait(&mut self, id: &str, phase: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.root.join(format!("{id}.{phase}")).exists() {
            let error =
                fs::read_to_string(self.root.join(format!("{id}.error"))).unwrap_or_default();
            if phase == "error" && !error.is_empty() {
                return;
            }
            assert!(error.is_empty(), "client {id}: {error}");
            if let Some(status) = self.children.get_mut(id).unwrap().try_wait().unwrap() {
                assert!(
                    status.success() && self.root.join(format!("{id}.{phase}")).exists(),
                    "client {id} exited before {phase}: {status}"
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "client {id} did not reach {phase}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn release(&mut self, id: &str) {
        fs::write(self.root.join(format!("{id}.release")), b"release").unwrap();
        let mut child = self.children.remove(id).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                panic!("client {id} did not release");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn kill(&mut self, id: &str) {
        let mut child = self.children.remove(id).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }
    fn assert_waiting(&self, ids: &[&str]) {
        for id in ids {
            assert!(
                !self.root.join(format!("{id}.granted")).exists(),
                "{id} exceeded shared capacity or ordering"
            );
        }
    }
}
impl Drop for Processes {
    fn drop(&mut self) {
        for child in self.children.values_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn separate_processes_share_capacity_and_bound_foreground_priority_with_fifo() {
    let mut processes = Processes::new();
    processes.start("blocker", false, 1);
    processes.wait("blocker", "granted");
    for (id, foreground) in [
        ("background", false),
        ("fg1", true),
        ("fg2", true),
        ("fg3", true),
        ("fg4", true),
    ] {
        processes.start(id, foreground, 1);
        processes.wait(id, "queued");
    }
    processes.assert_waiting(&["background", "fg1", "fg2", "fg3", "fg4"]);
    processes.release("blocker");
    for (next, waiting) in [
        ("fg1", vec!["fg2", "fg3", "background", "fg4"]),
        ("fg2", vec!["fg3", "background", "fg4"]),
        ("fg3", vec!["background", "fg4"]),
        ("background", vec!["fg4"]),
        ("fg4", vec![]),
    ] {
        processes.wait(next, "granted");
        processes.assert_waiting(&waiting);
        processes.release(next);
    }
}

#[test]
fn killed_waiters_and_active_owners_release_capacity_without_replay() {
    let mut processes = Processes::new();
    processes.start("active", false, 1);
    processes.wait("active", "granted");
    processes.start("cancelled", true, 1);
    processes.wait("cancelled", "queued");
    processes.start("survivor", false, 1);
    processes.wait("survivor", "queued");
    processes.kill("cancelled");
    processes.assert_waiting(&["cancelled", "survivor"]);
    processes.kill("active");
    processes.wait("survivor", "granted");
    processes.assert_waiting(&["cancelled"]);
    processes.release("survivor");
    processes.start("fresh", false, 1);
    processes.wait("fresh", "granted");
    processes.assert_waiting(&["cancelled"]);
    processes.release("fresh");
}

#[test]
fn conflicting_process_capacity_refuses_until_live_requests_drain() {
    let mut processes = Processes::new();
    processes.start("active", false, 1);
    processes.wait("active", "granted");
    processes.start("conflict", true, 2);
    processes.wait("conflict", "error");
    let error = fs::read_to_string(processes.root.join("conflict.error")).unwrap();
    assert!(error.to_lowercase().contains("capacity"), "{error}");
    processes.assert_waiting(&["conflict"]);
    processes.release("active");
    processes.start("reconfigured", true, 2);
    processes.wait("reconfigured", "granted");
    processes.start("second", false, 2);
    processes.wait("second", "granted");
    processes.release("reconfigured");
    processes.release("second");
}
