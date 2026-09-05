# Issue #86: release fixture Git line-ending repair

Source: https://github.com/EricleungDK/Alfredo/issues/86

## Outcome and scope

Confirmed against untouched main `3e03c6579bfc4fd1c73f2e94b5b9761cb1a99463`, downloaded into `/tmp/alfredo-issue86-diagnosis`. The temporary repository created by `writeReleaseTracker` does not inherit Alfredo's root `.gitattributes`. Inherited `core.autocrlf=true` consequently emits LF-to-CRLF warnings during `git add`; the existing empty-stderr assertion correctly fails before rendering App.

The implemented [fixture fix](../../mission-control/src/alfredo-release-seam.test.tsx) gives the fixture its own `* text=auto eol=lf` policy. It retains every existing journey assertion and subprocess diagnostic assertion. Three regression cases invoke the actual fixture builder with isolated inherited `autocrlf=true/false/input`, `eol=crlf`, and strict `safecrlf=true`; they verify committed content, byte-identical LF checkout, and clean Git status. All environment overrides are restored and temporary repositories removed in `finally`.

The follow-up implement request applied the verified patch to the current local branch and to an isolated repair branch based on remote main. No production behavior or global Git configuration changed. Pre-existing checkout context edits remain unstaged.

## Feedback and hypotheses

Original exact-symptom command, from the isolated `mission-control` directory:

```bash
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.autocrlf GIT_CONFIG_VALUE_0=true npm test -- --run src/alfredo-release-seam.test.tsx
```

Untouched result: one failed test at `writeReleaseTracker:141`; Git stderr contains the three reported `LF will be replaced by CRLF` warnings for `01-release-seam.md`, `02-runner-recovery.md`, and `PRD.md`.

The subsecond diagnostic harness at `/tmp/alfredo-issue86-diagnosis/repro.cjs` extracts and executes the actual fixture builder, retaining its assertions. Repeated inherited `true` runs fail; `false` passes. This minimizes away the entire UI/backend journey.

Ranked predictions shown before probes:

1. Inherited autocrlf causes the warnings: changing only true to false should pass. Confirmed.
2. Missing fixture-local policy allows the inheritance: adding LF attributes while retaining inherited true should pass. Confirmed.
3. Another host configuration is required: isolated configuration containing only true should not reproduce. Falsified.

## Verification

With the lockfile dependencies installed in the isolated directory:

```bash
npm test -- --run src/alfredo-release-seam.test.tsx -t 'release tracker preserves LF'
npm run typecheck
```

- Regression before fix: one failed (`true`), two passed; strict safecrlf rejects the same unwanted conversion. 0.83 seconds total.
- Regression after fix: three passed. 0.81 seconds total. The full journey is excluded only by this focused invocation, not skipped in source.
- Typechecking: passed. Initial reused dependencies lacked Node types; isolated `npm ci --ignore-scripts` resolved that setup limitation without dependency-file changes.
- Original full journey after fix under inherited true: fixture passes; journey fails at `JSON.parse(launch.stdout)` with `Unexpected end of JSON input`.
- Untouched main under false, which avoids the fixture defect: the same launcher parse failure at original line 458. Thus the later failure is independent; its root cause was not investigated in this bounded task.
- No full-journey, product-release, or Windows acceptance success is claimed. Verification host: Linux/WSL, Node 24.11.0.

Raw logs remain at `/tmp/issue86-regression-red.log`, `/tmp/issue86-regression-green.log`, `/tmp/issue86-verified.log`, and `/tmp/issue86-baseline-later-failure.log`. No debug instrumentation was added to product code; the throwaway harness is retained in the clearly named temporary diagnosis directory.

## Ownership and prevention

Issue #86 assigns repair task `01a0712d-a6ed-79c0-a7f6-4e1c02cbc519`; GitHub showed no open PR and the cleanup PR #85 had already merged. The initial diagnosis stopped before publication. The user subsequently explicitly requested implementation and issue closure, resolving ownership for this session. Publication is scoped to the reviewed local commit and one draft repair PR; issue closure is explicitly requested even though the draft remains unmerged. No merge is authorized.

The existing real fixture builder is an adequate regression seam. Owning the temporary repository's line-ending policy and exercising commit plus checkout under contrasting inherited settings would have prevented this defect; no architectural change is needed.

## Implementation verification (follow-up request)

- Current checkout regression: red before attributes, three cases green afterward.
- Typecheck: passes on both current local source and isolated current-main repair source.
- Standards review: zero findings. Spec review: zero findings. Independent read-only reviewers examined the bounded delta against local starting commit a0195ad.
- Permitted full frontend suite on current-main repair branch: **324 passed, 1 failed**, 16 files, 43.34 seconds. All three new regressions pass.
- The remaining full-journey failure is a fail-closed retirement check: `Open-handle inspection was unavailable before retirement: [Errno 13] Permission denied: '/proc/527/cwd' (systemd)`.
- Untouched main's original release-seam file, run with inherited autocrlf=false under the same permitted environment, produces that exact same retirement reason (18.71 seconds). It is independent of this test-fixture fix.
- The initial sandboxed full suite reported 288 passed / 37 failed, including `listen EPERM` and empty launcher output. The permitted run resolves those sandbox failures. Neither result justifies weakening the retirement assertion or claiming the full journey passes.
- Final logs: `/tmp/issue86-local-red.log`, `/tmp/issue86-local-green.log`, `/tmp/issue86-full-suite-permitted.log`, `/tmp/issue86-baseline-permitted.log`, and both `issue86-*-typecheck.log` files.
- The source delta is 49 additions confined to one test file. No production code, dependency manifest, lockfile, or original journey assertion changed. The existing architecture offers an adequate regression seam; no architectural follow-up is required.
