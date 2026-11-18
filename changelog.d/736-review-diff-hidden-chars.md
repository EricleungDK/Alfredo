### Security

- The `/go` review view (F3, `/evidence ID`) and the dashboard evidence view now show a
  worker's patch and check output with hidden characters visible as `<U+XXXX>`. A lone
  carriage return that Python, Node or YAML read as a line break no longer vanishes from
  its line, and bidi overrides, isolates, zero-width and other default-ignorable characters
  are no longer drawn raw, so the diff reads as the bytes hold it (CWE-451, Trojan Source).
  Ordinary CRLF line endings show no marker.
