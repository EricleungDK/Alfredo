### Added

- `alfredo-tui/eval/` is a reproducible harness eval: six fixed tasks (including a
  repair task, dependent tasks, a multi-file rename and a negative control), each graded
  by `check.sh` on a `git archive` of the result. `run_eval.py --agent alfredo|claude-code
  --backend fixture|ollama --trials N` prints schema-valid JSON with per-trial pass,
  turns, tool calls, tokens, errors, repairs, wall time, RSS and aggregate metrics
  M1-M16 (`null` with a reason where a mode cannot produce one). Fixture mode runs in CI.
