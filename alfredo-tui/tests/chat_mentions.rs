use alfredo_tui::chat_mentions::attach;
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn repo() -> tempdir::Dir {
    let dir = tempdir::Dir::new();
    let root = dir.path();
    git(root, &["init", "-q", "--template="]);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("calc.py"), "def answer():\n    return 42\n").unwrap();
    fs::write(root.join("src/a.rs"), "fn a() {}\n").unwrap();
    fs::write(root.join(".env"), "SECRET=1\n").unwrap();
    fs::write(root.join("big.txt"), "z".repeat(100 * 1024)).unwrap();
    fs::write(root.join("blob.bin"), b"\0\x01").unwrap();
    std::os::unix::fs::symlink("/etc/passwd", root.join("link.txt")).unwrap();
    git(root, &["add", "-f", "."]);
    git(root, &["commit", "-q", "-m", "init"]);
    dir
}

mod tempdir {
    use std::path::{Path, PathBuf};
    pub struct Dir(PathBuf);
    impl Dir {
        pub fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "alfredo-mentions-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[tokio::test]
async fn committed_head_content_is_attached_not_the_working_tree() {
    let dir = repo();
    fs::write(dir.path().join("calc.py"), "EDITED = 1\n").unwrap();
    fs::write(dir.path().join("new.py"), "untracked\n").unwrap();
    let out = attach(dir.path(), "Look at @calc.py and @src/a.rs").await;
    assert!(out
        .block
        .contains("FILE calc.py\ndef answer():\n    return 42\n"));
    assert!(out.block.contains("FILE src/a.rs\nfn a() {}"));
    assert!(!out.block.contains("EDITED"));
    assert!(out.notices.is_empty(), "{:?}", out.notices);
    let out = attach(dir.path(), "see @new.py").await;
    assert!(out.block.is_empty());
    assert_eq!(
        out.notices,
        ["@new.py: not attached (not committed at HEAD)"]
    );
}

#[tokio::test]
async fn unsafe_and_unreadable_paths_are_refused_with_reasons() {
    let dir = repo();
    let out = attach(
        dir.path(),
        "@/etc/passwd @../x.txt @src/../calc.py @.env @link.txt @src/ @blob.bin @big.txt",
    )
    .await;
    assert!(
        out.block.contains("FILE big.txt"),
        "oversized files are cut, not refused"
    );
    assert!(out.block.contains("[truncated: 32768 of 102400 bytes]"));
    for fragment in [
        "@/etc/passwd: not attached (rejected",
        "@../x.txt: not attached (rejected",
        "@src/../calc.py: not attached (rejected",
        "@.env: not attached (excluded",
        "@link.txt: not attached (not a regular file",
        "@blob.bin: not attached (binary)",
    ] {
        assert!(
            out.notices.iter().any(|n| n.starts_with(fragment)),
            "{fragment}: {:?}",
            out.notices
        );
    }
    assert!(!out.block.contains("root:") && !out.block.contains("SECRET"));
}

#[tokio::test]
async fn non_mentions_attach_nothing_and_say_nothing() {
    let dir = repo();
    let out = attach(dir.path(), "mail a@b.com, ping @everyone, @wayfinder map").await;
    assert!(out.block.is_empty() && out.notices.is_empty());
}

#[tokio::test]
async fn no_head_commit_gives_one_notice() {
    let dir = tempdir::Dir::new();
    git(dir.path(), &["init", "-q", "--template="]);
    let out = attach(dir.path(), "@calc.py").await;
    assert!(out.block.is_empty());
    assert_eq!(out.notices.len(), 1);
}
