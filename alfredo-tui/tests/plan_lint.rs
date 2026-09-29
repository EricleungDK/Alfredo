//! Unattended plan validation: checks must be runnable from committed files,
//! the task's own files or its dependencies' files.
use alfredo_tui::{
    plan_lint,
    planner::{Plan, Step},
    tasks::WorkPolicy,
};

fn step(files: &[&str], check: &[&str], dependencies: &[u64]) -> Step {
    Step {
        title: "Task".into(),
        acceptance: vec!["works".into()],
        model: "fixture".into(),
        dependencies: dependencies.to_vec(),
        policy: WorkPolicy {
            files: files.iter().map(|s| s.to_string()).collect(),
            check: check.iter().map(|s| s.to_string()).collect(),
        },
    }
}
fn plan(tasks: Vec<Step>) -> Plan {
    Plan {
        prompt: "goal".into(),
        planner: "fixture".into(),
        tasks,
        context: None,
        scope: None,
        architecture: None,
    }
}
fn tracked(path: &str) -> bool {
    ["README.md", "tests/test_a.py", "pkg/__init__.py"]
        .iter()
        .any(|known| *known == path || known.starts_with(&format!("{path}/")))
}
fn lint(plan: &Plan) -> Vec<String> {
    plan_lint::lint(plan, &tracked, &|program| program != "pytest")
}

#[test]
fn check_needing_a_later_tasks_file_is_rejected_with_fix_options() {
    let plan = plan(vec![
        step(
            &["textutil.py"],
            &["python3", "-m", "unittest", "test_textutil.py"],
            &[],
        ),
        step(
            &["test_textutil.py"],
            &["python3", "-m", "unittest", "test_textutil.py"],
            &[1],
        ),
    ]);
    assert_eq!(
        lint(&plan),
        vec!["Task 1 check references test_textutil.py, which does not exist yet and is not written by task 1 or its dependencies. Either write it in the same task, make the task depend on the task that writes it, or use a check that only needs this task's files.".to_string()]
    );
}

#[test]
fn own_files_transitive_dependencies_and_committed_paths_satisfy_checks() {
    let plan = plan(vec![
        step(&["a.py"], &["python3", "./a.py"], &[]),
        step(&["b.py"], &["python3", "b.py"], &[1]),
        step(
            &["c.py"],
            &[
                "python3",
                "-m",
                "unittest",
                "a.py",
                "tests/",
                "README.md",
                "pkg",
            ],
            &[2],
        ),
        step(&["lib/x.js"], &["node", "lib"], &[]),
    ]);
    assert_eq!(lint(&plan), Vec::<String>::new());
}

#[test]
fn unittest_module_arguments_map_to_python_files() {
    let bad = plan(vec![step(
        &["textutil.py"],
        &["python3", "-m", "unittest", "test_textutil"],
        &[],
    )]);
    let findings = lint(&bad);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].starts_with("Task 1 check references test_textutil.py,"),
        "{findings:?}"
    );
    let nested = plan(vec![step(
        &["textutil.py"],
        &["python3", "-B", "-m", "unittest", "-v", "app.test_mod"],
        &[],
    )]);
    assert!(lint(&nested)[0].contains("references app/test_mod.py,"));
    let good = plan(vec![
        step(
            &["pkg/test_mod.py"],
            &["python3", "-m", "unittest", "pkg.test_mod"],
            &[],
        ),
        step(
            &["test_x.py"],
            &[
                "python3",
                "-m",
                "unittest",
                "test_x.TestX.test_y",
                "tests.test_a",
            ],
            &[],
        ),
    ]);
    assert_eq!(lint(&good), Vec::<String>::new());
}

#[test]
fn unclassifiable_arguments_are_never_rejected() {
    let plan = plan(vec![
        step(&["a.py"], &["python3", "-c", "import textutil"], &[]),
        step(&["b.py"], &["python3", "-c", "x.py"], &[]),
        step(
            &["c.py"],
            &["python3", "--version=3.11", "3.11", "-k", "x.y"],
            &[],
        ),
        step(
            &["d.py"],
            &[
                "python3",
                "-m",
                "unittest",
                "discover",
                "-s",
                "tests",
                "-p",
                "test_*.py",
            ],
            &[],
        ),
        step(
            &["e.py"],
            &["python3", "-m", "unittest", "discover", "missing"],
            &[],
        ),
        step(
            &["f.py"],
            &[
                "python3",
                "/opt/x.py",
                "../x.py",
                "http://example.com/a.py",
                "-o",
                "out.txt",
            ],
            &[],
        ),
        step(&["g.py"], &["python3", "-m", "pytest.main"], &[]),
    ]);
    assert_eq!(lint(&plan), Vec::<String>::new());
}

#[test]
fn unrelated_tasks_writing_the_same_file_are_rejected() {
    let bad = plan(vec![
        step(&["a.py"], &["true"], &[]),
        step(&["b.py"], &["true"], &[]),
        step(&["a.py", "c.py"], &["true"], &[2]),
    ]);
    assert_eq!(
        lint(&bad),
        vec!["Tasks 1 and 3 both write a.py but neither depends on the other. Merge them or make one depend on the other.".to_string()]
    );
    let ordered = plan(vec![
        step(&["a.py"], &["true"], &[]),
        step(&["b.py"], &["true"], &[1]),
        step(&["a.py"], &["true"], &[2]),
    ]);
    assert_eq!(lint(&ordered), Vec::<String>::new());
}

#[test]
fn empty_files_shell_strings_and_missing_programs_are_rejected() {
    let bad = plan(vec![
        step(&[], &["true"], &[]),
        step(&["a.py"], &["python3 -m unittest"], &[]),
        step(&["b.py"], &["pytest", "b.py"], &[]),
    ]);
    let findings = lint(&bad);
    assert_eq!(findings.len(), 3, "{findings:?}");
    assert_eq!(
        findings[0],
        "Task 1 lists no policy files; every task must write at least one file."
    );
    assert!(findings[1].starts_with("Task 2 check must be an argv array"));
    assert!(
        findings[2]
            .starts_with("Task 3 check program \"pytest\" is not available in the check sandbox"),
        "{findings:?}"
    );
}

#[test]
fn sandbox_program_lookup_mirrors_the_check_sandbox_path() {
    assert!(plan_lint::sandbox_program("sh"));
    assert!(plan_lint::sandbox_program("/bin/sh"));
    assert!(!plan_lint::sandbox_program("alfredo-no-such-program-x9"));
    assert!(!plan_lint::sandbox_program(
        "/usr/bin/alfredo-no-such-program-x9"
    ));
    // Outside the read-only system roots bound into the sandbox.
    assert!(!plan_lint::sandbox_program("/opt/alfredo/bin/tool"));
    // Relative to the worktree: not resolvable here, so never rejected.
    assert!(plan_lint::sandbox_program("./run.sh"));
    assert!(plan_lint::sandbox_program("scripts/check.sh"));
}
