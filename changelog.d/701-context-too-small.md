### Fixed

- A worker prompt larger than `--num-ctx` now fails with a typed `context-too-small`
  code, and `/go` stops with the "raise --num-ctx (≈N tokens needed)" reason and no
  repairs instead of creating three repair tasks, each with a larger prompt, and
  advising a larger model.
