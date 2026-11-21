use std::process::Command;

#[test]
fn help_and_version_work_without_a_terminal_or_model_server() {
    for flag in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
            .arg(flag)
            .env("OLLAMA_HOST", "not-a-valid-endpoint")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
        assert!(!output.stdout.contains(&27));
    }
}

/// Runs the binary with stdout piped, drops the read end at once (like `| head -0`), and
/// returns the exit status and stderr.
fn run_with_closed_stdout(args: &[&str]) -> (std::process::ExitStatus, String) {
    use std::{io::Read, process::Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(args)
        .env("OLLAMA_HOST", "http://127.0.0.1:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (child.wait().unwrap(), stderr)
}

#[test]
fn version_into_a_closed_pipe_exits_quietly_with_status_zero() {
    for flag in ["--version", "-V"] {
        let (status, stderr) = run_with_closed_stdout(&[flag]);
        assert!(!stderr.contains("panicked"), "{flag}: {stderr}");
        assert!(stderr.is_empty(), "{flag}: {stderr}");
        assert_eq!(status.code(), Some(0), "{flag}");
    }
}

#[test]
fn help_into_a_closed_pipe_exits_quietly_with_status_zero() {
    for flag in ["--help", "-h"] {
        let (status, stderr) = run_with_closed_stdout(&[flag]);
        assert!(!stderr.contains("panicked"), "{flag}: {stderr}");
        assert!(stderr.is_empty(), "{flag}: {stderr}");
        assert_eq!(status.code(), Some(0), "{flag}");
    }
}

#[test]
fn doctor_into_a_closed_pipe_prints_no_panic() {
    let dir = std::env::temp_dir().join(format!("alfredo-cli-doctor-pipe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    assert!(Command::new("git")
        .args([
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "init",
            "-q"
        ])
        .arg(&workspace)
        .status()
        .unwrap()
        .success());
    let (status, stderr) = run_with_closed_stdout(&[
        "--doctor",
        "--endpoint",
        "http://127.0.0.1:1",
        "--workspace",
        workspace.to_str().unwrap(),
        "--state-dir",
        dir.join("state").to_str().unwrap(),
    ]);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_eq!(status.code(), Some(0), "{stderr}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn version_and_help_into_a_fully_read_pipe_keep_their_exit_status_and_text() {
    let version = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        format!("alfredo-tui {}\n", env!("CARGO_PKG_VERSION"))
    );
    let help = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.stdout.ends_with(b"\n"));
    assert!(help.stderr.is_empty());
}

#[test]
fn help_documents_zero_ceremony_start_and_select_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--select", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--select"));
    assert!(help.contains("mission default"));
    assert!(!help.contains("migration in progress"));
    let quickstart: Vec<_> = help.lines().take(3).collect();
    assert!(quickstart[0].starts_with("Quickstart:"), "{help}");
    assert!(quickstart[1].contains("/go"), "{help}");
    assert!(quickstart[2].contains("F1 help"), "{help}");
}

#[test]
fn invalid_arguments_fail_before_entering_terminal_mode() {
    for args in [
        vec!["--model"],
        vec!["--model", ""],
        vec!["--unknown"],
        vec!["--mission", "one", "--new-mission", "two"],
        vec!["--new-mission"],
        vec!["--endpoint", "file:///tmp"],
        vec!["--keep-alive"],
        vec!["--keep-alive", "bogus"],
        vec!["--num-ctx"],
        vec!["--num-ctx", "2047"],
        vec!["--num-ctx", "262145"],
        vec!["--num-ctx", "big"],
        vec!["--max-turns"],
        vec!["--max-turns", "0"],
        vec!["--max-turns", "33"],
        vec!["--max-turns", "many"],
        vec!["--connect-retries"],
        vec!["--connect-retries", "-1"],
        vec!["--connect-retries", "11"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.contains(&27));
    }
}

