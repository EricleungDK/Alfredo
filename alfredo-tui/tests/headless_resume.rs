//! Issue #584: `--resume --headless` retries the task an unreachable model server
//! stopped, without planning again; every other failed run stays "nothing to resume".
mod ollama_fixture;

use alfredo_tui::tasks::{Action, Request, TaskStore};
use ollama_fixture::{done, Fixture, Reply};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

const GOAL: &str = "Make answer return 42 and add app";
/// What the fixture's worker does with an implement request.
const ANSWER: u8 = 0;
const DROP: u8 = 1;
const PROSE: u8 = 2;

fn git(workspace: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "gc.auto=0", "-c", "maintenance.auto=false"])
        .args(["-c", "commit.gpgSign=false", "-C"])
        .arg(workspace)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repository(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("alfredo-resume-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("calc.py"), "def answer():\n    return 0\n").unwrap();
    for args in [
        &["init", "-q", "--template=", "--initial-branch=main"][..],
        &["config", "user.name", "Fixture"],
        &["config", "user.email", "fixture@example.invalid"],
        &["add", "-A"],
        &["commit", "-qm", "base"],
    ] {
        git(&workspace, args);
    }
    root
}

/// A planner for two tasks and a worker whose behaviour the test switches.
fn server(mode: &Arc<AtomicU8>) -> Fixture {
    let check = |code: &str| json!(["/usr/bin/python3", "-B", "-c", code]);
    let plan = json!({"tasks": [
        {"title": "Make answer return 42", "acceptance": ["answer() returns 42"],
         "model": "fixture", "dependencies": [],
         "policy": {"files": ["calc.py"],
                    "check": check("from calc import answer; assert answer() == 42")}},
        {"title": "Add app reporting the answer", "acceptance": ["main() reports answer=42"],
         "model": "fixture", "dependencies": [1],
         "policy": {"files": ["app.py"],
                    "check": check("from app import main; assert main() == 'answer=42'")}},
    ]})
    .to_string();
    let calc = json!({"files": [{"path": "calc.py", "content": "def answer():\n    return 42\n"}]})
        .to_string();
    let app = json!({"files": [{"path": "app.py", "content":
        "from calc import answer\n\n\ndef main():\n    return f'answer={answer()}'\n"}]})
    .to_string();
    let mode = mode.clone();
    ollama_fixture::serve(ollama_fixture::reserve(), move |request, _| {
        if request.path != "POST /api/chat" {
            return ollama_fixture::running(&["fixture"]);
        }
        let planner = request.body["messages"][0]["content"]
            .as_str()
            .is_some_and(|text| text.starts_with("Act as Frontier Architect"));
        if planner {
            return done(&plan);
        }
        let prompt = request.body["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap_or_default();
        match mode.load(Ordering::SeqCst) {
            DROP => Reply::Close,
            PROSE => done("I would rather talk about it."),
            _ if prompt.starts_with("Implement this task: Add app") => done(&app),
            _ => done(&calc),
        }
    })
}

fn planner_requests(fixture: &Fixture) -> usize {
    fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| {
            request.body["messages"][0]["content"]
                .as_str()
                .is_some_and(|text| text.starts_with("Act as Frontier Architect"))
        })
        .count()
}

fn worker_requests(fixture: &Fixture) -> usize {
    fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| {
            request.body["messages"]
                .as_array()
                .and_then(|messages| messages.last())
                .and_then(|message| message["content"].as_str())
                .is_some_and(|text| text.starts_with("Implement this task"))
        })
        .count()
}

