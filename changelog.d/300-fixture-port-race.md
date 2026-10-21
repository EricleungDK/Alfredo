### Fixed

- The test Ollama fixture no longer panics with `AddrInUse` (a CI flake on main) when another test or process takes the port it reserved: `serve` retries briefly, then falls back to a fresh port, and tests that need a refused-until-served port use `serve_at`, which waits for the exact port.
