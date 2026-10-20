### Added

- `python3 alfredo-tui/eval/run_eval.py --agent alfredo --code-health` computes the scoreboard's M15 code-health numbers (clippy pedantic count, non-test unwrap/expect, largest non-test module, Rust and Python test counts) mechanically, so the figure no longer depends on which grep was used. Without the flag M15 stays `null`.
