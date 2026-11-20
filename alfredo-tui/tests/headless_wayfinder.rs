//! Issue #741: `--go GOAL --headless` refuses only goals that read as a project start, and
//! the refusal names the phrase that matched. The refusal comes before any Git or model
//! access, so no repository or server is needed.
use std::{
    fs,
    process::{Command, Stdio},
};

fn headless(goal: &str, tag: &str) -> (Option<i32>, String) {
    let root = std::env::temp_dir().join(format!("alfredo-hw-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("workspace")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--model", "x", "--endpoint", "http://127.0.0.1:1"])
        .args(["--connect-retries", "0", "--go", goal, "--headless"])
        .arg("--workspace")
        .arg(root.join("workspace"))
        .arg("--state-dir")
        .arg(root.join("state"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&root);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_fix_goal_that_mentions_a_new_app_is_not_refused_as_a_wayfinder_goal() {
    let (code, err) = headless("fix the new app crash on startup", "fix");
    assert!(!err.contains("Wayfinder goals need"), "{err}");
    assert!(err.contains("is not a Git repository"), "{err}");
    assert_eq!(code, Some(2), "{err}");
}

/// Issue #752: everyday coding goals with a creation verb a few words before "the new ..."
/// plan normally instead of being refused as a project start.
#[test]
fn coding_goals_with_a_verb_near_the_new_app_are_not_refused() {
    for (tag, goal) in [
        ("tests", "write tests for the new service"),
        ("sure", "make sure the new app starts"),
        ("crash", "fix the crash when I start the new app"),
    ] {
        let (code, err) = headless(goal, tag);
        assert!(!err.contains("Wayfinder goals need"), "{goal}: {err}");
        assert!(err.contains("is not a Git repository"), "{goal}: {err}");
        assert_eq!(code, Some(2), "{err}");
    }
}

#[test]
fn a_project_start_goal_is_refused_naming_the_phrase_and_the_way_out() {
    let (code, err) = headless("Build a new app that prints hello", "build");
    assert_eq!(code, Some(2), "{err}");
    assert!(
        err.contains(
            "Wayfinder goals need the interactive terminal; \"build a new app\" reads as a \
             project start. Rephrase it as a coding goal, or use the terminal"
        ),
        "{err}"
    );
}

/// Issue #763: "build me a new app" is a project start like "create a new app".
#[test]
fn a_pronoun_phrased_project_start_goal_is_refused_like_the_plain_one() {
    let (code, err) = headless("build me a new app", "pronoun");
    assert_eq!(code, Some(2), "{err}");
    assert!(
        err.contains(
            "Wayfinder goals need the interactive terminal; \"build me a new app\" reads as a \
             project start. Rephrase it as a coding goal, or use the terminal"
        ),
        "{err}"
    );
}

#[test]
fn a_goal_that_enters_wayfinder_another_way_keeps_the_plain_refusal() {
    let (code, err) = headless("plan a migration of the whole system", "plan");
    assert_eq!(code, Some(2), "{err}");
    assert!(
        err.contains("Wayfinder goals need the interactive terminal; state a coding goal"),
        "{err}"
    );
    assert!(!err.contains("reads as a project start"), "{err}");
}
