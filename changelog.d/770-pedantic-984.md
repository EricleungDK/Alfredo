### Changed

- Internal: `explicit_flag_refusal` takes a grouped `ExplicitFlags` instead of four bools and
  `Completion::f1` is `#[must_use]`, clearing two clippy pedantic warnings. No behaviour change.
