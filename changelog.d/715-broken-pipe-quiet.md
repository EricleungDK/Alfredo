### Fixed

- `--version`, `--help` and `--doctor` no longer print a Rust panic (`failed printing to
  stdout: Broken pipe`) when the reader closes early, as in `alfredo-tui --help | head`
  or `--doctor | head -2`. They exit quietly with status 0.
