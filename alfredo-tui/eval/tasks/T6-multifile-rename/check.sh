#!/bin/sh
python3 -B - <<'PY'
import pathlib

import util
from main import report

assert not hasattr(util, "fetch_data"), "old name still defined"
assert util.load_records() == [1, 2, 3]
assert report() == "3 items, total 6"
for name in ("util.py", "a.py", "b.py", "main.py"):
    assert "fetch_data" not in pathlib.Path(name).read_text(), f"fetch_data left in {name}"
PY
