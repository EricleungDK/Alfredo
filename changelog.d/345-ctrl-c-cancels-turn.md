### Changed

- Ctrl+C during a running chat turn now cancels the turn like Esc (partial reply, prompt
  draft and session kept) instead of quitting; when idle, or while workers run, it still
  quits like Ctrl+Q. The F1 help, README and reference describe it. Headless SIGINT is
  unchanged.
