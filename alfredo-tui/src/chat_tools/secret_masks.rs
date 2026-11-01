//! Secret masks for sandboxed runs (issue #469). The sandbox binds the Coding Workspace
//! read-write, so a rule-approved or carded `run_command` could read a secret file that
//! the read tools refuse. Before each sandboxed run, every workspace entry the secret
//! predicate ([`crate::planning_context::secret`]) matches is hidden: a secret file is
//! bound over with `/dev/null`, and a secret directory with an empty read-only directory.
//!
//! The walk is bounded and does not follow symbolic links. It includes git-ignored files,
//! which are exactly the ones a `.gitignore` leaves out of Git. Only the workspace's own
//! root `.git` is exempt: the sandbox already binds it read-only, and Git needs it. Every
//! nested `.git` (a vendored clone, a submodule's gitfile) is an ordinary secret entry and
//! is masked, because it can hold credentials in `.git/config`. When the walk cannot finish
//! (an unreadable directory, the visit or depth budget, the mask cap, a secret-named
//! symbolic link or a name that is not UTF-8), it refuses, so the caller runs nothing
//! rather than run unmasked.
use crate::planning_context;
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

/// Most masks (files and directories together) one run may carry. The masks are read-only
/// binds declared with the sandbox's other read-only bindings, which the execution
/// validator caps at 256 entries; the rest of the run takes a handful of those.
pub(crate) const LIMIT: usize = 128;
/// Entries one walk may visit before it refuses, so a huge tree cannot stall a run.
const VISIT_LIMIT: usize = 250_000;
/// Directory depth one walk may descend before it refuses.
const DEPTH_LIMIT: usize = 64;

/// The workspace entries the sandbox must not see. Paths are absolute and canonical, so
/// they match the mount destinations the sandbox is built with.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Masks {
    /// Every non-directory entry that matches the secret list: regular files, and also
    /// FIFOs, sockets and other special files. Each is bound over with `/dev/null`.
    pub(crate) files: Vec<String>,
    /// Secret directories, each bound over with the empty read-only directory of the run.
    pub(crate) dirs: Vec<String>,
}

/// The mounts one sandboxed run binds over its masks, and the host directory the directory
/// masks use. The directory is removed when this value is dropped, so it lives exactly as
/// long as the run.
pub(crate) struct Mounts {
    /// Present only when a directory is masked. Held for its drop, never read.
    _empty: Option<EmptyDir>,
    /// `(source, destination)` read-only bindings: `/dev/null` for each file, the empty
    /// directory for each directory. The sandbox binds these and declares them to the
    /// validator, so both come from this one list and cannot drift apart.
    pub(crate) pairs: Vec<(String, String)>,
}

impl Masks {
    fn len(&self) -> usize {
        self.files.len() + self.dirs.len()
    }

    /// Build the mounts for these masks, creating the empty directory when one is needed.
    pub(crate) fn mounts(&self) -> Result<Mounts, String> {
        let empty = if self.dirs.is_empty() {
            None
        } else {
            Some(EmptyDir::create()?)
        };
        let mut pairs: Vec<(String, String)> = self
            .files
            .iter()
            .map(|file| ("/dev/null".to_string(), file.clone()))
            .collect();
        if let Some(empty) = &empty {
            pairs.extend(
                self.dirs
                    .iter()
                    .map(|dir| (empty.path().to_string(), dir.clone())),
            );
        }
        Ok(Mounts {
            _empty: empty,
            pairs,
        })
    }
}

/// Walk `root` (canonical) and collect its masks, or say why they cannot be collected.
pub(crate) fn scan(root: &Path) -> Result<Masks, String> {
    scan_with(root, VISIT_LIMIT)
}

fn scan_with(root: &Path, visit_limit: usize) -> Result<Masks, String> {
    let mut scan = Scan {
        masks: Masks::default(),
        visits: 0,
        visit_limit,
    };
    scan.directory(root, "", 0)?;
    Ok(scan.masks)
}

struct Scan {
    masks: Masks,
    visits: usize,
    visit_limit: usize,
}

