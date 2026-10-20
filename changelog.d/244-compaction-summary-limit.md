### Security

- A compaction summary longer than the 4 KiB note limit is now cut to fit, including the
  `[... N bytes omitted]` marker, so it can no longer make every later conversation save fail.
  A summary that still cannot be stored is reported as a failed summary and leaves the
  conversation unchanged.
