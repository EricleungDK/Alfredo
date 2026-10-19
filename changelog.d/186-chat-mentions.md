### Added

- Chat prompts accept `@relative/path` file mentions: the committed HEAD content of the file
  is attached to the message sent to the model (up to 8 files and 32 KiB per prompt, long
  files cut with a `[truncated: N of M bytes]` marker), while the transcript shows what you
  typed. Secret/generated paths, `..`, absolute, symlink, untracked, binary and oversized
  files are refused with a notice naming the path and reason; `a@b.com`, `@wayfinder` and
  bare `@words` are not treated as mentions. The chat preamble now says it sees only
  attached files.
