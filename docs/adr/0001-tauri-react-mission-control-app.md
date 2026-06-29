# Use Tauri and React for the Mission Control App

Albert's Mission Control App will use Tauri as the desktop shell and React with TypeScript for the interactive user interface. This keeps the desktop runtime lighter than Electron while supporting the stateful Agent Console, Shell Terminal, live Issue Slice board, review controls, and local Orchestrator integration that a document-first CSS and MDX approach cannot provide alone.

## Consequences

Albert must expose a long-running local backend and batched event stream rather than starting a new Python process for every UI action. MDX may be used for document-like mission content, but accepted mission state and workflow controls remain structured React surfaces owned by the Orchestrator.