impl Scan {
    /// One directory: `path` on disk, `relative` its `/`-separated path in the workspace
    /// ("" for the root), `depth` how many directories down it is.
    fn directory(&mut self, path: &Path, relative: &str, depth: usize) -> Result<(), String> {
        if depth > DEPTH_LIMIT {
            return Err(format!(
                "the tree is nested deeper than {DEPTH_LIMIT} directories"
            ));
        }
        let entries = fs::read_dir(path)
            .map_err(|error| format!("{} could not be listed: {error}", shown(relative)))?;
        for entry in entries {
            let entry = entry
                .map_err(|error| format!("{} could not be listed: {error}", shown(relative)))?;
            self.visits += 1;
            if self.visits > self.visit_limit {
                return Err(format!(
                    "the workspace has more than {} entries to scan",
                    self.visit_limit
                ));
            }
            let name = entry.file_name();
            // Only the workspace's own `.git` is exempt; a nested one is masked below.
            if relative.is_empty() && name.to_str() == Some(".git") {
                continue;
            }
            let name = name.to_string_lossy().into_owned();
            let child = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            let file_type = entry
                .file_type()
                .map_err(|error| format!("{child} could not be read: {error}"))?;
            let secret = planning_context::secret(&child);
            if file_type.is_symlink() {
                // A symlink is never followed. A secret-named one cannot be masked: bwrap
                // would mount over whatever it points at, so the run is refused instead.
                if secret {
                    return Err(format!(
                        "{child} is a symbolic link named like a secret file, and a link cannot be masked safely; remove it or rename it"
                    ));
                }
                continue;
            }
            if secret {
                let text = entry.path().into_os_string().into_string().map_err(|_| {
                    format!("{child} is not valid UTF-8 and matches a secret file name")
                })?;
                if file_type.is_dir() {
                    self.masks.dirs.push(text);
                } else {
                    self.masks.files.push(text);
                }
                if self.masks.len() > LIMIT {
                    return Err(format!(
                        "the workspace has more than {LIMIT} secret files and directories to hide"
                    ));
                }
                continue;
            }
            if file_type.is_dir() {
                self.directory(&entry.path(), &child, depth + 1)?;
            }
        }
        Ok(())
    }
}

fn shown(relative: &str) -> &str {
    if relative.is_empty() {
        "the Coding Workspace"
    } else {
        relative
    }
}

/// Names of the empty directories made so far in this process, so each run's is unique.
static EMPTY_DIR_NAMES: AtomicU64 = AtomicU64::new(0);

/// An empty directory on the host, made for one sandboxed run and removed when the run
/// ends. Each secret directory is bound over with it read-only, so the sandbox sees the
/// directory with nothing in it.
#[derive(Debug)]
pub(crate) struct EmptyDir(String);

impl EmptyDir {
    fn create() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "alfredo-mask-{}-{}",
            std::process::id(),
            EMPTY_DIR_NAMES.fetch_add(1, Ordering::Relaxed)
        ));
        // `create_dir` refuses an existing path, so a name planted in advance is never reused.
        fs::create_dir(&path).map_err(|error| format!("no empty directory for masks: {error}"))?;
        // Canonical, as the sandbox validator requires of every bound path.
        let canonical = path
            .canonicalize()
            .and_then(|canonical| {
                canonical
                    .into_os_string()
                    .into_string()
                    .map_err(|_| std::io::Error::other("not UTF-8"))
            })
            .map_err(|error| {
                let _ = fs::remove_dir(&path);
                format!("no empty directory for masks: {error}")
            })?;
        Ok(Self(canonical))
    }

    fn path(&self) -> &str {
        &self.0
    }
}

