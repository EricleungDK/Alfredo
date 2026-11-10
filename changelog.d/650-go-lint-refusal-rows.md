### Fixed

- The interactive bare `/go` refusal for a plan the unattended lint rejects (for example
  approved files over the 96 KiB bound) now shows the whole finding and `Fix it with
  /plan-revise (draft) or run it by hand with /approve` in the chat rows, with a short
  footer, instead of one line cut at the terminal width. The finding's trailing period is
  no longer doubled (`by hand.. Fix it`).
