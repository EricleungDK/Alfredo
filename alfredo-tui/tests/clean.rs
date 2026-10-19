//! `/clean`: remove only finished runs' worktrees whose saved work is verifiably elsewhere.
use alfredo_tui::{
    clean,
    provider::Ollama,
    tasks::{Action, Request, TaskStatus, TaskStore, WorkPolicy},
    worker::{self, FileEdit, FilePlan},
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

static ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    store: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-clean-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let workspace = root.join("workspace");
        fs::create_dir(&workspace).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
        ] {
            assert!(Command::new("git")
                .arg("-C")
                .arg(&workspace)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        fs::write(workspace.join("seed.txt"), "seed\n").unwrap();
        let fixture = Self {
            store: TaskStore::new(&root.join("state"), &workspace, "mission").unwrap(),
            root,
            workspace,
        };
        fixture.git(&["add", "."]);
        fixture.git(&["-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture"]);
        fixture
    }
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    }
    fn action(&self, action: Action) {
        let revision = self.store.snapshot().unwrap().revision;
        self.store
            .transact(Request {
                correlation: format!("action-{revision}"),
                expected_revision: revision,
                action,
            })
            .unwrap();
    }
    /// Propose, permit and approve a task that must create `file` containing `OK`.
    fn task(&self, file: &str) -> u64 {
        self.action(Action::Propose {
            title: format!("create {file}"),
            model: "fixture".into(),
            dependencies: vec![],
        });
        let id = self.store.snapshot().unwrap().tasks.len() as u64;
        self.action(Action::Permit {
            task: id,
            policy: WorkPolicy {
                files: vec![file.into()],
                check: vec![
                    "/usr/bin/python3".into(),
                    "-B".into(),
                    "-c".into(),
                    format!("assert open('{file}').read().strip() == 'OK'"),
                ],
            },
        });
        self.action(Action::Approve { task: id });
        id
    }
    /// Run the worker once; `content` is what the model writes into `file`.
    async fn run(&self, task: u64, file: &str, content: &str) -> TaskStatus {
        let (provider, server, _) = serve(file, content, Duration::ZERO);
        let revision = self.store.snapshot().unwrap().revision;
        let (snapshot, _) = worker::start(
            self.store.clone(),
            task,
            format!("run-{task}-{revision}"),
            revision,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        server.join().unwrap();
        snapshot.tasks[task as usize - 1].status.clone()
    }
    fn worktrees(&self) -> Vec<String> {
        self.git(&["worktree", "list", "--porcelain"])
            .lines()
            .filter_map(|line| line.strip_prefix("worktree ").map(str::to_owned))
            .collect()
    }
    fn run_dir(&self, task: u64) -> PathBuf {
        let snapshot = self.store.snapshot().unwrap();
        let run = snapshot.tasks[task as usize - 1].run.as_ref().unwrap();
        self.store.run_directory(&run.id).unwrap()
    }
    fn candidate(&self, task: u64) -> String {
        let evidence: worker::Evidence =
            serde_json::from_str(&self.store.evidence(task).unwrap()).unwrap();
        evidence.candidate_commit.unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new("chmod")
            .arg("-R")
            .arg("u+rwx")
            .arg(&self.root)
            .status();
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One-request fixture model that answers with one file.
fn serve(
    file: &str,
    content: &str,
    delay: Duration,
) -> (
    Ollama,
    thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<()>,
) {
    let plan = serde_json::to_string(&FilePlan {
        files: vec![FileEdit {
            path: file.into(),
            content: content.into(),
        }],
    })
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (notify, received) = std::sync::mpsc::channel();
    let job = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(e) => panic!("no worker request: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut header = Vec::new();
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        let length: usize = String::from_utf8(header)
            .unwrap()
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_string)
            })
            .unwrap()
            .parse()
            .unwrap();
        let mut request = vec![0; length];
        stream.read_exact(&mut request).unwrap();
        let _ = notify.send(());
        thread::sleep(delay);
        let frame = serde_json::json!({"message":{"content":plan},"done":true});
        let body = serde_json::to_string(&frame).unwrap();
        let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
    });
    (
        Ollama::new(&endpoint, Duration::from_secs(5)).unwrap(),
        job,
        received,
    )
}

