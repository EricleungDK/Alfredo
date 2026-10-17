### Fixed

- The default model is now `qwen2.5-coder:14b`, the model the README and `INSTALL.md`
  tell you to pull (it was `qwen3:14b`). A selected model missing from the server's
  catalog turns the header amber (`model X not installed · /models or ollama pull X`)
  instead of showing a plain green check. A 404 for a missing model names the model,
  keeps Ollama's own text and suggests `/models` / `ollama pull`; `/go` no longer
  retries it three times.
