#!/usr/bin/env python3
"""Fold changelog.d/ fragments into CHANGELOG.md under [Unreleased].

Each fragment holds one or more Keep a Changelog sections (`### Fixed` ...) with bullet
entries. Entries are appended to the matching section of [Unreleased], creating the
section in canonical order when missing, and the fragments are deleted.
`--check` only validates the fragments.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SECTIONS = ['Added', 'Changed', 'Deprecated', 'Removed', 'Fixed', 'Security']
HEADING = re.compile(r'^### (\w+)\s*$')


def fragment_paths(root: Path) -> list[Path]:
    folder = root / 'changelog.d'
    return sorted(p for p in folder.glob('*.md') if p.name != 'README.md')


def parse_fragment(path: Path) -> dict[str, str]:
    """Return {section: entries text}; raise ValueError on a malformed fragment."""
    sections: dict[str, list[str]] = {}
    current = None
    for line in path.read_text(encoding='utf-8').splitlines():
        match = HEADING.match(line)
        if match:
            current = match.group(1)
            if current not in SECTIONS:
                raise ValueError(f'{path.name}: unknown section "### {current}"')
            sections.setdefault(current, [])
            continue
        if current is None:
            if line.strip():
                raise ValueError(f'{path.name}: text before the first "### Section" heading')
            continue
        sections[current].append(line)
    result = {name: '\n'.join(lines).strip('\n') for name, lines in sections.items()}
    result = {name: text for name, text in result.items() if text.strip()}
    if not result:
        raise ValueError(f'{path.name}: no entries')
    for name, text in result.items():
        if not text.lstrip().startswith('- '):
            raise ValueError(f'{path.name}: entries under "### {name}" must be "- " bullets')
    return result


def fold(changelog: str, fragments: list[dict[str, str]]) -> str:
    lines = changelog.split('\n')
    start = next(i for i, l in enumerate(lines) if l.startswith('## [Unreleased]'))
    end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith('## [')), len(lines))
    body = lines[start + 1:end]

    # Split [Unreleased] into its sections, keeping any preamble.
    order: list[str] = []
    blocks: dict[str, list[str]] = {}
    preamble: list[str] = []
    current = None
    for line in body:
        match = HEADING.match(line)
        if match:
            current = match.group(1)
            order.append(current)
            blocks[current] = []
            continue
        (blocks[current] if current else preamble).append(line)

    for fragment in fragments:
        for name, text in fragment.items():
            if name not in blocks:
                blocks[name] = []
                order.append(name)
            entries = blocks[name]
            while entries and not entries[-1].strip():
                entries.pop()
            if entries:
                entries.append('')
            entries.extend(text.split('\n'))

    order.sort(key=lambda n: SECTIONS.index(n) if n in SECTIONS else len(SECTIONS))
    out = [l for l in preamble if l.strip()]
    rebuilt = [lines[start], '']
    if out:
        rebuilt += out + ['']
    for name in order:
        entries = blocks[name]
        while entries and not entries[0].strip():
            entries.pop(0)
        while entries and not entries[-1].strip():
            entries.pop()
        if entries:
            rebuilt += [f'### {name}', ''] + entries + ['']
    return '\n'.join(lines[:start] + rebuilt + lines[end:])


def main(argv: list[str] | None = None, root: Path = ROOT) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--check', action='store_true', help='validate fragments only')
    args = parser.parse_args(argv)
    paths = fragment_paths(root)
    try:
        parsed = [parse_fragment(p) for p in paths]
    except ValueError as error:
        print(f'changelog fragment error: {error}', file=sys.stderr)
        return 1
    if args.check:
        print(f'{len(paths)} changelog fragment(s) OK')
        return 0
    if not paths:
        print('no changelog fragments to collect')
        return 0
    changelog = root / 'CHANGELOG.md'
    changelog.write_text(fold(changelog.read_text(encoding='utf-8'), parsed), encoding='utf-8')
    for path in paths:
        path.unlink()
    print(f'folded {len(paths)} fragment(s) into CHANGELOG.md')
    return 0


if __name__ == '__main__':
    sys.exit(main())