fn kept(plan: &clean::Plan, task: u64) -> Option<&str> {
    plan.keep
        .iter()
        .find(|k| k.task == task)
        .map(|k| k.reason.as_str())
}
fn removable(plan: &clean::Plan, task: u64) -> bool {
    plan.remove.iter().any(|e| e.task == task)
}

#[tokio::test]
async fn accepted_run_is_cleaned_but_candidate_evidence_and_branch_survive() {
    let fixture = Fixture::new();
    let task = fixture.task("a.txt");
    assert_eq!(
        fixture.run(task, "a.txt", "OK\n").await,
        TaskStatus::ReviewReady
    );
    fixture.action(Action::Review { task, accept: true });
    let directory = fixture.run_dir(task);
    let candidate = fixture.candidate(task);
    assert!(directory.join("worktree/a.txt").exists());
    assert_eq!(fixture.worktrees().len(), 2);

    let plan = clean::plan(&fixture.store).await.unwrap();
    assert!(removable(&plan, task), "{:?}", plan.keep);
    assert!(plan.bytes() > 0);
    // Listing changes nothing.
    assert!(directory.join("worktree").exists());

    let outcome = clean::clean(&fixture.store).await.unwrap();
    assert_eq!(outcome.removed.len(), 1);
    assert!(outcome.failed.is_empty());
    assert!(!directory.join("worktree").exists());
    // The user's repository lists only itself; no stale registration remains.
    assert_eq!(fixture.worktrees().len(), 1);
    assert!(!fixture
        .workspace
        .join(".git/worktrees")
        .join(directory.parent().unwrap().file_name().unwrap())
        .exists());
    // Candidate ref, evidence and the review branch still work.
    assert_eq!(
        fixture
            .git(&["rev-parse", &format!("refs/alfredo/candidates/{candidate}")])
            .trim(),
        candidate
    );
    assert!(directory.join("evidence.json").exists());
    assert_eq!(fixture.candidate(task), candidate);
    let revision = fixture.store.snapshot().unwrap().revision;
    let (_, notice) =
        alfredo_tui::branch::publish(fixture.store.clone(), task, revision, "after-clean".into())
            .await
            .unwrap();
    assert!(notice.contains("git switch"), "{notice}");
    // The user's own files and HEAD are untouched; a second clean is a no-op.
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("seed.txt")).unwrap(),
        "seed\n"
    );
    assert!(!fixture.workspace.join("a.txt").exists());
    assert!(clean::plan(&fixture.store).await.unwrap().remove.is_empty());
}

