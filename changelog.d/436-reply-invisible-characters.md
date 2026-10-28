### Security

- Chat replies show tag characters, zero-width characters, variation selectors and line or
  paragraph separators as `<U+XXXX>`, in prose and in fenced code, as the approval card
  does, instead of drawing them invisibly. Control characters in a reply (ESC, BEL and
  the rest, except newline and tab) show as `<U+XXXX>` too, as does a carriage return.
