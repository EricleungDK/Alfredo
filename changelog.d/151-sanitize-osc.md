### Fixed

- Worker failure tails and repair prompts no longer show leftover escape-sequence text
  such as `8;;file:///...` (hyperlinks), `0;title` (window titles) or a stray `B`
  (from `tput sgr0`). OSC, DCS, APC and PM strings and charset selections are now
  removed whole.
