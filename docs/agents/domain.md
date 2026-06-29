# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- `CONTEXT.md` at the repo root.
- `docs/adr/` if it exists and touches the area being changed.

If any of these files do not exist, proceed silently. Do not suggest creating them upfront.

## File structure

This is a single-context repo:

```text
/
├── CONTEXT.md
├── docs/adr/
└── .agent/
```

## Use the glossary's vocabulary

When output names a domain concept, use the term as defined in `CONTEXT.md`. Avoid drifting to synonyms for established terms such as Frontier Model, Local Agent, Orchestrator, Evidence Package, Plan Grill Gate, Product Requirements Document, Issue Slice, and Issue Graph.

## Flag ADR conflicts

If output contradicts an existing ADR, surface the conflict explicitly rather than silently overriding it.
