### Fixed

- Command-history eviction no longer takes the command a pending plan draft names as its
  origin when that command is `Unknown`, so every later conversation save keeps working
  instead of refusing with "Planner draft origin requires its saved command".
