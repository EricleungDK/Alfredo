### Fixed

- `--chat - --headless` now reads the prompt from stdin (UTF-8, at most 256 KiB) instead of
  sending a literal `-` to the model; empty stdin, oversized input or invalid UTF-8 exits 2
  with a message naming the problem.
- `--chat` followed by another flag (or nothing) now reports `--chat needs a PROMPT`
  instead of consuming the flag and complaining that `--chat needs --headless`.
