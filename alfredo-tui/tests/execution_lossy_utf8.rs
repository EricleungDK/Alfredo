//! Issue #678: command output that is not valid UTF-8 is decoded lossily (each bad
//! byte becomes U+FFFD) and never changes how the process ended. The decoded text is
//! still bounded by the request's output limit. These go through the public
//! `RustExecutionProvider` so the real sandboxed launcher decodes the output.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use alfredo_tui::execution::{
    ExecutionAuthority, ExecutionLimits, ExecutionReceipt, ExecutionRequest, ExecutionSandbox,
    RustExecutionProvider, EXECUTION_SCHEMA_VERSION,
};

const REPLACEMENT: &str = "\u{FFFD}";

/// True when bubblewrap can start a sandbox here; the tests skip otherwise.
fn sandbox_works() -> bool {
    let ok = std::process::Command::new("/usr/bin/bwrap")
        .args([
            "--unshare-user",
            "--unshare-net",
            "--ro-bind",
            "/",
            "/",
            "--",
            "/bin/true",
        ])
        .status()
        .is_ok_and(|status| status.success())
        && std::path::Path::new("/usr/bin/prlimit").is_file();
    if !ok {
        eprintln!("SKIP: bubblewrap unavailable; sandboxed execution not exercised");
    }
    ok
}

