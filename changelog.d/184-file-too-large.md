### Fixed

- An approved source file over 64 KiB (or approved files over 96 KiB together) no longer
  burns the repair budget with advice to try a larger model. The run fails with a typed
  `file-too-large` code and `calc.py is 153 KiB; workers can edit files up to 64 KiB. Split
  the file, or edit it by hand.`; autopilot stops without repairs and shows a `Next` row, a
  manual `/run` shows the same text, and the planning lint rejects a draft that approves a
  committed file over the bound.
