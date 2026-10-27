### Fixed

- The Wayfinder chat terminal smoke test waits for the whole tools-off footer before
  asserting on it, so a partial redraw no longer fails main CI.
