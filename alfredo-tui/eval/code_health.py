"""M15 code health, computed mechanically (stdlib only; shells out to cargo for clippy and the test list).

`python3 alfredo-tui/eval/code_health.py` prints the numbers; `run_eval.py --code-health` stores them as M15.

Method (also in eval/README.md):
- pedantic: `cargo clippy --locked --lib --bins --message-format=short -- -W clippy::pedantic`; counts the
  `path:line:col: warning:` diagnostic lines (not the `generated N warnings` summary lines), split lib/bin by
  those summary lines, which must add up to the diagnostic count.
- Non-test code: `src/**/*.rs` minus every item gated by a test-only `#[cfg(...)]` (see TEST_CFG). The gated
  item is skipped by brace matching, so code after a `#[cfg(test)] mod tests { ... }` still counts and a gated
  `use`, `fn` or statement hides only itself. A file-level `#![cfg(test)]` hides the whole file. `tests/*.rs`
  (integration tests) is test code and is excluded from every count here.
- unwrap/expect: occurrences of `.unwrap(` / `.expect(` in non-test code, ignoring comments and string literals
  (so `unwrap_or`, `expect_err` and prose do not count).
- loc: physical lines (`wc -l`, blank lines and comments included) of non-test code; the largest module is the
  `src/**/*.rs` file with the most.
- rust_tests: `cargo test --locked -- --list` test entries, minus the `--ignored` list; py_tests: `def test_`
  methods in `alfredo-tui/tests/test_*.py` (the `*_smoke.py` journeys are not counted).
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

TUI = Path(__file__).resolve().parents[1]

# A cfg predicate is test-only code when it names `test` and has no `not(`: #[cfg(test)],
# #[cfg(all(test, unix))], #[cfg(any(test, feature = "x"))], #[cfg(all(unix, test))].
CFG_ATTRIBUTE = re.compile(r'[ \t]*#(!?)\[\s*cfg\s*\(', re.M)
HAS_TEST = re.compile(r'\btest\b')
STATEMENT_ITEM = re.compile(r'(?:pub(?:\([^)]*\))?\s+)?(?:use\b|static\b|type\b|let\b|extern\s+crate\b|const\s+(?!fn\b|unsafe\b|async\b))')
ITEM_START = re.compile(r'(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe|const|default)\s+)*(?:extern\s+(?:"[^"]*"\s+)?)?'
                        r'(?:fn|mod|impl|struct|enum|trait|union|macro_rules!)\b')
CALL = re.compile(r'\.(?:unwrap|expect)\(')
DIAGNOSTIC = re.compile(r'^\S[^\n]*?:\d+:\d+: (?:warning|error)\b', re.M)
SUMMARY = re.compile(r'^warning: `[^`]+` \((lib|bin[^)]*)\) generated (\d+) warnings?', re.M)


class CodeHealthError(Exception):
    """A measurement could not be taken (reported as that part's null reason)."""


def mask(text):
    """Return `text` with comments and the insides of string/char literals blanked (newlines kept)."""
    out = list(text)
    n, i = len(text), 0

    def blank(a, b):
        for k in range(a, min(b, n)):
            if out[k] != '\n':
                out[k] = ' '

    while i < n:
        c = text[i]
        if text.startswith('//', i):
            end = text.find('\n', i)
            end = n if end < 0 else end
            blank(i, end)
            i = end
        elif text.startswith('/*', i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith('/*', j):
                    depth, j = depth + 1, j + 2
                elif text.startswith('*/', j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c == 'r' and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == '_')) \
                and re.match(r'r#*"', text[i:i + 40]):
            hashes = len(re.match(r'r(#*)"', text[i:i + 40]).group(1))
            close = '"' + '#' * hashes
            end = text.find(close, i + 2 + hashes)
            end = n if end < 0 else end + len(close)
            blank(i + 2 + hashes, end - len(close))
            i = end
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == '\\' else 1
            blank(i + 1, j)
            i = j + 1
        elif c == "'":
            if text[i + 1:i + 2] == '\\':
                end = text.find("'", i + 3)
                end = n if end < 0 else end
                blank(i + 1, end)
                i = end + 1
            elif text[i + 2:i + 3] == "'":
                blank(i + 1, i + 2)
                i += 3
            else:  # a lifetime such as 'a or 'static
                i += 1
        else:
            i += 1
    return ''.join(out)


def _match_close(masked, start):
    """Index just past the bracket that closes the one at `start`."""
    depth, j = 0, start
    while j < len(masked):
        if masked[j] in '([{':
            depth += 1
        elif masked[j] in ')]}':
            depth -= 1
            if depth == 0:
                return j + 1
        j += 1
    return len(masked)


def _item_end(masked, pos):
    """End index of the item or statement starting at `pos` (after its attributes)."""
    while True:
        match = re.compile(r'\s*#!?\[').match(masked, pos)
        if not match:
            break
        pos = _match_close(masked, match.end() - 1)
    pos = len(masked) - len(masked[pos:].lstrip())
    if STATEMENT_ITEM.match(masked, pos):
        depth, j = 0, pos
        while j < len(masked):
            if masked[j] in '([{':
                depth += 1
            elif masked[j] in ')]}':
                depth -= 1
            elif masked[j] == ';' and depth <= 0:
                return j + 1
            j += 1
        return len(masked)
    if not ITEM_START.match(masked, pos):
        return _field_or_statement_end(masked, pos)
    depth, j = 0, pos
    while j < len(masked):
        char = masked[j]
        if char in '([':
            depth += 1
        elif char in ')]':
            depth -= 1
        elif char == '{' and depth <= 0:
            return _match_close(masked, j)
        elif char == ';' and depth <= 0:
            return j + 1
        j += 1
    return len(masked)


def _field_or_statement_end(masked, pos):
    """End of a gated struct field, enum variant, match arm or expression statement: the first `,` or `;`
    outside brackets and generics, or the bracket that closes the enclosing block."""
    depth = angle = 0
    j = pos
    while j < len(masked):
        char = masked[j]
        if char in '([{':
            depth += 1
        elif char in ')]}':
            depth -= 1
            if depth < 0:
                return j
        elif char == '<' and masked[j - 1] not in ' =<' and depth == 0:
            angle += 1
        elif char == '>' and angle and masked[j - 1] not in '-=':
            angle -= 1
        elif char in ',;' and depth == 0 and angle == 0:
            return j + 1
        j += 1
    return len(masked)


def test_spans(masked):
    """(start, end) character spans of test-gated code, or None when the whole file is test code."""
    spans = []
    for match in CFG_ATTRIBUTE.finditer(masked):
        close = _match_close(masked, match.end() - 1)
        predicate = masked[match.end():close - 1]
        if not HAS_TEST.search(predicate) or re.search(r'\bnot\s*\(', predicate):
            continue
        if match.group(1):
            return None
        spans.append((match.start(), _item_end(masked, masked.index(']', close) + 1)))
    return spans


def scan_source(text):
    """Non-test numbers for one Rust source: {unwrap_expect, loc_nontest, loc_total}."""
    total = len(text.splitlines())
    masked = mask(text)
    spans = test_spans(masked)
    if spans is None:
        return {'unwrap_expect': 0, 'loc_nontest': 0, 'loc_total': total}
    test_lines = set()
    for start, end in spans:
        first = masked.count('\n', 0, start)
        last = masked.count('\n', 0, max(start, end - 1))
        test_lines.update(range(first, last + 1))
    calls = sum(1 for m in CALL.finditer(masked) if masked.count('\n', 0, m.start()) not in test_lines)
    return {'unwrap_expect': calls, 'loc_nontest': total - len(test_lines), 'loc_total': total}


def scan_tree(root):
    """Scan `<root>/src/**/*.rs`; paths in the result are relative to `root`."""
    root = Path(root)
    unwrap_expect = total_loc = 0
    largest = None
    for path in sorted((root / 'src').rglob('*.rs')):
        result = scan_source(path.read_text(encoding='utf-8'))
        unwrap_expect += result['unwrap_expect']
        total_loc += result['loc_nontest']
        if largest is None or result['loc_nontest'] > largest['loc_nontest']:
            largest = {'path': path.relative_to(root).as_posix(), 'loc_nontest': result['loc_nontest']}
    return {'unwrap_expect_nontest': unwrap_expect, 'largest_module': largest, 'total_loc': total_loc}


def parse_clippy(output):
    """Count clippy diagnostics in `--message-format=short` output; split lib/bin by the summary lines."""
    diagnostics = len(DIAGNOSTIC.findall(output))
    lib = bins = 0
    for target, count in SUMMARY.findall(output):
        if target == 'lib':
            lib += int(count)
        else:
            bins += int(count)
    if lib + bins != diagnostics:
        raise CodeHealthError(f'clippy printed {diagnostics} diagnostics but its summary lines add up to '
                              f'{lib + bins} (lib {lib}, bin {bins}); duplicates or a changed output format')
    return {'pedantic_lib': lib, 'pedantic_bin': bins, 'pedantic': diagnostics}


def run_clippy(root, timeout=1800):
    command = ['cargo', 'clippy', '--locked', '--manifest-path', str(Path(root) / 'Cargo.toml'), '--lib', '--bins',
               '--message-format=short', '--', '-W', 'clippy::pedantic']
    try:
        run = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise CodeHealthError(f'cargo clippy did not run: {error}') from error
    if run.returncode != 0:
        raise CodeHealthError('cargo clippy failed: ' + (run.stderr.strip().splitlines() or ['no output'])[-1])
    return parse_clippy(run.stderr + run.stdout)


def count_listed_tests(listing):
    """Number of tests in `cargo test -- --list` output (benchmarks excluded)."""
    return sum(1 for line in listing.splitlines() if line.endswith(': test'))


def count_rust_tests(root, timeout=3600):
    """(tests not ignored, tests ignored) from `cargo test --locked -- --list [--ignored]`."""
    base = ['cargo', 'test', '--locked', '--manifest-path', str(Path(root) / 'Cargo.toml')]
    counts = []
    for extra in ([], ['--ignored']):
        try:
            run = subprocess.run(base + ['--', '--list'] + extra, capture_output=True, text=True, timeout=timeout)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise CodeHealthError(f'cargo test --list did not run: {error}') from error
        if run.returncode != 0:
            raise CodeHealthError('cargo test --list failed: ' + (run.stderr.strip().splitlines() or ['no output'])[-1])
        counts.append(count_listed_tests(run.stdout))
    everything, ignored = counts
    return everything - ignored, ignored


def count_python_tests(root):
    return sum(len(re.findall(r'^\s*def test_', path.read_text(encoding='utf-8'), re.M))
               for path in sorted((Path(root) / 'tests').glob('test_*.py')))


def collect(root=TUI):
    """All M15 numbers. A part that cannot be measured is None and its reason is in `reasons`."""
    reasons = {}
    health = {'pedantic': None, 'pedantic_lib': None, 'pedantic_bin': None, 'rust_tests': None,
              'rust_tests_ignored': None}
    try:
        health.update(run_clippy(root))
    except CodeHealthError as error:
        reasons['pedantic'] = str(error)
    health.update(scan_tree(root))
    try:
        health['rust_tests'], health['rust_tests_ignored'] = count_rust_tests(root)
    except CodeHealthError as error:
        reasons['rust_tests'] = str(error)
    health['py_tests'] = count_python_tests(root)
    return health, reasons


def main(argv=None):
    parser = argparse.ArgumentParser(description='Print the M15 code-health numbers as JSON.')
    parser.parse_args(argv)
    health, reasons = collect()
    print(json.dumps({**health, 'reasons': reasons}, indent=2))
    return 1 if reasons else 0


if __name__ == '__main__':
    sys.exit(main())
