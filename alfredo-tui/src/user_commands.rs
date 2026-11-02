//! User-defined slash commands (issue #486). STUB for the red commit.
use std::path::{Path, PathBuf};

pub const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Default)]
pub struct Scan {
    pub names: Vec<String>,
    pub refused: Vec<(String, String)>,
}
impl Scan {
    pub fn notice(&self) -> Option<String> {
        None
    }
}

pub fn directory(state_dir: &Path) -> PathBuf {
    state_dir.join("commands")
}
pub fn scan(_dir: &Path) -> Scan {
    Scan::default()
}
pub fn publish(_scan: &Scan) {}
pub fn expand(_dir: &Path, _text: &str) -> Option<Result<String, String>> {
    None
}
