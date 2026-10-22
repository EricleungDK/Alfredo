### Fixed

- Eval M8 probe: the mock server now stamps the reply time before writing it and the probe waits for a late stamp, so a fast screen read no longer nulls M8 with "never reached the server"; reply to screen is clamped at 0.
