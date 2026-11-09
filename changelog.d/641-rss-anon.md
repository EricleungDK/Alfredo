### Added

- Eval M16 now also reports `secondary.rss_anon_kb`, the median sampled peak of `RssAnon`
  (anonymous memory, without the binary's page-cache-dependent file pages), and each trial
  records `rss_anon_kb`. The headline M16 value stays `VmHWM`.
