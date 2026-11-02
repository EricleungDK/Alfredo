### Fixed
- `/repair ID retry` after the "Ollama unreachable" autopilot stop now resumes the stopped run as the stop's `Next` row promises: the repair is approved and run, held dependents follow and the run ends on one `alfredo/go-*` branch; the repair counts against `--max-repairs`, and sandbox-refusal and file-too-large stops still stay finished (#403).
