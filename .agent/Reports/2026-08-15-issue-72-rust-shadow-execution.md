# Issue #72 — Shadow Rust execution receipts against Python

**Date:** 2026-08-15
**Status:** Locally complete; trusted Linux source and hardened packaged-provider release gates pass, while Rust remains a non-authoritative shadow pending human acceptance and downstream cutover issues
**Last verified:** 2026-08-30
**Source:** [GitHub Issue #72](https://github.com/EricleungDK/Alfredo/issues/72)
**Parent:** [PRD #56](https://github.com/EricleungDK/Alfredo/issues/56)
**Depends on:** [Issue #71](https://github.com/EricleungDK/Alfredo/issues/71), [Issue #65](https://github.com/EricleungDK/Alfredo/issues/65)

## Outcome

Issue #72 now has a production-shaped Rust host-effects candidate behind the Issue #71 request/receipt boundary. Python remains the authorization, persistence, replay, reconciliation, and canonical-write authority. The candidate is invoked only by an explicit Python shadow adapter; it is not wired into the Mission coordinator or any product cutover path.

The Rust provider validates the versioned `ExecutionRequest`, requires a trusted prepared Bubblewrap argv with an exact namespace/mount/resource shape, launches with a cleared environment and bounded input, supervises a process group with identity-checked cleanup, captures bounded output, and returns typed receipts for completion, failure, cancellation, timeout, output-limit, start failure, and outcome-unknown paths. The no-desktop `alfredo-execution-provider` binary exposes one bounded JSONL request/response boundary for shadow samples.

The final review correction also prevents ordinary command arguments from becoming implicit host-read mounts. Only governed `PATH` executables and private temporary interpreter scripts may contribute implicit executable/script bindings; every other host read must come from the authority-declared sandbox boundary.

## Python-owned shadow controls

- `CanonicalStoreHashGuard` hashes every declared canonical store before and after a sample. Any unapproved byte change disables the sample; approved observation records are explicit and remain visible in the result.
- `normalize_execution_receipt` and `compare_execution_receipts` remove provider-generated identity/timestamp noise while retaining request identity, effect, status, exit outcome, output bytes/digests, reconciliation state, visible projections, process outcomes, and structured error fields.
- `ShadowCohortDefinition` recomputes immutable fixture/source/artifact digests, admits only measured production stages, and rejects reducer, sidecar, and microbenchmark evidence. Every sample binds the cohort, fixture root, artifact command, and stage marks.
- `RustShadowProvider` uses an allowlisted environment and bounded stdout/stderr readers; malformed output, non-zero provider exits, process crashes, start failures, timeout, invalid typed receipts, and structured provider failures are non-authoritative outcomes.
- `RustEligibilityStore` persists a separate, locked, atomic, directory-durable, fail-closed decision. Contract parity, canonical-store integrity, crash-cut, state-version, packaging, release-gate, production-equivalent cohort, and complete stage evidence are all required. Release evidence reopens the exact verified manifest and both tarballs, verifies their byte counts and hashes, inspects the provider and desktop metadata inside the platform archive, and rechecks the external provider plus manifest on every eligibility reload.

## Acceptance evidence

| Issue requirement | Evidence | Result |
|---|---|---|
| Shared contract behavior | Rust request-digest fixture, strict schema/sandbox/resource validation, typed start/status tests, process-level completion/timeout/output/cancellation tests, JSONL contract-failure round-trip, Python Issue #71 replay/crash tests, and the ten-cohort installed packaged-provider suite | Passes against both source-built and exact packaged providers on trusted WSL2 Linux/Bubblewrap |
| Canonical store integrity | Python hash guard and mutation/approved-observation tests plus before/after hashing around each installed Rust sample | Passes; an alternating mutate/restore provider fails on its first cohort |
| Normalized receipt and structured failure parity | Normalized receipt, process-outcome, visible-projection, structured-failure projections, provider crash, and packaged success/failure/timeout/output/cancellation/replay/validation cohorts | Passes against Python and Rust on Linux; the packaged suite binds the exact provider and suite SHA-256 values and leaves both canonical roots byte-unchanged |
| Production-equivalent shadow cohorts | Recomputed fixture/source/provider digests, bound roots/commands, measured S/R stages, and forbidden reducer/sidecar/microbenchmark tests | Passes |
| Eligibility and fallback | All-gates-required, artifact-bound release evidence, crash-cut/state-version/store-integrity, atomic persistence, and malformed-state fail-closed tests | Passes; no cutover is enabled by this implementation |

No Rust reducer, sidecar, or microbenchmark result is presented as a product speed claim.

## Linux acceptance correction (2026-08-30)

The first trusted-Linux run exposed two contract gaps hidden by the earlier Darwin skip. The JSONL success fixture had dropped Python's required `prlimit` wrapper and canonicalized fixed `/bin`-style mount destinations away from the production request shape. After the fixture was made production-shaped, Rust rejected the required nested resource separator and treated fixed merged-`/usr` mount aliases as non-canonical. Rust also used narrower default-profile numbers as schema maxima, omitted three Python-approved sanitized environment keys, accepted an unwrapped resource command, and detected only adjacent duplicate allowed paths.

The corrected Rust boundary now splits the outer Bubblewrap argv at its first separator, validates the nested trusted `prlimit` wrapper and exact limits, permits only the six exact fixed read-only system mount pairs without weakening canonical validation for other mounts, and matches Python's versioned resource, environment, and allowed-path validation. Pure Rust tests retain these rules even on hosts where the process fixture must skip. Python remains the sole canonical writer and Rust eligibility remains disabled.

## Packaged-provider release gate (2026-08-30)

The Linux platform package carries `bin/alfredo-execution-provider` beside the AppImage and records its SHA-256 in `desktop.json`. Production release generation builds the no-desktop provider with release optimizations and no desktop features. The isolated-registry verifier requires the exact file in the pack manifest, installs it through the meta package's optional platform dependency, rejects symlinks or digest changes, and runs exactly ten named/statused cohorts through the installed Python backend and exact installed Rust provider: success, non-zero failure, timeout with descendant cleanup, aggregate output limit, cancellation, Python-authoritative provider-free replay, provider crash, missing resource wrapper, undeclared host read, and schema-version rejection. The timeout cohort proves externally that neither provider leaves a delayed descendant marker, and the crash cohort compares the complete normalized receipt. It accepts publishable output only when the exact cohort inventory, per-sample normalized parity/proof fields, and recomputed suite digest match while before/after hashes around every Rust sample leave the actual installed canonical runtime tree plus sentinel root byte-unchanged. `release:check` independently extracts the provider from the preserved platform tarball and recomputes the same digest against the same-job verification manifest.

The current publishable gate binds the 1,201,328-byte provider at SHA-256 `d4b6b6428829a0b25b346a43fee5c4c9cd79d7bf4dbb67a37d43f82916521ea0`; the ten request digests produce suite digest `6572c7985a65aa88462deb2cfebc3dd66afede3d78ae6d349441734a0c4c8599`. The final preserved platform tarball SHA-256 is `2b9b7266d143578a6690436fa00f83099267bbb24c024f453f5da025428f0785`, the meta tarball SHA-256 is `547f1be2673f135248cf69e46ce5de66f409c7add8eb0b6208c971aa1cfb45c1`, and the same-job release manifest SHA-256 is `73f496b5ec187dc6f21f7bb1822920b95ae6ac2c0b6dee5b92a426096389500a`. The installed AppImage is 79,763,960 bytes at SHA-256 `f80b4cbf4bc08730e5cd33e0bad31c3a66584ff54e2dc4d474430e18d93757c3`. These are local same-job verification facts, not external publication or provenance.

Eligibility does not trust a synthetic summary of those facts. `RustReleaseGateEvidence` validates the complete production manifest shape, exact package names/order/version, npm SHA-512 integrity, platform and meta `package.json` identities, CLI aliases, exact optional dependency, AppImage/provider bytes, and complete `desktop.json` contract. Its persisted provider and manifest paths/hashes are revalidated on load, so post-eligibility tampering fails closed. The build-time and runtime Rust inventories intentionally repeat the exact contract boundary so packaging and invocation each assert it independently.

## Verification

Current trusted-Linux verification passed:

- `python3 -m unittest tests.test_execution_shadow` — 16 passed, including exact release-artifact identity/integrity/reload, mutate/restore rejection, descendant cleanup, and full crash parity
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml --no-default-features execution -- --nocapture` — 14 passed, including real Linux completion, timeout, output-limit, cancellation, undeclared-host-read, and provider-envelope control paths
- `cargo build --manifest-path mission-control/src-tauri/Cargo.toml --no-default-features --bin alfredo-execution-provider`
- `uv run --with pytest python -m pytest -q tests/test_execution.py tests/test_execution_integration.py tests/test_execution_shadow.py` — 44 passed plus 10 subtests, with the real JSONL/Bubblewrap parity sample executed rather than skipped
- `npm test -- --run tests/shadow-provider-contract.test.js tests/alfredo-entrypoint.test.js` — 4 passed, including exact ten-cohort evidence validation and the isolated-registry packaged-provider fixture
- `npm run typecheck` in `mission-control/`
- `npm run release:verify` — publishable isolated-registry install, exact packaged-provider digest, ten installed Python/Rust cohorts, unchanged canonical runtime tree, and GUI/backend smoke passed
- `npm run release:check` — independently reopened both tarballs and matched the packaged provider to the verified manifest
- `audit_documentation.py --path . --json` — 6 unrelated historical findings: 3 broad missing-connection heuristics and 3 orphaned July reports
- `validate_standards.py --path . --json` — grade A / 98%: 92% traceability and 100% accuracy, completeness, and discoverability; the 3 low findings are the same orphaned July reports

The broader final matrix also recorded mainline failures outside this slice rather than hiding them: Python discovery ran 755 tests with 21 failures, 28 errors, and 2 skips across retirement/repair/capability/shell behavior; the general Vitest run passed 292 tests and failed one cloud-controller timeout plus the pre-existing Node-environment `window` setup suite; the dedicated gateway command hit that same setup error; and full Rust/Tauri passed 65 of 66 with one review/retirement fixture failure. The earlier 12 Rust execution tests, 44 focused Python execution/shadow tests plus 10 subtests, four performance test files, production fixtures, TypeScript, build, functional browser, and four layout viewports passed; the post-review 14-test Rust slice, 16-test Python shadow slice, 4-test Node contract/package slice, release verification, real-artifact eligibility load, and release checking also pass.

The initial 2026-08-15 Darwin evidence remains historical: 9 Rust tests passed, 25 focused Python tests passed with one Bubblewrap-dependent skip, and formatting, build, check, frontend typecheck/build, Ruff, Python compilation, and diff checks passed. It was not trusted-Linux success-parity evidence.

## Files

- `albert_mvp/execution_shadow.py` — Python shadow runner, store guard, parity projection, provider adapter, cohort and eligibility gates.
- `tests/test_execution_shadow.py` — Python shadow, parity, crash, structured-failure, store-integrity, cohort, and eligibility tests.
- `mission-control/src-tauri/src/execution.rs` — Rust request/receipt model, provider, supervision, cancellation, output/resource bounds, and tests.
- `mission-control/src-tauri/src/bin/alfredo-execution-provider.rs` — no-desktop JSONL provider process.
- `mission-control/src-tauri/Cargo.toml` and `Cargo.lock` — direct `libc` and `sha2` dependencies.
- `mission-control/scripts/verify-shadow-provider.py` — installed Python/Rust parity and store-integrity release probe.
- `mission-control/scripts/shadow-provider-contract.js` — exact ten-cohort evidence, proof-field, root, status, digest, and suite-digest consumer contract.
- `mission-control/tests/shadow-provider-contract.test.js` — passing and fail-closed regression coverage for the release-evidence consumer.
- `mission-control/scripts/build-desktop-release.js`, `build-npm-release.js`, `verify-installed-release.js`, and `check-verified-release.js` — release build, package, install, parity, digest, and preserved-tarball gates for the exact provider.

## Human acceptance and rollback

Issue #72 remains `ready-for-human` until the Mission Commander reviews this evidence. No product cutover is part of this change: Issues #73 and #74 own effect-specific Rust cuts, and Issue #75 owns integrated packaged-workstation cutover/fallback verification. Any parity, store-integrity, crash, protocol, package, or release failure records a disabled reason and leaves Python authority unchanged.

Related architecture and protocol guidance is recorded in [project architecture](../System/project_architecture.md), [API/command boundaries](../System/api_endpoints.md), and the living [agent context](../Tasks/context.md).
