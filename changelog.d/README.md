# Changelog fragments

Pull requests do not edit `CHANGELOG.md` directly. Each change adds one small file here
instead, so parallel pull requests never conflict on the same lines.

## Add a fragment

Create `changelog.d/<issue>-<short-slug>.md`, for example `changelog.d/146-markdown-fence-strip.md`:

```markdown
### Fixed

- One or more bullets written exactly as they should appear in `CHANGELOG.md`,
  wrapped at about 90 columns.
```

- The first non-empty line is the Keep a Changelog section: `### Added`, `### Changed`,
  `### Deprecated`, `### Removed`, `### Fixed` or `### Security`.
- A fragment may contain several sections; each starts with its `###` heading.
- Use a unique file name (the issue number keeps it unique).

## Release

Before tagging a release, fold the fragments into `CHANGELOG.md` under `[Unreleased]`
and delete them:

```bash
python3 alfredo-tui/scripts/collect_changelog.py
```

Then rename `[Unreleased]` to the version as usual. `--check` validates fragments without
changing anything; CI runs it through `alfredo-tui/tests/test_changelog_fragments.py`.
