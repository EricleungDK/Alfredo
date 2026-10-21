### Fixed

- `read_file` now shows a line longer than the output budget cut at a character boundary
  with a `[line N is M bytes; showed the first K]` marker, instead of returning an empty
  result whose continuation hint repeated itself.
