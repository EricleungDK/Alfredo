### Fixed

- The headless chat docs (`docs/reference.md`, `README.md`) now document `--chat -`: the
  prompt is read from stdin to EOF and must be nonempty UTF-8 of at most 256 KiB (else exit 2),
  and a flag or nothing right after `--chat` is `--chat needs a PROMPT`. A doc test guards it.
