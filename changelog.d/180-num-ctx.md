### Added

- `--num-ctx N` (env `ALFREDO_NUM_CTX`, 2048-262144 tokens) sends Ollama `num_ctx` on every
  chat, planner, worker, repair and preload request. A request whose estimated prompt exceeds
  the window is refused before sending instead of being truncated silently, the chat shows
  `ctx used/N` after each reply (with a warning from 95%), and `--doctor` prints a
  `NOTE context:` line. Unset behaviour is unchanged.
