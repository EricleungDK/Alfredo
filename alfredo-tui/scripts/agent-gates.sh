#!/usr/bin/env bash
# Run the same gates as CI (.github/workflows/rust-terminal.yml) and print one summary table.
# Usage: alfredo-tui/scripts/agent-gates.sh [--quick] [--retry-flaky] [--json FILE]
#   --quick        fmt, clippy, cargo test, Python tests and audit only (no smokes)
#   --retry-flaky  re-run failing Rust test binaries once; a pass on retry is reported FLAKY
#   --json FILE    also write the results as JSON
# Exit status: 0 when every gate passed (FLAKY counts as passed), 1 otherwise.
# Source agent-env.sh first so bubblewrap, cargo-audit and the cargo env are in place.
set -u
ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"; cd "$ROOT"
QUICK=0; RETRY=0; JSON=""
while [ $# -gt 0 ]; do case "$1" in
  --quick) QUICK=1;; --retry-flaky) RETRY=1;; --json) JSON="$2"; shift;;
  *) echo "unknown option $1" >&2; exit 2;; esac; shift; done
LOG="${TMPDIR:-/tmp}/agent-gates.$$"; mkdir -p "$LOG"
names=(); results=(); notes=()
gate() { # gate NAME CMD...
  local name="$1"; shift; local f="$LOG/$(echo "$name" | tr -c 'a-zA-Z0-9' _).log"
  if "$@" >"$f" 2>&1; then names+=("$name"); results+=(PASS); notes+=("")
  else names+=("$name"); results+=(FAIL); notes+=("$(grep -E 'error|FAILED|panicked|failed' "$f" | head -3 | tr '\n' ' ' | cut -c1-200) (log: $f)"); fi
}
M=alfredo-tui/Cargo.toml
gate "fmt"     cargo fmt --manifest-path $M -- --check
gate "clippy"  cargo clippy --locked --manifest-path $M --all-targets -- -D warnings
gate "cargo test" cargo test --locked --no-fail-fast --manifest-path $M
if [ "$RETRY" = 1 ] && [ "${results[2]}" = FAIL ]; then
  failing=$(grep -E '^test .* \.\.\. FAILED$' "$LOG/cargo_test.log" | awk '{print $2}' | sort -u)
  if [ -n "$failing" ] && cargo test --locked --no-fail-fast --manifest-path $M -- --exact $failing >"$LOG/retry.log" 2>&1; then
    results[2]=FLAKY; notes[2]="passed on retry: $(echo $failing | cut -c1-200)"
  fi
fi
gate "python tests" python3 -m unittest discover -s alfredo-tui/tests -p 'test_*.py'
if [ "$QUICK" = 0 ]; then
  gate "build debug" cargo build --locked --manifest-path $M
  # Smokes: exactly the list CI runs, read from the workflow so it never drifts.
  for s in $(grep -oE 'python3 alfredo-tui/tests/[a-z_]+_smoke\.py' .github/workflows/rust-terminal.yml | awk '{print $2}' | grep -vE 'audit_smoke|release_smoke'); do
    gate "$(basename "$s" .py)" python3 "$s"
  done
fi
gate "cargo audit" bash -c 'cd alfredo-tui && cargo audit --file Cargo.lock --deny warnings'
[ "$QUICK" = 0 ] && gate "audit_smoke" python3 alfredo-tui/tests/audit_smoke.py
echo; echo "| Gate | Result | Note |"; echo "|---|---|---|"
fail=0
for i in "${!names[@]}"; do echo "| ${names[$i]} | ${results[$i]} | ${notes[$i]} |"; [ "${results[$i]}" = FAIL ] && fail=1; done
if grep -qsE 'bubblewrap|Bubblewrap executable' "$LOG"/cargo_test.log 2>/dev/null; then
  echo; echo "Note: some failures mention bubblewrap — environmental unless bwrap is installed (source agent-env.sh)."; fi
if [ -n "$JSON" ]; then
  python3 - "$JSON" "${#names[@]}" "${names[@]}" "${results[@]}" <<'PY'
import json,sys
out,n=sys.argv[1],int(sys.argv[2]); a=sys.argv[3:]
json.dump([{"gate":a[i],"result":a[n+i]} for i in range(n)],open(out,"w"),indent=1)
PY
fi
exit $fail
