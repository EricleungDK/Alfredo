### Fixed

- `--num-ctx` no longer refuses every request at 4096 or below, or halves the usable
  prompt at 8192. The reply limit (`num_predict`) is now clamped to the room the window
  leaves after the estimated prompt (never below 256), and a request is refused only
  when the estimated prompt plus 256 tokens exceeds the window. Chat keeps "shorten the
  request"; planner, worker and repair refusals say "raise --num-ctx (≈N tokens
  needed)". A reply cut off by the clamped limit names the real limit and suggests
  raising `--num-ctx`. Without `--num-ctx`, requests are unchanged.
