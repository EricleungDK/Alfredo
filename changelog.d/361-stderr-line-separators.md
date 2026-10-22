### Security

- Headless chat stderr now shows invisible and bidi format characters from model-chosen
  text (such as U+202E) as `<U+XXXX>` instead of drawing them, so a path or tool argument
  cannot read differently in a CI log than it does when it runs.
- Line and paragraph separators (U+2028, U+2029, U+0085) are line breaks in single-line
  output: headless stderr and the other single-line helpers turn them into spaces, and the
  project-notes and plan-draft strips remove them, so no second line can be forged.
