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
fn piped_launch_reports_how_to_get_usage_without_terminal_escapes() {
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--endpoint", "http://127.0.0.1:1", "--model", "fixture"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Interactive terminal required"));
    assert!(output.stdout.is_empty());
}
