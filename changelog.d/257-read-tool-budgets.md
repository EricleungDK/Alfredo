### Fixed

- Chat `grep` stops after 64 MiB scanned or 10 s and reports `[stopped: scanned N MiB in S s; narrow with path]`; the read calls of one model reply share a 30 s limit, not counting approval or write/run time (later reads answer `error: not run; round time limit reached`); and Esc now stops a running grep or git read within about a second instead of letting it run on.