impl Drop for EmptyDir {
    fn drop(&mut self) {
        // Best effort: a directory left behind is empty and holds nothing.
        let _ = fs::remove_dir(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::ffi::OsStrExt, path::PathBuf};

    /// Scratch directories for these tests; separate from the empty-directory names.
    static SCRATCH: AtomicU64 = AtomicU64::new(0);

    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "alfredo-masks-{tag}-{}-{}",
            std::process::id(),
            SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn secret_files_and_directories_are_masked_and_nothing_else() {
        let root = scratch("walk");
        fs::create_dir_all(root.join(".ssh")).unwrap();
        fs::write(root.join(".ssh/id_rsa"), "k").unwrap();
        fs::create_dir_all(root.join("sub/deeper")).unwrap();
        fs::write(root.join("sub/.env"), "x").unwrap();
        fs::write(root.join("sub/deeper/token.pem"), "x").unwrap();
        fs::write(root.join("README.md"), "hi").unwrap();
        fs::write(root.join(".gitignore"), ".env\n").unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::write(root.join("node_modules/pkg/.env"), "x").unwrap();
        let masks = scan(&root).unwrap();
        let mut files = masks.files.clone();
        files.sort();
        assert_eq!(
            files,
            vec![
                text(&root.join("node_modules/pkg/.env")),
                text(&root.join("sub/.env")),
                text(&root.join("sub/deeper/token.pem")),
            ]
        );
        assert_eq!(masks.dirs, vec![text(&root.join(".ssh"))]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn only_the_root_git_is_exempt_and_nested_git_is_masked() {
        let root = scratch("git");
        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::write(root.join(".git/config"), "[remote]\n").unwrap();
        fs::create_dir_all(root.join("nested/.git")).unwrap();
        fs::write(root.join("nested/.git/config"), "[remote]\n").unwrap();
        fs::create_dir_all(root.join("notgit/.git")).unwrap();
        fs::write(root.join("notgit/.git/secret.env"), "x").unwrap();
        fs::create_dir_all(root.join("ptr")).unwrap();
        fs::write(root.join("ptr/.git"), "gitdir: elsewhere\n").unwrap();
        let masks = scan(&root).unwrap();
        let mut dirs = masks.dirs.clone();
        dirs.sort();
        assert_eq!(
            dirs,
            vec![
                text(&root.join("nested/.git")),
                text(&root.join("notgit/.git")),
            ]
        );
        assert_eq!(masks.files, vec![text(&root.join("ptr/.git"))]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_symlink_is_never_followed_and_a_secret_named_one_is_refused() {
        let root = scratch("links");
        fs::create_dir_all(root.join("outside")).unwrap();
        fs::write(root.join("outside/.env"), "x").unwrap();
        std::os::unix::fs::symlink("outside", root.join("alias")).unwrap();
        std::os::unix::fs::symlink("README.md", root.join("link.txt")).unwrap();
        assert_eq!(
            scan(&root).unwrap().files,
            vec![text(&root.join("outside/.env"))]
        );
        std::os::unix::fs::symlink("outside", root.join(".env")).unwrap();
        let error = scan(&root).unwrap_err();
        assert!(error.contains(".env is a symbolic link"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_visit_budget_refuses_instead_of_stopping_early() {
        let root = scratch("budget");
        for index in 0..10 {
            fs::write(root.join(format!("file{index}")), "x").unwrap();
        }
        let error = scan_with(&root, 5).unwrap_err();
        assert!(error.contains("more than 5 entries"), "{error}");
        assert!(scan_with(&root, 10).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_tree_nested_past_the_depth_limit_refuses() {
        let root = scratch("depth");
        let mut deep = root.clone();
        for _ in 0..=DEPTH_LIMIT {
            deep = deep.join("d");
        }
        fs::create_dir_all(&deep).unwrap();
        let error = scan(&root).unwrap_err();
        assert!(error.contains("nested deeper"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_secret_name_that_is_not_utf8_refuses_and_a_plain_one_is_ignored() {
        let root = scratch("utf8");
        let plain = root.join(std::ffi::OsStr::from_bytes(b"\xffplain.txt"));
        fs::write(plain, "x").unwrap();
        assert_eq!(scan(&root).unwrap(), Masks::default());
        let secret = root.join(std::ffi::OsStr::from_bytes(b".env\xff"));
        fs::write(secret, "x").unwrap();
        let error = scan(&root).unwrap_err();
        assert!(error.contains("not valid UTF-8"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn files_bind_dev_null_and_directories_bind_the_empty_directory() {
        let root = scratch("mounts");
        fs::create_dir_all(root.join(".ssh")).unwrap();
        fs::write(root.join(".env"), "x").unwrap();
        let masks = scan(&root).unwrap();
        let mounts = masks.mounts().unwrap();
        let empty = mounts
            .pairs
            .iter()
            .find(|(_, dest)| dest.ends_with("/.ssh"))
            .unwrap();
        assert_ne!(empty.0, "/dev/null");
        assert!(Path::new(&empty.0).is_dir());
        assert_eq!(
            mounts
                .pairs
                .iter()
                .find(|(_, dest)| dest.ends_with("/.env")),
            Some(&("/dev/null".to_string(), text(&root.join(".env"))))
        );
        assert_eq!(mounts.pairs.len(), 2);
        let empty_path = PathBuf::from(&empty.0);
        drop(mounts);
        assert!(!empty_path.exists(), "the empty directory outlived its run");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn no_directory_is_made_when_only_files_are_masked() {
        let root = scratch("no-dir");
        fs::write(root.join(".env"), "x").unwrap();
        let mounts = scan(&root).unwrap().mounts().unwrap();
        assert!(mounts._empty.is_none());
        assert_eq!(mounts.pairs.len(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
