### Fixed

- Reading or grepping a file over the 4 MiB limit now says `grep can't search it either;
  use run_command (for example grep -n PATTERN FILE)` instead of the circular `use grep`.
