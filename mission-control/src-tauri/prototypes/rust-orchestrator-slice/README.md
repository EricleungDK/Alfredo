# PROTOTYPE — Rust Orchestrator vertical slice

## Question

Can a narrow Rust Orchestrator slice read Alfredo's current schema-v1 Workspace
Snapshot, represent the agreed Starting Location → Coding Workspace → Mission
Formation Route → governed dispatch states, reuse the existing correlated
newline-JSON transport envelope, bind visible effects to receipts, survive
replay/crash cuts, and remain rollback-safe enough to inform the Python versus
staged-Rust architecture decision?

This is disposable decision evidence, not production authority. It writes only
short-lived `PROTOTYPE-*` files below a process-qualified temporary directory
during the rollback demonstration. It never reads or mutates Alfredo's real
runtime stores.

## Run the GUI review

From `mission-control`:

```bash
npm run prototype:rust-gui
```

This opens the real Tauri desktop workstation. The current React GUI and Python
backend remain live underneath a clearly marked Rust shadow review surface.
Every shadow request reads Python before and after, executes the transition in
Rust, keeps its candidate state in React memory, and reports whether Python
authority stayed unchanged. Use the bottom switcher or the `?variant=A|B|C`
query to compare:

- **A — Shadow rail:** a persistent engineering inspector beside the workstation.
- **B — Flight recorder bench:** a horizontal transition and receipt workbench.
- **C — Cutover lens:** a focused Python-versus-Rust authority comparison.

`Discard Rust state · return to live Python` drops the in-memory candidate and
reimports the live Python snapshot. It is the prototype rollback switch.

## Run the terminal evidence

From `mission-control`:

```bash
npm run prototype:rust-orchestrator
```

For the non-interactive supporting evidence:

```bash
npm run prototype:rust-orchestrator -- --review
```

For the existing persistent transport envelope:

```bash
printf '%s\n' '{"id":"review-1","argv":["prototype-review"]}' \
  | npm run prototype:rust-orchestrator -- --protocol
```

## Boundary

- `model.rs` is the pure, portable transition model.
- `main.rs` is the throwaway terminal/protocol/measurement shell.
- `mission-control/src/prototypes/RustOrchestratorGuiPrototype.tsx` mounts the
  live Rust shadow review around the current production React workstation.
- Tauri command `rust_orchestrator_prototype` imports the current Python
  projection and applies the same reducer in memory; it performs no canonical
  write and is present only to answer this Wayfinder decision.
- Current schema-v1 state imports into a versioned prototype model without
  rewriting the source bytes.
- The current launch contract still cannot represent “selection required”
  because `selected_workspace` is mandatory. The slice therefore demonstrates
  that a versioned contract extension is required; it does not pretend the
  unchanged v1 contract can express the new journey.
- Python remains the only production authority unless a later architecture
  decision explicitly migrates a bounded module.
