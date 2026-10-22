# Flaky protocol

1. **Quantify**: run the single test alone N× (N ≥ 10, or 20 for rare flakes), then once in the full suite. Record the pass rate: `k/N`.
2. **Find the cause**: ordering, shared state, real time, network, port reuse, unawaited async, test parallelism. Under load (`--parallel`, `stress`) if a rate is 0 alone but nonzero in the suite.
3. **Fix the cause**, red first where possible (a test that forces the bad ordering or timing).
4. **Re-quantify** with the same N: report `before k/N → after k/N`.

Keep timeouts and retry counts where they are; a fix that only raises a global timeout, adds retries, or skips the test is a mask, so report it as one and get user sign-off.
