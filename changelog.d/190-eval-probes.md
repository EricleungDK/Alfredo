### Added

- `alfredo-tui/eval/run_eval.py --agent alfredo --probes` now fills the scoreboard rows that were
  `null`: M8 (chat reply-to-screen and Enter-to-request over 20 chats, from a marker-replying fake
  server and raw pty bytes), M9 (keystroke echo p50/p95 over 100 keys), M10 (the parsed
  `rendering_cost` release bench; `--no-render-probe` skips its slow build) and M14 (kill -9 during
  generation, restart paused, `/resume` accepts, 3 trials). Probes are opt-in, use and record the
  binary profile, and `schema.json` accepts the new `secondary` values and `config.probes`.