/// Run `script` under `/bin/sh -c` in the sandbox with the given output limit.
fn run(name: &str, script: &str, output_limit_bytes: usize) -> ExecutionReceipt {
    let root = std::env::temp_dir().join(format!(
        "alfredo-lossy-utf8-{}-{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("fixture root should be created");
    let root: PathBuf = fs::canonicalize(root).expect("fixture root should be canonical");
    let root_text = root.to_string_lossy().into_owned();
    let limits = ExecutionLimits {
        timeout_seconds: 5.0,
        output_limit_bytes,
        address_space_bytes: 8 * 1024 * 1024 * 1024,
        file_size_bytes: 2 * 1024 * 1024 * 1024,
        open_file_limit: 1024,
        process_count_limit: 256,
        descendant_grace_seconds: 0.1,
    };
    let mut argv: Vec<String> = [
        "/usr/bin/bwrap",
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup-try",
        "--cap-drop",
        "ALL",
        "--tmpfs",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
    ]
    .iter()
    .map(|part| (*part).to_owned())
    .collect();
    for system_root in ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"] {
        if std::path::Path::new(system_root).exists() {
            argv.extend([
                "--ro-bind".to_owned(),
                system_root.to_owned(),
                system_root.to_owned(),
            ]);
        }
    }
    argv.extend([
        "--bind".to_owned(),
        root_text.clone(),
        root_text.clone(),
        "--chdir".to_owned(),
        root_text.clone(),
        "--".to_owned(),
        "/usr/bin/prlimit".to_owned(),
        format!("--as={}", limits.address_space_bytes),
        format!("--fsize={}", limits.file_size_bytes),
        format!("--nofile={}", limits.open_file_limit),
        format!("--nproc={}", limits.process_count_limit),
        "--".to_owned(),
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        script.to_owned(),
    ]);
    let request = ExecutionRequest {
        schema_version: EXECUTION_SCHEMA_VERSION,
        request_id: format!("lossy-utf8-{name}"),
        effect: "local-agent".to_owned(),
        argv,
        working_directory: root_text.clone(),
        authority: ExecutionAuthority::LocalAgent {
            mission_id: "lossy-utf8-mission".to_owned(),
            session_id: "lossy-utf8-session".to_owned(),
            session_revision: 1,
            runner_operation_id: format!("runner:{name}"),
            worktree_identity: "managed:lossy-utf8".to_owned(),
            allowed_paths: vec!["src".to_owned()],
        },
        limits,
        sandbox: ExecutionSandbox {
            mode: "bubblewrap".to_owned(),
            readable_roots: vec![root_text.clone()],
            writable_roots: vec![root_text],
            readonly_bindings: Vec::new(),
        },
        environment: BTreeMap::new(),
        input_text: None,
        input_sha256: None,
        shell: false,
    };
    let receipt = RustExecutionProvider::new()
        .execute(&request)
        .expect("system provider should return a receipt");
    let _ = fs::remove_dir_all(root);
    receipt
}

#[test]
fn non_utf8_output_is_decoded_lossily_and_keeps_the_true_outcome() {
    if !sandbox_works() {
        return;
    }
    let receipt = run(
        "keeps-outcome",
        "printf 'caf\\351\\n'; printf 'bad \\351\\n' >&2; exit 3",
        4096,
    );
    assert_eq!(receipt.status, "failed", "{receipt:?}");
    assert_eq!(receipt.exit_code, Some(3));
    assert_eq!(receipt.stdout, "caf\u{FFFD}\n");
    assert_eq!(receipt.stderr, "bad \u{FFFD}\n");
    assert_eq!(receipt.stdout_bytes, receipt.stdout.len());
    assert!(receipt.error_code.is_empty(), "{receipt:?}");
}

#[test]
fn one_non_utf8_byte_does_not_stop_a_finished_command_completing() {
    if !sandbox_works() {
        return;
    }
    let receipt = run("completes", "printf 'caf\\351\\n'", 4096);
    assert_eq!(receipt.status, "completed", "{receipt:?}");
    assert_eq!(receipt.exit_code, Some(0));
    assert_eq!(receipt.stdout, "caf\u{FFFD}\n");
}

#[test]
fn lossy_decoding_keeps_valid_text_and_replaces_only_invalid_bytes() {
    if !sandbox_works() {
        return;
    }
    // Valid multi-byte text (h, e-acute, llo), a lone 0xFF, and a truncated
    // two-byte sequence at the very end.
    let receipt = run("valid-text", "printf 'h\\303\\251llo \\377 ok \\303'", 4096);
    assert_eq!(receipt.status, "completed", "{receipt:?}");
    assert_eq!(receipt.stdout, "h\u{e9}llo \u{FFFD} ok \u{FFFD}");
}

#[test]
fn decoded_output_is_bounded_by_the_output_limit_on_a_character_boundary() {
    if !sandbox_works() {
        return;
    }
    // 200 invalid bytes fit the captured limit but decode to 600 bytes. 512 bytes
    // holds 170 whole replacement characters (510 bytes); the cut at 512 lands inside
    // a character and must back up to 510.
    let receipt = run(
        "bounded",
        "i=0; while [ $i -lt 200 ]; do printf '\\377'; i=$((i+1)); done",
        512,
    );
    assert_eq!(receipt.status, "output-limit", "{receipt:?}");
    assert_eq!(receipt.stdout, REPLACEMENT.repeat(170), "{receipt:?}");
    assert_eq!(receipt.stdout_bytes, 510);
}

/// Shell fragment printing `count` copies of the invalid byte 0xFF to `fd`.
fn bad_bytes(count: usize, fd: u8) -> String {
    format!("i=0; while [ $i -lt {count} ]; do printf '\\377' >&{fd}; i=$((i+1)); done; ")
}

// The launcher captures at most `limit` minus a reserved message budget (256 bytes)
// of raw output, then bounds the decoded text by `limit`. 200 invalid bytes are
// 200 raw bytes (well inside the capture budget) and decode to exactly 600.

#[test]
fn decoded_output_exactly_at_the_limit_stays_completed() {
    if !sandbox_works() {
        return;
    }
    let receipt = run("at-limit", &bad_bytes(200, 1), 600);
    assert_eq!(receipt.status, "completed", "{receipt:?}");
    assert_eq!(receipt.exit_code, Some(0));
    assert_eq!(receipt.stdout, REPLACEMENT.repeat(200));
    assert!(receipt.error_code.is_empty(), "{receipt:?}");
}

#[test]
fn decoded_output_over_the_limit_is_cut_on_a_character_boundary() {
    if !sandbox_works() {
        return;
    }
    // One byte over: the cut at 599 splits the last replacement character, so it
    // backs up to the boundary at 597.
    let receipt = run("one-over", &bad_bytes(200, 1), 599);
    assert_eq!(receipt.status, "output-limit", "{receipt:?}");
    assert_eq!(receipt.stdout, REPLACEMENT.repeat(199));
    // A cut that already lands on a boundary keeps every byte up to it.
    let receipt = run("on-boundary", &bad_bytes(200, 1), 597);
    assert_eq!(receipt.status, "output-limit", "{receipt:?}");
    assert_eq!(receipt.stdout, REPLACEMENT.repeat(199));
}

#[test]
fn stdout_filling_the_limit_exactly_leaves_no_room_for_stderr() {
    if !sandbox_works() {
        return;
    }
    // stdout decodes to exactly the 600-byte limit; stderr decodes to 3 more.
    let script = format!("{}{}", bad_bytes(200, 1), bad_bytes(1, 2));
    let receipt = run("stdout-fills", &script, 600);
    assert_eq!(receipt.status, "output-limit", "{receipt:?}");
    assert_eq!(receipt.stdout, REPLACEMENT.repeat(200));
    assert_eq!(receipt.stderr, "");
}
