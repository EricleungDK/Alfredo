### Fixed

- Chat `write_file` and `edit_file` no longer overwrite a read-only file. Since the atomic
  write, a file without the owner write bit (for example `chmod 444`) was replaced anyway;
  it is now refused with `<path> is read-only; …` before any approval card, and left unchanged.