#[tokio::test]
async fn awaiting_review_missing_ref_tampered_evidence_and_foreign_dirs_are_kept() {
    let fixture = Fixture::new();
    let review = fixture.task("review.txt");
    let accepted_no_ref = fixture.task("noref.txt");
    let tampered = fixture.task("tampered.txt");
    let rejected = fixture.task("rejected.txt");
    for (task, file) in [
        (review, "review.txt"),
        (accepted_no_ref, "noref.txt"),
        (tampered, "tampered.txt"),
        (rejected, "rejected.txt"),
    ] {
        assert_eq!(
            fixture.run(task, file, "OK\n").await,
            TaskStatus::ReviewReady
        );
    }
    for (task, accept) in [(accepted_no_ref, true), (tampered, true), (rejected, false)] {
        fixture.action(Action::Review { task, accept });
    }
    // Accepted, but its candidate ref was deleted out from under it.
    let candidate = fixture.candidate(accepted_no_ref);
    fixture.git(&[
        "update-ref",
        "-d",
        &format!("refs/alfredo/candidates/{candidate}"),
    ]);
    // Accepted, but the saved evidence no longer matches its receipt.
    let evidence = fixture.run_dir(tampered).join("evidence.json");
    let mut bytes = fs::read(&evidence).unwrap();
    bytes.push(b' ');
    fs::write(&evidence, bytes).unwrap();
    // The user's own worktree and a stray directory are never candidates.
    let user = fixture.root.join("user-wt");
    fixture.git(&["worktree", "add", "-q", "--detach", user.to_str().unwrap()]);

    let plan = clean::plan(&fixture.store).await.unwrap();
    assert!(
        kept(&plan, review).unwrap().contains("awaiting review"),
        "{:?}",
        plan.keep
    );
    assert!(
        kept(&plan, accepted_no_ref).unwrap().contains("candidate"),
        "{:?}",
        plan.keep
    );
    assert!(
        kept(&plan, tampered).unwrap().contains("evidence"),
        "{:?}",
        plan.keep
    );
    assert!(removable(&plan, rejected), "{:?}", plan.keep);
    assert_eq!(plan.remove.len(), 1);

    let outcome = clean::clean(&fixture.store).await.unwrap();
    assert_eq!(outcome.removed.len(), 1);
    for task in [review, accepted_no_ref, tampered] {
        assert!(
            fixture.run_dir(task).join("worktree").exists(),
            "task {task}"
        );
    }
    assert!(!fixture.run_dir(rejected).join("worktree").exists());
    assert!(user.join(".git").exists());
    assert!(fixture.worktrees().iter().any(|w| w.ends_with("user-wt")));
}

#[tokio::test]
async fn failed_run_without_candidate_is_removable_and_keeps_its_patch_evidence() {
    let fixture = Fixture::new();
    let task = fixture.task("bad.txt");
    assert_eq!(
        fixture.run(task, "bad.txt", "WRONG\n").await,
        TaskStatus::Failed
    );
    let plan = clean::plan(&fixture.store).await.unwrap();
    assert!(removable(&plan, task), "{:?}", plan.keep);
    clean::clean(&fixture.store).await.unwrap();
    assert!(!fixture.run_dir(task).join("worktree").exists());
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(task).unwrap()).unwrap();
    assert!(evidence.candidate_commit.is_none());
    assert!(evidence.patch.contains("WRONG"), "{}", evidence.patch);
    assert!(fixture
        .git(&["for-each-ref", "refs/alfredo/candidates"])
        .is_empty());
}

