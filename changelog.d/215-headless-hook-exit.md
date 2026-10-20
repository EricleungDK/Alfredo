### Fixed

- `--headless` now exits 2 with `Error: …` on stderr and nothing on stdout when Ollama is unreachable or the model is not installed (it exited 1 like a failed run), prints a failed run's reason on its stderr line (`failed 0/0: <reason>`), and after printing the JSON waits for the `--on-finish` hook (10 s limit, then killed and reaped) and reports a spawn failure, non-zero exit or timeout on stderr instead of orphaning it.
