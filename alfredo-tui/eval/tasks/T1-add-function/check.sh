#!/bin/sh
# Deterministic grader: run from the root of the exported candidate tree.
python3 -B - <<'PY'
from mathx import clamp, double

assert clamp(5, 0, 3) == 3
assert clamp(-5, 0, 3) == 0
assert clamp(2, 0, 3) == 2
assert clamp(0, 0, 3) == 0 and clamp(3, 0, 3) == 3
assert double(4) == 8
PY
