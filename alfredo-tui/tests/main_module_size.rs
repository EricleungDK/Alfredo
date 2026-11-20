//! M15 code-health bound for the binary entry point (#753): the CLI-support
//! helpers live in `src/cli_support.rs`, which keeps `src/main.rs` within its
//! line budget.

use std::{fs, path::Path};

const MAX_MAIN_LINES: usize = 2650;

fn source(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)).unwrap()
}

/// Lines before the inline `#[cfg(test)]` module, the same split `--code-health` uses.
fn non_test_lines(text: &str) -> usize {
    text.lines()
        .take_while(|line| line.trim() != "#[cfg(test)]")
        .count()
}

#[test]
fn main_stays_within_its_line_budget() {
    let lines = non_test_lines(&source("main.rs"));
    assert!(
        lines <= MAX_MAIN_LINES,
        "src/main.rs has {lines} non-test lines (budget {MAX_MAIN_LINES}); move cohesive helpers to their own module"
    );
}

#[test]
fn cli_support_helpers_live_in_their_own_module() {
    let support = source("cli_support.rs");
    let main = source("main.rs");
    for helper in [
        "fn launch_error_message(",
        "const KNOWN_FLAGS:",
        "const STDIN_PROMPT_CAP:",
        "fn read_stdin_prompt(",
    ] {
        assert!(support.contains(helper), "cli_support.rs lacks {helper}");
        assert!(!main.contains(helper), "main.rs still defines {helper}");
    }
}
