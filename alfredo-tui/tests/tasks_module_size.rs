//! M15 code-health bound for the task store (#702): the refusal and message
//! helpers live in `src/tasks/refusal.rs`, which keeps `src/tasks.rs` (the
//! store core) within its line budget.

use std::{fs, path::Path};

const MAX_TASKS_LINES: usize = 2650;

fn source(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)).unwrap()
}

#[test]
fn tasks_module_stays_within_its_line_budget() {
    let lines = source("tasks.rs").lines().count();
    assert!(
        lines <= MAX_TASKS_LINES,
        "src/tasks.rs has {lines} lines (budget {MAX_TASKS_LINES}); move cohesive helpers to their own module"
    );
}

#[test]
fn refusal_helpers_live_in_their_own_module() {
    let refusal = source("tasks/refusal.rs");
    let tasks = source("tasks.rs");
    for helper in [
        "fn capacity_refusal(",
        "fn is_capacity_refusal(",
        "fn status_label(",
        "fn running_hint(",
        "fn review_refusal(",
        "fn run_refusal(",
        "fn not_run(",
        "enum Refusal ",
    ] {
        assert!(refusal.contains(helper), "tasks/refusal.rs lacks {helper}");
        assert!(!tasks.contains(helper), "tasks.rs still defines {helper}");
    }
}
