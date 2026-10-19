//! The user's own completion hook (`--on-finish PROGRAM`, env `ALFREDO_ON_FINISH`).
//!
//! One absolute path to an executable the user chose at launch; never read from
//! repository files. It runs without a shell, outside the sandbox, with the
//! parent environment plus `ALFREDO_*` variables describing the finished run, a
//! 10 second limit and discarded output. Failure only produces a footer notice.

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, OnceLock},
    time::{Duration, Instant},
};

/// Longest the hook may run before it is killed.
pub const TIMEOUT: Duration = Duration::from_secs(10);

static CONFIGURED: OnceLock<PathBuf> = OnceLock::new();

/// The validated hook chosen at startup, if any.
pub fn configured() -> Option<PathBuf> {
    CONFIGURED.get().cloned()
}

/// Record the startup hook (first call wins).
pub fn configure(path: PathBuf) {
    let _ = CONFIGURED.set(path);
}

/// An absolute path to an existing executable regular file.
pub fn validate(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if value.is_empty() || !path.is_absolute() {
        return Err("--on-finish needs an absolute path to an executable program".into());
    }
    let meta = std::fs::metadata(&path)
        .map_err(|error| format!("--on-finish {value}: cannot read it: {error}"))?;
    if !meta.is_file() {
        return Err(format!("--on-finish {value}: not a regular file"));
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(format!("--on-finish {value}: not executable"));
    }
    Ok(path)
}

/// Run `program` with `env` added to the inherited environment; blocks up to
/// `timeout`, then kills and reaps the child. Output is discarded.
pub fn run(program: &Path, env: &[(&str, String)], timeout: Duration) -> Result<(), String> {
    let mut child = Command::new(program)
        .envs(env.iter().map(|(key, value)| (*key, value.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start: {error}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("failed ({status})")),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("killed after {} s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cannot wait: {error}"));
            }
        }
    }
}

/// Run the hook on its own thread; the receiver yields a footer notice only when
/// it failed.
pub fn spawn(program: PathBuf, env: Vec<(&'static str, String)>) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        if let Err(error) = run(&program, &env, TIMEOUT) {
            let _ = sender.send(format!("--on-finish hook {error}"));
        }
    });
    receiver
}

/// Environment values may not contain NUL.
pub fn env_value(value: &str) -> String {
    value.replace('\0', "")
}
