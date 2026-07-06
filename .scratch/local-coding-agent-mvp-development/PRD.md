Status: complete

# Local Coding Agent MVP Development Roadmap

## Parent

.scratch/local-coding-agent-mvp/PRD.md

## Problem Statement

Albert's product workflow is itself part of the product under development. The current development work should not depend on approving and launching Issue Slices through Albert, because that would couple the product's fixture/workflow data to the way the product is built.

This development tracker isolates implementation work for building Albert's MVP from the product workflow that Albert will eventually provide.

## Scope

Build the next MVP layer for Albert using normal repository development practices: code changes, tests, and review. The issues in this tracker are implementation backlog items, not Issue Slices to be launched by the current `albert_mvp` command.

## Development Principles

- Treat `.scratch/local-coding-agent-mvp/` as the product PRD and workflow fixture.
- Treat `.scratch/local-coding-agent-mvp-development/` as the implementation backlog for building the product.
- Do not use the unfinished Albert workflow as the required workflow for developing Albert.
- Keep implementation slices independently testable and mergeable.
- Preserve the product vocabulary from `CONTEXT.md` and the parent PRD.
