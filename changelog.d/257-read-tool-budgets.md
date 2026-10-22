### Fixed

- Chat `grep` stops after 64 MiB scanned or 10 s and reports `[stopped: scanned N MiB in S s; narrow with path]`; a tool round is limited to 30 s (later calls answer `error: not run; round time limit reached`); and Esc now stops a running grep or git read within about a second instead of letting it run on.
