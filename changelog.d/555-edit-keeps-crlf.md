### Fixed

- An EDIT block on a file that uses CRLF line endings throughout now keeps them, so a
  one-line edit is a one-line diff instead of a rewrite of every line. Files with LF or
  mixed endings behave as before.
