#!/bin/bash
# Throwaway sample repo for the README recording: generic name, generic author.
set -euo pipefail
R=${1:?usage: sample_repo.sh DIR}
rm -rf "$R"; mkdir -p "$R"; cd "$R"
git init -q -b main
git config user.name "Demo"; git config user.email "demo@example.com"
cat > textutil.py <<'P'
"""Small text helpers."""


def shout(text):
    return text.upper() + "!"
P
cat > test_textutil.py <<'P'
import unittest
from textutil import shout


class ShoutTest(unittest.TestCase):
    def test_shout(self):
        self.assertEqual(shout("hi"), "HI!")


if __name__ == "__main__":
    unittest.main()
P
git add -A; git commit -q -m "Initial commit"
