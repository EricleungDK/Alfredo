#!/bin/sh
python3 -B - <<'PY'
from parity import is_even, is_positive

assert is_even(4) is True
assert is_even(7) is False
assert is_even(0) is True
assert is_positive(3) is True
PY
