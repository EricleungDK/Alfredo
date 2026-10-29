### Fixed

- `@path` completion no longer runs on the key path: the lookup moves to a worker thread,
  so keys echo at once in large repositories (10k files were 66-84 ms per key after `@`)
  and a list for an older draft is never shown. The listing is kept for the keystrokes of
  one `@` word, and completion now reaches files past the 5000th (up to 100000), so
  `@d99/file7` lists in a 30k-file repository.
