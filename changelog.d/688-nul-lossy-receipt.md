### Fixed

- A command whose output contains a NUL byte (for example `printf 'a\0b'`) no longer
  fails with "execution receipt output is invalid". The NUL is replaced with U+FFFD in
  the receipt and the true status and exit code are kept, like other undecodable output.
