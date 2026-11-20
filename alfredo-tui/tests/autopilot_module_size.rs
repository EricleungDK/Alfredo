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

/// #742: clippy pedantic `must_use_candidate` flagged these public helpers;
/// each must carry `#[must_use]` directly above its signature.
#[test]
fn public_helpers_are_must_use() {
    let text = source("autopilot.rs");
    let lines: Vec<&str> = text.lines().collect();
    for name in [
        "from_numstat",
        "row",
        "json",
        "cancels_chat_turn",
        "queue_held",
        "settles_draft_after_command",
        "finish_notice",
        "stop_notice",
    ] {
        let needle = format!("pub fn {name}(");
        let at = lines
            .iter()
            .position(|l| l.trim_start().starts_with(&needle))
            .unwrap_or_else(|| panic!("autopilot.rs lacks {needle}"));
        let mut i = at;
        let mut found = false;
        while i > 0 {
            i -= 1;
            let prev = lines[i].trim();
            if prev == "#[must_use]" {
                found = true;
                break;
            }
            if !prev.starts_with("#[") && !prev.starts_with("///") {
                break;
            }
        }
        assert!(found, "pub fn {name} in autopilot.rs lacks #[must_use]");
    }
}