#[tokio::test]
async fn running_worker_is_never_listed_for_removal() {
    let fixture = Fixture::new();
    let task = fixture.task("slow.txt");
    let (provider, server, received) = serve("slow.txt", "OK\n", Duration::from_millis(600));
    let store = fixture.store.clone();
    let worker = tokio::spawn(async move {
        worker::start(
            store,
            task,
            "slow".into(),
            3,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    while received.try_recv().is_err() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let plan = clean::plan(&fixture.store).await.unwrap();
    assert!(plan.remove.is_empty());
    assert!(
        kept(&plan, task).unwrap().contains("not finished"),
        "{:?}",
        plan.keep
    );
    let outcome = clean::clean(&fixture.store).await.unwrap();
    assert!(outcome.removed.is_empty());
    assert!(fixture.run_dir(task).join("worktree").exists());
    worker.await.unwrap().unwrap();
    server.join().unwrap();
}

#[tokio::test]
async fn unregistered_or_symlinked_worktree_directory_is_kept() {
    let fixture = Fixture::new();
    let task = fixture.task("odd.txt");
    assert_eq!(
        fixture.run(task, "odd.txt", "OK\n").await,
        TaskStatus::ReviewReady
    );
    fixture.action(Action::Review { task, accept: true });
    let directory = fixture.run_dir(task);
    let victim = fixture.root.join("victim");
    fs::create_dir(&victim).unwrap();
    fs::write(victim.join("precious.txt"), "keep").unwrap();
    // Replace the worktree with a symlink to a directory the user owns.
    fixture.git(&[
        "worktree",
        "remove",
        "--force",
        directory.join("worktree").to_str().unwrap(),
    ]);
    std::os::unix::fs::symlink(&victim, directory.join("worktree")).unwrap();
    let plan = clean::plan(&fixture.store).await.unwrap();
    assert!(plan.remove.is_empty(), "{:?}", plan.keep);
    clean::clean(&fixture.store).await.unwrap();
    assert!(victim.join("precious.txt").exists());
}

#[test]
fn command_refuses_while_autopilot_or_a_worker_is_active_and_needs_the_confirm_word() {
    use alfredo_tui::task_control::TaskControl;
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut tasks = TaskControl::new(fixture.store.clone());
    assert!(clean::command(&runtime, &mut tasks, true, "")
        .unwrap_err()
        .contains("autopilot"));
    assert!(clean::command(&runtime, &mut tasks, true, "yes")
        .unwrap_err()
        .contains("autopilot"));
    tasks.workers.insert(1, Arc::new(AtomicBool::new(false)));
    assert!(clean::command(&runtime, &mut tasks, false, "yes")
        .unwrap_err()
        .contains("worker"));
    tasks.workers.clear();
    assert!(clean::command(&runtime, &mut tasks, false, "now")
        .unwrap_err()
        .starts_with("Usage: /clean"));
    assert!(clean::command(&runtime, &mut tasks, false, "")
        .unwrap()
        .contains("Nothing to clean"));
}

#[tokio::test]
async fn users_own_worktrees_survive_and_a_foreign_stale_entry_blocks_the_global_prune() {
    let fixture = Fixture::new();
    let task = fixture.task("c.txt");
    assert_eq!(
        fixture.run(task, "c.txt", "OK\n").await,
        TaskStatus::ReviewReady
    );
    fixture.action(Action::Review { task, accept: true });
    let live = fixture.root.join("user-live");
    let gone = fixture.root.join("user-gone");
    for path in [&live, &gone] {
        fixture.git(&["worktree", "add", "-q", "--detach", path.to_str().unwrap()]);
    }
    fs::write(live.join("scratch.txt"), "mine").unwrap();
    // The user's other worktree vanished (say, an unmounted drive): a prune would forget it.
    fs::remove_dir_all(&gone).unwrap();

    let outcome = clean::clean(&fixture.store).await.unwrap();
    assert_eq!(outcome.removed.len(), 1);
    assert!(!outcome.pruned);
    assert!(outcome.prune_note.as_deref().unwrap().contains("skipped"));
    assert_eq!(
        fs::read_to_string(live.join("scratch.txt")).unwrap(),
        "mine"
    );
    let listed = fixture.worktrees();
    assert!(
        listed.iter().any(|w| w.ends_with("user-live")),
        "{listed:?}"
    );
    assert!(
        listed.iter().any(|w| w.ends_with("user-gone")),
        "{listed:?}"
    );
    assert!(!listed.iter().any(|w| w.contains("/runs/")), "{listed:?}");
}

#[tokio::test]
async fn nothing_removed_means_no_prune_and_the_kept_list_comes_from_the_executed_plan() {
    let fixture = Fixture::new();
    let task = fixture.task("d.txt");
    assert_eq!(
        fixture.run(task, "d.txt", "OK\n").await,
        TaskStatus::ReviewReady
    );
    let outcome = clean::clean(&fixture.store).await.unwrap();
    assert!(outcome.removed.is_empty() && !outcome.pruned);
    assert!(outcome
        .keep
        .iter()
        .any(|k| k.task == task && k.reason.contains("awaiting")));
}

#[tokio::test]
async fn symlinked_run_directory_is_ignored() {
    let fixture = Fixture::new();
    let task = fixture.task("e.txt");
    assert_eq!(
        fixture.run(task, "e.txt", "OK\n").await,
        TaskStatus::ReviewReady
    );
    fixture.action(Action::Review { task, accept: true });
    let directory = fixture.run_dir(task);
    let moved = fixture.root.join("moved-run");
    fs::rename(&directory, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &directory).unwrap();
    let plan = clean::plan(&fixture.store).await.unwrap();
    assert!(plan.remove.is_empty());
    assert!(moved.join("worktree").exists());
}
