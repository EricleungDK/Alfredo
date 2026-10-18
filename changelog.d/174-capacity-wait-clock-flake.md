### Fixed

- The capacity-wait layout test no longer compares two draws of a wall-clock elapsed
  label, so `cargo test --locked` stops failing intermittently when the draws straddle a
  100 ms boundary.
