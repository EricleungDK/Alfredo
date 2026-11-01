### Fixed

- A `--state-dir` or `ALFREDO_STATE_DIR` spelled with `.`, `..` or `//` (for example
  `../state`) passed `--doctor`, then every task check failed with "Check boundary does not
  name the canonical managed run" and spent the whole repair budget. The task state root is
  now cleaned lexically before it is used. A `..` that follows a symlink is refused, and
  symlinked state ancestors and state inside the workspace are still refused.
