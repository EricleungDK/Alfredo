### Changed

- A `write_file` or `edit_file` approval card now shows one `path · N bytes` row, and drops the `args` excerpt when the card would otherwise be too tall, so a short change to a long file (3 contiguous lines on a 24x80 terminal) can be approved with `y` instead of going deny-only. The whole diff must still fit; `run_command` and git cards keep their `args` row (#586).
