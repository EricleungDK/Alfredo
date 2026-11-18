### Security

- The `/go` review view (F3, `/evidence ID`), the dashboard evidence view and the live check
  output now show a worker's patch and check output with hidden characters visible as
  `<U+XXXX>`. A lone carriage return that Python, Node or YAML read as a line break no longer
  vanishes from its line, and bidi overrides, isolates, zero-width and other default-ignorable
  characters are no longer drawn raw (CWE-451, Trojan Source). The failing-check tail shows
  the raw output lines, so an escape sequence or a hyperlink target is no longer hidden.
  Tabs in a diff or check line show as visible `→` stops instead of disappearing. Ordinary
  CRLF line endings show no marker.
