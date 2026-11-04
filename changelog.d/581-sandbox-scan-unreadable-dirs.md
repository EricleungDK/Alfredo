### Fixed

- A sandboxed `run_command` is no longer refused in a workspace with a directory it cannot
  list (a root-owned or mode-0700 Docker volume) or with more than 250,000 entries. An
  unreadable non-secret directory is bound over with the empty read-only directory, so it
  looks empty and counts toward the 128-mask cap; the entry limit is now 1,000,000, and its
  refusal names the top-level directory with the most entries. Secret-named symbolic links,
  more than 128 masks, non-UTF-8 secret names and nesting past 64 directories still refuse.
