//! A command listed in F1 must never be reported as unknown when typed bare.
use alfredo_tui::{
    agent_view, autopilot, autopilot::Autopilot, commands::COMMANDS, model::App,
    task_control::TaskControl, tasks::TaskStore,
};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};
static ID: AtomicU64 = AtomicU64::new(0);
use tokio::runtime::Runtime;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-bare-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("workspace")).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
        ] {
            assert!(Command::new("git")
                .arg("-C")
                .arg(root.join("workspace"))
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn control(runtime: &Runtime, store: TaskStore) -> TaskControl {
    let mut tasks = TaskControl::new(store);
    tasks.refresh(runtime);
    let deadline = Instant::now() + Duration::from_secs(5);
    while tasks.pending {
        tasks.poll();
        assert!(Instant::now() < deadline, "{}", tasks.notice);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(tasks.snapshot.is_some(), "{}", tasks.notice);
    tasks
}

/// Mirrors the Enter routing of the terminal loop for a typed command.
fn submit(
    runtime: &Runtime,
    app: &mut App,
    tasks: &mut TaskControl,
    autopilot: &mut Autopilot,
    text: &str,
) -> String {
    let text = text.trim();
    let verb = text.split_whitespace().next().unwrap_or("");
    if autopilot::is_command(text) {
        return autopilot
            .command(runtime, tasks, text, "model", 1)
            .unwrap_or_else(|error| error);
    }
    if matches!(verb, "/watch" | "/tell") {
        return match agent_view::console(app, tasks, text) {
            Some(Ok(notice)) | Some(Err(notice)) => notice,
            None => String::new(),
        };
    }
    match tasks.prepare_command(text, "model") {
        Err(error) => error,
        Ok(Some(_)) => String::new(),
        Ok(None) => match tasks.command(runtime, text, "model") {
            Ok(()) => String::new(),
            Err(error) => error,
        },
    }
}

#[test]
fn every_listed_command_typed_bare_is_known() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut unknown = vec![];
    for (command, _) in COMMANDS {
        // The terminal loop answers these exact words itself, before any parser.
        if matches!(*command, "/workspace" | "/models" | "/model") {
            continue;
        }
        for typed in [command.to_string(), format!("{command}   ")] {
            let state = fixture
                .0
                .join(format!("state{}", command.replace('/', "-")));
            let store = TaskStore::new(&state, &fixture.0.join("workspace"), "bare").unwrap();
            let mut tasks = control(&runtime, store);
            let mut app = App::new("fixture".into());
            let mut autopilot = Autopilot::open(&state, "default").unwrap();
            let message = submit(&runtime, &mut app, &mut tasks, &mut autopilot, &typed);
            if message.contains("Unknown command") {
                unknown.push(format!("{typed:?}: {message}"));
            }
            assert!(
                tasks.snapshot.as_ref().is_none_or(|s| s.tasks.is_empty()),
                "{typed:?} created a task"
            );
        }
    }
    assert!(unknown.is_empty(), "{unknown:#?}");
}

#[test]
fn bare_task_and_after_print_usage_without_creating_work() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let store =
        TaskStore::new(&fixture.0.join("state"), &fixture.0.join("workspace"), "u").unwrap();
    let mut tasks = control(&runtime, store);
    let mut app = App::new("fixture".into());
    let mut autopilot = Autopilot::open(&fixture.0.join("state"), "default").unwrap();
    for (typed, usage) in [
        ("/task", "Usage: /task DESCRIPTION"),
        ("/task   ", "Usage: /task DESCRIPTION"),
        ("/after", "Usage: /after IDS DESCRIPTION"),
        ("/after 1", "Usage: /after IDS DESCRIPTION"),
        ("/after 1,2", "Usage: /after IDS DESCRIPTION"),
        ("/after 1,2   ", "Usage: /after IDS DESCRIPTION"),
    ] {
        assert_eq!(
            submit(&runtime, &mut app, &mut tasks, &mut autopilot, typed),
            usage,
            "{typed:?}"
        );
    }
    assert!(tasks.snapshot.as_ref().is_none_or(|s| s.tasks.is_empty()));
}
