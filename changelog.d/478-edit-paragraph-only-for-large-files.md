### Changed

- The worker prompt now offers EDIT blocks (the EDIT paragraph and the EDIT clause in the
  restated answer line) only when an approved existing file is 16 KiB or larger, saving
  about 150 tokens per worker request on small files. EDIT answers for small files still
  parse and apply; the JSON worker format is unchanged.