#[test]
fn connection_flags_are_documented_and_invalid_environment_fails_early() {
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    for text in [
        "--keep-alive",
        "--connect-retries",
        "ALFREDO_KEEP_ALIVE",
        "--num-ctx",
        "ALFREDO_NUM_CTX",
        "--max-turns",
        "ALFREDO_MAX_TURNS",
    ] {
        assert!(help.contains(text), "{text}");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--endpoint", "http://127.0.0.1:1", "--model", "fixture"])
        .env("ALFREDO_KEEP_ALIVE", "bogus")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("keep-alive"));
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args([
            "--endpoint",
            "http://127.0.0.1:1",
            "--model",
            "fixture",
            "--keep-alive",
            "default",
            "--connect-retries",
            "0",
        ])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("Interactive terminal required"));
}

#[test]
fn worker_format_flag_is_documented_and_validated() {
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--worker-format blocks|json"), "{help}");
    for args in [vec!["--worker-format"], vec!["--worker-format", "xml"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("--worker-format needs blocks or json"));
    }
    for value in ["blocks", "json"] {
        let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
            .args([
                "--endpoint",
                "http://127.0.0.1:1",
                "--model",
                "fixture",
                "--worker-format",
                value,
            ])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&output.stderr).contains("Interactive terminal required"));
    }
}

#[test]
fn piped_launch_reports_how_to_get_usage_without_terminal_escapes() {
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--endpoint", "http://127.0.0.1:1", "--model", "fixture"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Interactive terminal required"));
    assert!(output.stdout.is_empty());
}

#[test]
fn startup_errors_print_as_plain_text_without_debug_quotes() {
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .arg("--model")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("Error: --model"), "{stderr}");
    assert!(!stderr.contains('"'), "{stderr}");
}

