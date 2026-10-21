#!/bin/sh
# Deterministic grader: run from the root of the exported candidate tree.
python3 -B - <<'PY'
from units import km_to_miles, miles_to_km

assert abs(km_to_miles(1.609344) - 1) < 1e-9
assert abs(km_to_miles(16.09344) - 10) < 1e-9
assert km_to_miles(0) == 0
assert abs(miles_to_km(1) - 1.609344) < 1e-9
assert abs(miles_to_km(10) - 16.09344) < 1e-9
PY
