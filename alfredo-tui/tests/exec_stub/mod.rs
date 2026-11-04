//! Executable stand-ins (fake `bwrap`, `ssh`, hooks) that tests write and then run.
//!
//! Writing the script with `fs::write` in the test process is racy: tests in one binary
//! run as threads, and another test that spawns a process at that moment forks while the
//! script is still open for writing. The forked child holds that descriptor until it
//! execs, and running the script meanwhile fails with ETXTBSY, "Text file busy (os error
//! 26)". Here a `cp` child writes the file, so the test process never has it open for
//! writing and no other test's fork can inherit the descriptor.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// Writes `contents` to `path` as an executable (mode 0755) with no write descriptor in
/// this process.
pub fn write_executable(path: &Path, contents: &str) {
    let name = path.file_name().unwrap().to_string_lossy();
    let staging = path.with_file_name(format!(".{name}.stub-source"));
    // The staging file is never run, so a fork inheriting its descriptor is harmless.
    fs::write(&staging, contents).unwrap();
    let status = Command::new("cp").arg(&staging).arg(path).status().unwrap();
    assert!(status.success(), "cp {} failed: {status}", path.display());
    fs::remove_file(&staging).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
