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
    for text in ["--keep-alive", "--connect-retries", "ALFREDO_KEEP_ALIVE"] {
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
