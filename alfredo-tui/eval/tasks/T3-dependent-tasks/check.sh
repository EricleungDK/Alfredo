#!/bin/sh
python3 -B - <<'PY'
import pathlib

from invoice import total
from pricing import with_tax

assert with_tax(100) == 125.0
assert with_tax(0.1) == 0.12
assert total([100, 20]) == 150.0
assert total([]) == 0
assert "with_tax" in pathlib.Path("invoice.py").read_text()
PY
