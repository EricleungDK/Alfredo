### Fixed

- The M15 code-health clippy parser now subtracts the `(N duplicate[s])` suffix from a
  target's `generated N warnings` summary, so duplicate warnings no longer make
  `--code-health` report the pedantic count as null; a real mismatch still errors.
