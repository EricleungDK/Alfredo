//! Issue #689: an approval card for `write_file` or `edit_file` must not block on, or read
//! into memory, the file it targets. The fingerprint taken for the card and the one `apply`
//! checks again both open the target through the bounded, confined reader, so a FIFO is
//! refused at once and a file over `edit::MAX_FILE` is refused without being read.
#![cfg(unix)]

use alfredo_tui::{chat_tools::approval, model::ToolCall};
use serde_json::json;
use std::{
    ffi::CString,
    fs,
    os::unix::{ffi::OsStrExt, fs::FileTypeExt},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

/// A fresh, empty workspace named after `tag`; removed when the guard drops.
struct Workspace(PathBuf);

impl Workspace {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("alfredo-edit-target-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn mkfifo(path: &Path) {
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(
        unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) },
        0,
        "mkfifo failed"
    );
}

/// Opens a FIFO for writing without blocking. A reader stuck in a blocking open (the bug
/// under test) then returns, so a leaked test thread does not outlive its test. Does nothing
/// when no reader waits.
fn release_fifo(path: &Path) {
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
    if fd >= 0 {
        unsafe { libc::close(fd) };
    }
}

fn is_fifo(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_fifo())
}

#[cfg(target_os = "linux")]
fn proc_status_kb(key: &str) -> u64 {
    fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with(key))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}

#[cfg(target_os = "linux")]
fn vm_hwm_kb() -> u64 {
    proc_status_kb("VmHWM:")
}

#[cfg(target_os = "linux")]
fn vm_rss_kb() -> u64 {
    proc_status_kb("VmRSS:")
}

fn edit_call(path: &str) -> ToolCall {
    ToolCall {
        name: "edit_file".into(),
        arguments: json!({"path": path, "old_text": "a", "new_text": "b"}),
    }
}

fn write_call(path: &str) -> ToolCall {
    ToolCall {
        name: "write_file".into(),
        arguments: json!({"path": path, "content": "x\n"}),
    }
}

#[test]
fn an_edit_card_for_a_fifo_target_is_refused_promptly() {
    let ws = Workspace::new("fifo");
    // Each tool gets its own FIFO, and every outcome is read before any assertion, so a
    // blocked call for one tool cannot hide the other.
    let mut outcomes = Vec::new();
    for (tool, call) in [
        ("edit_file", edit_call("p-edit.txt")),
        ("write_file", write_call("p-write.txt")),
    ] {
        let fifo = ws.0.join(call.arguments["path"].as_str().unwrap());
        mkfifo(&fifo);
        let (tx, rx) = mpsc::channel();
        let root = ws.0.clone();
        std::thread::spawn(move || {
            let _ = tx.send(approval::card(&root, &call).map(|_| ()));
        });
        let got = rx.recv_timeout(Duration::from_secs(5));
        release_fifo(&fifo);
        outcomes.push((tool, got));
    }
    let blocked: Vec<String> = outcomes
        .iter()
        .filter(|(_, got)| !matches!(got, Ok(Err(e)) if e.contains("not a regular file")))
        .map(|(tool, got)| {
            format!("{tool} on a FIFO: {got:?} (want Err naming \"not a regular file\")")
        })
        .collect();
    assert!(blocked.is_empty(), "{}", blocked.join("; "));
}

#[cfg(target_os = "linux")]
#[test]
fn an_edit_card_for_an_oversized_target_does_not_read_it_into_memory() {
    let ws = Workspace::new("big");
    fs::File::create(ws.0.join("big.txt"))
        .unwrap()
        .set_len(1 << 30)
        .unwrap(); // 1 GiB, sparse
                   // VmHWM is the process peak and only rises, so reset it first (Linux >= 4.0): the
                   // growth measured below is then this call's alone, whatever earlier tests allocated.
    fs::write("/proc/self/clear_refs", "5").expect("reset the peak RSS counter");
    let before = vm_hwm_kb();
    // The reset must have brought the peak down to the current RSS, or the growth below
    // would measure nothing.
    assert!(
        before <= vm_rss_kb() + 1024,
        "peak RSS was not reset: {before} kB"
    );
    let got = approval::card(&ws.0, &edit_call("big.txt")).map(|_| ());
    let grew_mb = vm_hwm_kb().saturating_sub(before) / 1024;
    let error = got.expect_err("an oversized edit target was accepted");
    assert!(
        error.contains("over the 1048576-byte edit limit"),
        "{error}"
    );
    assert!(
        grew_mb < 64,
        "peak RSS grew {grew_mb} MiB for a 1 GiB target"
    );
}

#[test]
fn an_approved_edit_whose_target_became_a_fifo_is_refused_promptly() {
    let ws = Workspace::new("swap");
    let target = ws.0.join("t.txt");
    fs::write(&target, "a\n").unwrap();
    let card = approval::card(&ws.0, &edit_call("t.txt")).expect("card for a regular file");
    fs::remove_file(&target).unwrap();
    mkfifo(&target);
    let (tx, rx) = mpsc::channel();
    let root = ws.0.clone();
    std::thread::spawn(move || {
        let _ = tx.send(approval::apply(&root, &card));
    });
    let got = rx.recv_timeout(Duration::from_secs(5));
    // Nothing was written: the swapped-in FIFO is still there, not replaced by a file.
    let still_fifo = is_fifo(&target);
    release_fifo(&target);
    assert!(
        matches!(&got, Ok(Err(e)) if e.contains("not a regular file")),
        "apply on a FIFO: {got:?} (want Err naming \"not a regular file\")"
    );
    assert!(still_fifo, "apply replaced the FIFO");
}

#[test]
fn a_missing_write_target_still_fingerprints_as_absent() {
    let ws = Workspace::new("absent");
    let card = approval::card(&ws.0, &write_call("new.txt")).expect("card for a new file");
    assert!(
        card.fingerprint.ends_with(":absent"),
        "{}",
        card.fingerprint
    );
    let card = approval::card(&ws.0, &write_call("deep/new.txt")).expect("card under new dirs");
    assert!(
        card.fingerprint.contains(":absent+dirs:"),
        "{}",
        card.fingerprint
    );
}
