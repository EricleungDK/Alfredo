### Fixed

- `/go` and `/plan` no longer fail after about 90 s on repositories with more than about
  2,500 tracked files. Planning streams the committed tree listing and keeps a bounded
  ranked selection (tested at 20,000 files); above 1,000,000 entries it degrades with
  `listing_truncated` instead of failing. The shared Git helper now kills Git as soon as
  its output passes the 256-KiB bound and fails at once with "Git output exceeds the
  bounded evidence limit" instead of waiting out the 30-second deadline.
