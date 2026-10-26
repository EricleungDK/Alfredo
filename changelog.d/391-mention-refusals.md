### Fixed

- Chat `@path` refusals are no longer joined into one footer line that was cut to the
  width: every refused mention gets its own transcript row with the reason.
- A mention cut short by a space (`see @my file.txt` when `my file.txt` exists) or written
  in quotes (`@"my file.txt"`) now says `paths with spaces are not supported` instead of
  being dropped silently. Such paths are still never attached.
- `@path` and its limits are listed in `--help`, and `docs/reference.md` describes the
  refusal rows and the space and quote limits. Refusal notices show invisible characters
  as `<U+XXXX>`.
