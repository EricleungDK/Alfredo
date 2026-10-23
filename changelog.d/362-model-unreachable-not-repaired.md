### Fixed
- A dropped or refused Ollama connection during a worker or repair request no longer spends the repair budget: the task gets the typed `model-unreachable` failure, autopilot stops with the endpoint named, and `Next:` says to start Ollama and `/repair ID retry` instead of advising a larger model or `--max-repairs` (#362).
