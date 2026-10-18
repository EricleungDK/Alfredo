#!/bin/sh
python3 -B - <<'PY'
from names import greeting, initials

assert initials("Ada Lovelace") == "AL"
assert initials("grace brewster murray hopper") == "GBMH"
assert initials("Alan") == "A"
assert greeting("Bo") == "Hello, Bo!"
PY
