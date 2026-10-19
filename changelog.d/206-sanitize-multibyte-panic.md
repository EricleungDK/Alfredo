### Security

- Stop check-output sanitizing from panicking when a run-worktree path follows multi-byte whitespace such as a no-break space, em space or ideographic space. A crafted check output could previously abort the worker mid-run and leave the run claimed.
