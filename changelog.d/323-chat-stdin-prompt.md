### Added

- `--chat - --headless` reads the Chat Agent prompt from stdin to EOF (issue #323; UTF-8,
  cap 256 KiB, whitespace kept as given). Empty or whitespace-only stdin, stdin past the cap
  and a missing prompt (`--chat` last, or followed by a known flag such as `--headless`) are
  usage errors that exit 2 and print nothing on stdout, instead of sending a literal `-` or
  the next flag to the model.
