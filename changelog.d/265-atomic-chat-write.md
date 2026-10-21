### Fixed

- Approved `write_file`/`edit_file` now write atomically (synced temp file in the same directory, original permission bits, rename over the target): a failed write leaves the original intact, and a hardlinked file no longer changes its other link.
