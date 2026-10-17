### Fixed

- A FILE block for a Markdown file that opens and closes with a code fence (and has
  other fences inside) is no longer corrupted: only a single wrapping fence is stripped.
