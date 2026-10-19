//! The user's completion hook: validation, environment, time limit.
use alfredo_tui::on_finish::{run, validate};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn dir(name: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("alfredo-on-finish-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn script(dir: &Path, body: &str, mode: u32) -> PathBuf {
    let path = dir.join("hook.sh");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[test]
fn validate_accepts_only_absolute_executable_regular_files() {
    let dir = dir("validate");
    let good = script(&dir, "true", 0o755);
    assert_eq!(validate(good.to_str().unwrap()).unwrap(), good);
    assert!(validate("hook.sh").unwrap_err().contains("absolute"));
    assert!(validate("").is_err());
    assert!(validate(dir.join("missing").to_str().unwrap()).is_err());
    assert!(validate(dir.to_str().unwrap())
        .unwrap_err()
        .contains("regular file"));
    let plain = script(&dir, "true", 0o644);
    assert!(validate(plain.to_str().unwrap())
        .unwrap_err()
        .contains("not executable"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn run_passes_the_environment_without_a_shell_and_reports_nonzero_exit() {
    let dir = dir("env");
    let out = dir.join("out");
    let ok = script(
        &dir,
        &format!("printf '%s' \"$ALFREDO_GOAL\" > {}", out.display()),
        0o755,
    );
    run(
        &ok,
        &[("ALFREDO_GOAL", "a b; $(x) `y`".into())],
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), "a b; $(x) `y`");
    let bad = script(&dir, "exit 7", 0o755);
    let error = run(&bad, &[], Duration::from_secs(5)).unwrap_err();
    assert!(error.contains("failed"), "{error}");
    let missing = dir.join("nope");
    assert!(run(&missing, &[], Duration::from_secs(5))
        .unwrap_err()
        .contains("cannot start"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn run_kills_a_hook_that_exceeds_the_time_limit() {
    let dir = dir("slow");
    let slow = script(&dir, "exec sleep 30", 0o755);
    let started = Instant::now();
    let error = run(&slow, &[], Duration::from_millis(300)).unwrap_err();
    assert!(error.contains("killed"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
    fs::remove_dir_all(&dir).unwrap();
}
