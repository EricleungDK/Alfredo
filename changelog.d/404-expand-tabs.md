### Fixed

- Tabs no longer vanish from chat replies, expanded tool results and approval cards:
  each expands to the next 4-column stop, so tab-indented code keeps its indent. On an
  approval card a tab shows as `→` plus padding (a literal `→` shows as `<U+2192>`), so
  an edit that only adds or removes a tab no longer shows two identical `-`/`+` lines.
