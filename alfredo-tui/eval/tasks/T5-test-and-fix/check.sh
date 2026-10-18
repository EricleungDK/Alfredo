#!/bin/sh
python3 -B - <<'PY'
import pathlib
import re
import subprocess
import sys

from slug import slugify

assert slugify("Hello, World!") == "hello-world"
assert slugify("  Already  spaced  ") == "already-spaced"
assert slugify("a--b") == "a-b"
assert slugify("Rust 2024") == "rust-2024"

test = pathlib.Path("test_slug.py")
assert test.is_file(), "test_slug.py is missing"
assert len(re.findall(r"^\s*def test_", test.read_text(), re.M)) >= 2, "need at least two tests"
run = subprocess.run([sys.executable, "-B", "-m", "unittest", "test_slug"], capture_output=True, text=True)
assert run.returncode == 0, run.stderr
PY
