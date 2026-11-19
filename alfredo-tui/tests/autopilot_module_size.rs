//! M15 code-health bound for the autopilot module (#709): the plan-lint and
//! active-run refusal helpers live in `src/autopilot_refusal.rs`, which keeps
//! `src/autopilot.rs` (the largest module) within its line budget.

use std::{fs, path::Path};

const MAX_AUTOPILOT_LINES: usize = 2650;

fn source(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)).unwrap()
}

#[test]
fn autopilot_module_stays_within_its_line_budget() {
    let lines = source("autopilot.rs").lines().count();
    assert!(
        lines <= MAX_AUTOPILOT_LINES,
        "src/autopilot.rs has {lines} lines (budget {MAX_AUTOPILOT_LINES}); move cohesive helpers to their own module"
    );
}

#[test]
fn refusal_helpers_live_in_their_own_module() {
    let refusal = source("autopilot_refusal.rs");
    for helper in ["fn lint_refusal(", "fn draft_matches(", "fn scope_blocker("] {
        assert!(
            refusal.contains(helper),
            "autopilot_refusal.rs lacks {helper}"
        );
        assert!(
            !source("autopilot.rs").contains(helper),
            "autopilot.rs still defines {helper}"
        );
    }
}