#[test]
fn mission_flags_outside_a_repository_name_workspace_instead_of_being_ignored() {
    let dir = std::env::temp_dir().join(format!("alfredo-cli-norepo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for flag in ["--mission", "--new-mission"] {
        let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
            .args([flag, "alpha", "--state-dir"])
            .arg(dir.join("state"))
            .current_dir(&dir)
            .env("GIT_CEILING_DIRECTORIES", &dir)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(
            error.contains(flag) && error.contains("--workspace DIR"),
            "{error}"
        );
        assert!(!error.contains("Interactive terminal"), "{error}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn help_agents_line_says_ctrl_o_expands_instruction_and_each_files_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    let agents = help
        .lines()
        .find(|line| line.starts_with("Agents:"))
        .expect("Agents line");
    assert!(
        agents.contains("Ctrl+O expand instructions and each file's code"),
        "{agents}"
    );
}

#[test]
fn help_headless_entry_says_it_opens_the_default_or_named_mission() {
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    let entry = help
        .split("--headless (with --go GOAL)")
        .nth(1)
        .unwrap_or_else(|| panic!("no --headless entry in help: {help}"));
    let entry = entry.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        entry.contains(
            "Opens the repository's default mission, or the one --mission NAME / --new-mission NAME names."
        ),
        "{entry}"
    );
}

#[test]
fn help_says_resume_headless_retries_a_context_too_small_stop() {
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    let entry = help
        .lines()
        .find(|line| line.starts_with("--resume --headless"))
        .expect("the --resume --headless entry");
    assert!(entry.contains("model-unreachable"), "{entry}");
    assert!(
        entry.contains("context-too-small") && entry.contains("--num-ctx"),
        "{entry}"
    );
}

/// Issue #765: an explicit `--on-finish` / `--max-repairs` where it cannot apply is a usage
/// error (exit 2) that names the flag and where it applies, and nothing runs.
fn flag_run(args: &[&str], hook_env: Option<&str>) -> (std::process::Output, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!(
        "alfredo-cli-refusal-{}-{}",
        std::process::id(),
        args.join("_").replace(['/', ' '], "")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let marker = root.join("hook.out");
    let hook = root.join("hook.sh");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\necho ran > {}\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"));
    command
        .env("ALFREDO_MODEL", "fixture")
        .args(["--endpoint", "http://127.0.0.1:9"])
        .arg("--state-dir")
        .arg(root.join("state"))
        .arg("--workspace")
        .arg(&root);
    for arg in args {
        if *arg == "HOOK" {
            command.arg(&hook);
        } else {
            command.arg(arg);
        }
    }
    if hook_env.is_some() {
        command.env("ALFREDO_ON_FINISH", &hook);
    }
    (
        command.stdin(std::process::Stdio::null()).output().unwrap(),
        marker,
    )
}

fn assert_refused(args: &[&str], flag: &str) {
    let (output, marker) = flag_run(args, None);
    let err = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{args:?}: {err}");
    assert!(output.stdout.is_empty(), "{args:?}");
    // --resume keeps the paused run's own repair budget, so --max-repairs never names it (#776).
    let scope = if flag == "--max-repairs" {
        "--go or the terminal"
    } else {
        "--go, --resume or the terminal"
    };
    assert!(
        err.contains(&format!("{flag} applies only to {scope}")),
        "{args:?}: {err}"
    );
    if flag == "--max-repairs" {
        assert!(!err.contains("--resume"), "{args:?}: {err}");
    }
    assert!(!marker.exists(), "{args:?}: nothing runs");
}

#[test]
fn explicit_on_finish_with_chat_or_doctor_exits_two_naming_the_flag() {
    assert_refused(
        &["--chat", "hi", "--headless", "--on-finish", "HOOK"],
        "--on-finish",
    );
    assert_refused(&["--on-finish", "HOOK", "--doctor"], "--on-finish");
    assert_refused(&["--doctor", "--on-finish", "HOOK"], "--on-finish");
}

#[test]
fn explicit_max_repairs_with_chat_or_doctor_exits_two_naming_the_flag() {
    assert_refused(
        &["--chat", "hi", "--headless", "--max-repairs", "2"],
        "--max-repairs",
    );
    assert_refused(&["--doctor", "--max-repairs", "0"], "--max-repairs");
}

#[test]
fn both_flags_with_chat_refuse_and_the_first_named_is_on_finish() {
    assert_refused(
        &[
            "--chat",
            "hi",
            "--headless",
            "--max-repairs",
            "2",
            "--on-finish",
            "HOOK",
        ],
        "--on-finish",
    );
}

#[test]
fn the_environment_hook_alone_never_refuses_chat_or_doctor() {
    for args in [&["--doctor"][..], &["--chat", "hi", "--headless"]] {
        let (output, marker) = flag_run(args, Some("env"));
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(!err.contains("applies only to --go"), "{args:?}: {err}");
        assert!(!marker.exists());
    }
}

#[test]
fn on_finish_and_max_repairs_stay_accepted_with_go_resume_and_the_terminal() {
    for args in [
        &["--go", "x", "--headless", "--on-finish", "relative"][..],
        &["--resume", "--headless", "--on-finish", "relative"],
        &["--on-finish", "relative"],
    ] {
        let (output, _) = flag_run(args, None);
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(
            err.contains("--on-finish needs an absolute path"),
            "{args:?}: {err}"
        );
        assert!(!err.contains("applies only to"), "{args:?}: {err}");
    }
    for args in [
        &["--go", "x", "--headless", "--max-repairs", "2"][..],
        &["--go", "x", "--max-repairs", "2"],
        &["--on-finish", "HOOK", "--go", "x", "--headless"],
    ] {
        let (output, marker) = flag_run(args, None);
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(!err.contains("applies only to --go"), "{args:?}: {err}");
        assert!(!marker.exists() || args.contains(&"HOOK"));
    }
}

#[test]
fn help_and_reference_say_which_modes_on_finish_and_max_repairs_apply_to() {
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .arg("--help")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.contains("--on-finish applies only to --go, --resume and the terminal"),
        "{text}"
    );
    assert!(
        text.contains("--max-repairs applies only to --go and the terminal"),
        "{text}"
    );
    assert!(
        !text.contains("--max-repairs apply only to --go, --resume"),
        "{text}"
    );
}
