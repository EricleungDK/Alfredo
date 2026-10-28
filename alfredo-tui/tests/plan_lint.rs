//! Unattended plan validation: checks must be runnable from committed files,
//! the task's own files or its dependencies' files.
use alfredo_tui::{
    plan_lint,
    planner::{Plan, Step},
    planning_context::RepositoryContext,
    tasks::WorkPolicy,
    understanding::Binding,
};
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
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
        usage: None,
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
    plan_lint::lint(plan, &tracked, &|program| program != "pytest", &size)
}
fn size(path: &str) -> Option<u64> {
    (path == "big.py").then_some(156_697)
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

#[test]
fn runtime_data_files_in_policy_are_rejected_with_a_fix() {
    let bad = plan(vec![
        step(&["todo.py", "todo.json"], &["true"], &[]),
        step(&["data/app.sqlite"], &["true"], &[]),
    ]);
    let findings = lint(&bad);
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(
        findings[0].starts_with("Task 1 lists runtime data file todo.json"),
        "{findings:?}"
    );
    assert!(findings[0].contains("temp dir"), "{findings:?}");
    assert!(findings[1].starts_with("Task 2 lists runtime data file data/app.sqlite"));
}

#[test]
fn manifests_configs_fixtures_and_docs_are_not_runtime_data() {
    let ok = plan(vec![step(
        &[
            "package.json",
            "package-lock.json",
            "tsconfig.json",
            "tsconfig.build.json",
            "Cargo.toml",
            ".eslintrc.json",
            "app.config.json",
            "tests/fixtures/sample.json",
            "testdata/users.csv",
            "expected_output.json",
            "notes.txt",
            "todo.py",
        ],
        &["true"],
        &[],
    )]);
    assert_eq!(lint(&ok), Vec::<String>::new());
}

#[test]
fn committed_data_file_may_be_edited() {
    let edit = plan(vec![step(&["state.json"], &["true"], &[])]);
    let findings = plan_lint::lint(&edit, &|p| p == "state.json", &|_| true, &|_| None);
    assert_eq!(findings, Vec::<String>::new());
}

#[test]
fn committed_file_over_the_worker_bound_is_rejected_with_the_worker_message() {
    let large = plan(vec![step(&["big.py"], &["python3", "-c", "print(1)"], &[])]);
    let all = |_: &str| true;
    assert_eq!(
        plan_lint::lint(&large, &all, &all, &size),
        vec!["big.py is 154 KiB; workers can edit files up to 64 KiB. Split the file, or edit it by hand.".to_string()]
    );
    let small = step(&["calc.py"], &["python3", "-c", "print(1)"], &[]);
    assert_eq!(
        plan_lint::lint(&plan(vec![small]), &all, &all, &size),
        Vec::<String>::new()
    );
}

#[test]
fn file_the_plan_creates_is_not_sized_and_is_not_flagged() {
    // `size` answers only for committed blobs; a new file has none at the baseline.
    let new_file = plan(vec![step(
        &["fresh.py"],
        &["python3", "-c", "print(1)"],
        &[],
    )]);
    assert_eq!(lint(&new_file), Vec::<String>::new());
    let exact = |_: &str| Some(64 * 1024);
    let all = |_: &str| true;
    assert_eq!(
        plan_lint::lint(&new_file, &all, &all, &exact),
        Vec::<String>::new(),
        "exactly 64 KiB is within the bound"
    );
}

/// A scratch repository with one empty commit; `config` is appended to its `.git/config`.
/// Returns the directory and the commit (the plan baseline).
fn repository(name: &str, config: &str) -> (PathBuf, String) {
    let root =
        std::env::temp_dir().join(format!("alfredo-plan-lint-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let run = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    run(&["init", "-q"]);
    run(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@example.invalid",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "base",
    ]);
    let head = run(&["rev-parse", "HEAD"]);
    if !config.is_empty() {
        fs::OpenOptions::new()
            .append(true)
            .open(root.join(".git/config"))
            .unwrap()
            .write_all(config.as_bytes())
            .unwrap();
    }
    (root, head)
}

/// A plan whose baseline listing was truncated (300 paths, 1 shown), so the lint asks git
/// about every file the listing does not show.
fn scoped_plan(root: &Path, baseline: &str, files: &[&str], check: &[&str]) -> Plan {
    let mut draft = plan(vec![step(files, check, &[])]);
    draft.context = Some(RepositoryContext {
        baseline: baseline.into(),
        paths: vec!["README.md".into()],
        omitted_paths: 300,
        sources: vec![],
        omitted_sources: 0,
        listing_truncated: false,
    });
    draft.scope = Some(Box::new(Binding {
        workspace: root.into(),
        revision: 1,
        draft_revision: 1,
        brief: None,
        confirmed: false,
    }));
    draft
}

/// Opens `fifo` for writing, without blocking, until `done` is set. A git that
/// blocks reading the FIFO in open() then goes on, so no git process is left stuck.
fn release_fifo_until(fifo: PathBuf, done: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !done.load(Ordering::SeqCst) {
            let _ = fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&fifo);
            thread::sleep(Duration::from_millis(5));
        }
    })
}

#[test]
fn a_config_include_fifo_never_hangs_the_lint() {
    let (root, baseline) = repository("include-fifo", "");
    let fifo = root.join("blocked.config");
    assert!(Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    fs::OpenOptions::new()
        .append(true)
        .open(root.join(".git/config"))
        .unwrap()
        .write_all(format!("[include]\n\tpath = {}\n", fifo.display()).as_bytes())
        .unwrap();
    // `notes.txt` is a file the listing does not show: the lint sizes it with git.
    let draft = scoped_plan(&root, &baseline, &["notes.txt"], &["sh", "run.sh"]);
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(plan_lint::findings(&draft));
    });
    let answer = receiver.recv_timeout(Duration::from_secs(20));
    // Only a stalled lint is released; a lint that returned in time never touched the FIFO.
    let done = Arc::new(AtomicBool::new(false));
    let releaser = release_fifo_until(fifo, done.clone());
    if answer.is_err() {
        // Let the stalled lint finish, so no git is left blocked on the FIFO.
        let _ = receiver.recv_timeout(Duration::from_secs(20));
    }
    done.store(true, Ordering::SeqCst);
    releaser.join().unwrap();
    let findings = answer.expect("plan_lint::findings blocked on a FIFO include (#519)");
    // Unknown answers count as present, so nothing is reported.
    assert_eq!(findings, Vec::<String>::new());
}

#[test]
fn a_repository_config_with_filter_drivers_leaves_git_answers_unknown() {
    let (plain, plain_head) = repository("filter-control", "");
    let control = plan_lint::findings(&scoped_plan(
        &plain,
        &plain_head,
        &["out.txt"],
        &["sh", "missing/run.sh"],
    ));
    // Control: without the filter the missing check file is reported, so the lint does run.
    assert_eq!(control.len(), 1, "{control:?}");
    assert!(
        control[0].starts_with("Task 1 check references missing/run.sh,"),
        "{control:?}"
    );
    let (filtered, filtered_head) =
        repository("filter-config", "[filter \"lfs\"]\n\tclean = cat\n");
    let findings = plan_lint::findings(&scoped_plan(
        &filtered,
        &filtered_head,
        &["out.txt"],
        &["sh", "missing/run.sh"],
    ));
    assert_eq!(findings, Vec::<String>::new());
}
