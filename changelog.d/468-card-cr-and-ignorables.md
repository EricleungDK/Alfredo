### Security

- The approval card shows a lone carriage return as `<U+000D>` instead of dropping it, so
  a line that Python, Node or YAML read as two statements no longer looks like one comment.
- Every Unicode default-ignorable character (variation selectors, Hangul fillers, Mongolian
  free variation selectors, tag characters and the unassigned code points in their ranges)
  shows as `<U+XXXX>` on the approval card and in `⚙` rows instead of being drawn raw.
- Every C0 and C1 control character except newline and tab (ESC, BEL, DEL, NEL and
  the rest) shows as `<U+XXXX>` on the approval card instead of being dropped.
