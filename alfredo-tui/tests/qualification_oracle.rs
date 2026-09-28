use alfredo_tui::{inference_profile::digest, qualification_oracle::check_script};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

const VALID: &str = "import re\nprint('Benign import output remains allowed')\ndef parsed(text):\n    if not isinstance(text, str) or re.fullmatch(r'[0-9]+', text) is None:\n        return None\n    value = int(text)\n    return value if 1 <= value <= 65535 else None\ndef parse_port(text):\n    return parsed(text)\n";
static IDS: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-oracle-{}-{}",
            std::process::id(),
            IDS.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn run(&self, code: &str, required: bool, pinned: &BTreeMap<String, String>) -> Output {
        fs::write(self.0.join("solution.py"), code).unwrap();
        fs::write(
            self.0.join("check_fixture.py"),
            check_script(required, pinned),
        )
        .unwrap();
        Command::new("/usr/bin/python3")
            .args(["-I", "-B", "check_fixture.py"])
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn accepted(output: &Output) -> bool {
    output.status.success() && output.stdout == b"QUALIFICATION_CHECK_OK\n"
}

#[test]
fn independent_parent_accepts_helpers_imports_and_rejects_wrong_typed_values() {
    let fixture = Fixture::new();
    let valid = fixture.run(VALID, false, &BTreeMap::new());
    assert!(
        accepted(&valid),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    // Python bool equality with integer 1 must not satisfy the port return contract.
    let boolean = VALID.replace(
        "return parsed(text)",
        "return True if text == '1' else parsed(text)",
    );
    assert!(!accepted(&fixture.run(&boolean, false, &BTreeMap::new())));
    let non_string = VALID.replace(
        "return parsed(text)",
        "return 80 if type(text) is int else parsed(text)",
    );
    assert!(!accepted(&fixture.run(
        &non_string,
        false,
        &BTreeMap::new()
    )));
}

#[test]
fn model_early_exit_cannot_forge_the_parent_completion_marker() {
    let fixture = Fixture::new();
    for source in [
        "print('QUALIFICATION_CHECK_OK')\nraise SystemExit(0)\n",
        "import os\nos.write(1, b'QUALIFICATION_CHECK_OK\\n')\nos._exit(0)\n",
        "import sys\nsys.stdout.write('QUALIFICATION_CHECK_OK\\n')\nsys.exit(0)\n",
    ] {
        let output = fixture.run(source, false, &BTreeMap::new());
        assert!(!output.status.success());
        assert!(!accepted(&output));
    }
}

#[test]
fn bounded_child_output_and_parent_pinned_sources_are_enforced() {
    let fixture = Fixture::new();
    let flooded = format!("print('x' * 100_000)\n{VALID}");
    assert!(!accepted(&fixture.run(&flooded, false, &BTreeMap::new())));
    let references = [
        ("reference_left.py".to_string(), "# RATE = 17\n"),
        ("reference_right.py".to_string(), "# OFFSET = 23\n"),
    ];
    let mut pinned = BTreeMap::new();
    for (path, content) in references {
        fs::write(fixture.0.join(&path), content).unwrap();
        pinned.insert(path, digest(content.as_bytes()));
    }
    let valid = "def transform(value):\n    return value * 17 + 23\n";
    let output = fixture.run(valid, true, &pinned);
    assert!(
        accepted(&output),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mutation = format!(
        "from pathlib import Path\nPath('reference_left.py').write_text('changed')\n{valid}"
    );
    assert!(!accepted(&fixture.run(&mutation, true, &pinned)));
}
