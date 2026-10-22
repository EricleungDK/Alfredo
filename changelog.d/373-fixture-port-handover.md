### Fixed

- The test fixture's `serve_at_waits_for_a_port_that_is_released_shortly` self-test no longer
  fails intermittently with `AddrInUse`: it holds an ephemeral port from the start instead of
  releasing and re-binding one that a parallel test could take.