fn go(root: &Path, fixture: &Fixture, extra: &[&str]) -> Output {
    let resume = extra.contains(&"--resume");
    let mut command = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"));
    // `--resume` takes the model from the conversation, never from a flag.
    command.args(["--endpoint", &fixture.endpoint, "--connect-retries", "0"]);
    if !resume {
        command.args(["--model", "fixture", "--go", GOAL]);
    }
    command
        .arg("--workspace")
        .arg(root.join("workspace"))
        .arg("--state-dir")
        .arg(root.join("state"))
        .args(extra)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn result(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim().lines().count(),
        1,
        "one JSON line: {stdout:?}\nstderr: {}",
        stderr(output)
    );
    serde_json::from_str(stdout.trim()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A run the model server's dropped connections stopped.
fn unreachable_stop(root: &Path, fixture: &Fixture, mode: &AtomicU8, repairs: &str) -> Output {
    mode.store(DROP, Ordering::SeqCst);
    let output = go(root, fixture, &["--headless", "--max-repairs", repairs]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    output
}

fn next_step_line(err: &str) -> bool {
    err.lines()
        .any(|line| line.starts_with("next: ") && line.contains("--resume --headless"))
}

#[test]
fn an_unreachable_stop_names_the_stopped_task_and_resume_headless_as_the_next_step() {
    let root = repository("names");
    let mode = Arc::new(AtomicU8::new(ANSWER));
    let fixture = server(&mode);
    let output = unreachable_stop(&root, &fixture, &mode, "3");
    let value = result(&output);
    assert_eq!(value["state"], "failed", "{value}");
    assert_eq!(value["failed"][0]["id"], 1, "{value}");
    assert!(value["failed"][0]["reason"]
        .as_str()
        .unwrap()
        .contains(&format!("Cannot reach Ollama at {}", fixture.endpoint)));
    assert!(next_step_line(&stderr(&output)), "{}", stderr(&output));
    let _ = fs::remove_dir_all(&root);
}

/// Issue #707: a goal larger than `--num-ctx` stops `--go --headless` on the first planning
/// attempt with the provider's reason, not "Planning failed 3 times".
#[test]
fn headless_goal_over_num_ctx_stops_without_retrying_the_planner() {
    let root = repository("planctx");
    let mode = Arc::new(AtomicU8::new(ANSWER));
    let fixture = server(&mode);
    let goal = format!(
        "Make the answer 42. {}",
        "Keep the change small and tested. ".repeat(110)
    );
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--endpoint", &fixture.endpoint, "--connect-retries", "0"])
        .args(["--model", "fixture", "--num-ctx", "2048", "--go", &goal])
        .arg("--workspace")
        .arg(root.join("workspace"))
        .arg("--state-dir")
        .arg(root.join("state"))
        .arg("--headless")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(err.contains("raise --num-ctx (≈"), "{err}");
    assert!(!err.contains("failed 3 times"), "{err}");
    assert_eq!(planner_requests(&fixture), 0);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resume_headless_retries_the_stopped_task_without_planning_again() {
    let root = repository("retry");
    let mode = Arc::new(AtomicU8::new(ANSWER));
    let fixture = server(&mode);
    unreachable_stop(&root, &fixture, &mode, "3");
    mode.store(ANSWER, Ordering::SeqCst);
    let head = git(&root.join("workspace"), &["rev-parse", "HEAD"]);
    let output = go(&root, &fixture, &["--resume", "--headless"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let value = result(&output);
    assert_eq!(value["state"], "done", "{value}");
    assert_eq!(value["goal"], GOAL);
    assert_eq!(
        (&value["accepted"], &value["total"]),
        (&json!(2), &json!(2))
    );
    assert_eq!(value["failed"], json!([]));
    assert_eq!(planner_requests(&fixture), 1, "the plan is not remade");
    assert_eq!(git(&root.join("workspace"), &["rev-parse", "HEAD"]), head);
    let branch = value["branch"].as_str().unwrap();
    assert!(branch.starts_with("alfredo/go-"), "{value}");
    assert_eq!(
        git(
            &root.join("workspace"),
            &["show", &format!("{branch}:calc.py")]
        ),
        "def answer():\n    return 42"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_retry_that_meets_the_unreachable_model_again_stops_at_once_with_the_next_step() {
    let root = repository("again");
    let mode = Arc::new(AtomicU8::new(ANSWER));
    let fixture = server(&mode);
    unreachable_stop(&root, &fixture, &mode, "1");
    let started = Instant::now();
    let output = go(&root, &fixture, &["--resume", "--headless"]);
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "a spent retry must not wait out its timeout"
    );
    assert!(
        !stderr(&output).contains("did not restart"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let value = result(&output);
    assert_eq!(value["state"], "failed", "{value}");
    assert_eq!(
        value["failed"][0]["id"], 3,
        "the retry task is the one that stopped"
    );
    assert!(next_step_line(&stderr(&output)), "{}", stderr(&output));
    mode.store(ANSWER, Ordering::SeqCst);
    let output = go(&root, &fixture, &["--resume", "--headless"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(result(&output)["state"], "done");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_retry_that_fails_otherwise_ends_without_waiting_and_without_a_next_step() {
    let root = repository("spent");
    let mode = Arc::new(AtomicU8::new(ANSWER));
    let fixture = server(&mode);
    unreachable_stop(&root, &fixture, &mode, "1");
    // The server is back but its answers are unusable: the retry and one automatic
    // repair run (--max-repairs 1 + 1), then the family stops for its budget.
    mode.store(PROSE, Ordering::SeqCst);
    let started = Instant::now();
    let output = go(&root, &fixture, &["--resume", "--headless"]);
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let value = result(&output);
    assert_eq!(
        (&value["state"], &value["repairs"]),
        (&json!("failed"), &json!(2))
    );
    assert!(value["failed"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("repair budget exhausted (2/2)"));
    assert!(!stderr(&output).contains("did not restart"));
    assert!(!next_step_line(&stderr(&output)), "{}", stderr(&output));
    assert_eq!(planner_requests(&fixture), 1);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resume_headless_after_any_other_failure_exits_2_nothing_to_resume() {
    let root = repository("other");
    let mode = Arc::new(AtomicU8::new(PROSE));
    let fixture = server(&mode);
    let output = go(&root, &fixture, &["--headless", "--max-repairs", "0"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(result(&output)["state"], "failed");
    assert!(!next_step_line(&stderr(&output)), "{}", stderr(&output));
    let before = worker_requests(&fixture);
    let output = go(&root, &fixture, &["--resume", "--headless"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(output.stdout.is_empty());
    assert!(
        stderr(&output).contains("nothing to resume"),
        "{}",
        stderr(&output)
    );
    assert_eq!(planner_requests(&fixture), 1);
    assert_eq!(worker_requests(&fixture), before, "no task ran");
    let _ = fs::remove_dir_all(&root);
}

/// Issue #696: at the 256-task limit `--go` is a setup refusal before any planner request
/// (exit 2, nothing on stdout, the limit and the way out on stderr) and leaves no paused
/// run, so `--resume --headless` has nothing to resume.
#[test]
fn go_at_the_task_limit_is_refused_before_planning_and_leaves_nothing_to_resume() {
    let root = repository("capacity");
    let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "default").unwrap();
    let mut snapshot = store.snapshot().unwrap();
    while snapshot.tasks.len() < 256 {
        snapshot = store
            .transact(Request {
                correlation: format!("pad-{}", snapshot.revision),
                expected_revision: snapshot.revision,
                action: Action::Propose {
                    title: "filler".into(),
                    model: "fixture".into(),
                    dependencies: vec![],
                },
            })
            .unwrap()
            .0;
    }
    let fixture = server(&Arc::new(AtomicU8::new(ANSWER)));
    let output = go(&root, &fixture, &["--headless"]);
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(2), "{err}");
    assert!(
        output.stdout.is_empty(),
        "setup errors print nothing on stdout"
    );
    for expected in [
        "--go refused",
        "256",
        "--new-mission NAME",
        "Start New Mission",
    ] {
        assert!(err.contains(expected), "{expected}: {err}");
    }
    assert!(!err.contains("no effect after"), "{err}");
    assert_eq!(planner_requests(&fixture), 0, "a planner request was sent");
    assert_eq!(worker_requests(&fixture), 0);
    let output = go(&root, &fixture, &["--resume", "--headless"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("nothing to resume"),
        "{}",
        stderr(&output)
    );
    assert_eq!(planner_requests(&fixture), 0);
    let _ = fs::remove_dir_all(&root);
}
