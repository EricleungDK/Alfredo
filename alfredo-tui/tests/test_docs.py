"""Doc drift guards: entry-point docs link to real files and document every CLI flag."""
from pathlib import Path
import os
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
ENTRY_DOCS = ['README.md', 'CONTRIBUTING.md', 'docs/README.md', 'alfredo-tui/README.md',
              'alfredo-tui/INSTALL.md', '.agent/README.md', '.agent/Tasks/STATUS.md']
LINK = re.compile(r'\[[^\]]*\]\(([^)\s]+)\)')
IMAGE = re.compile(r'<img[^>]+src="([^"]+)"')


class LinkTests(unittest.TestCase):
    def test_relative_links_resolve(self):
        broken = []
        for name in ENTRY_DOCS:
            doc = ROOT / name
            self.assertTrue(doc.is_file(), f'{name} is missing')
            text = doc.read_text(encoding='utf-8')
            for target in LINK.findall(text) + IMAGE.findall(text):
                if re.match(r'[a-z]+:', target) or target.startswith('#'):
                    continue
                path = target.split('#')[0]
                if not (doc.parent / path).exists():
                    broken.append(f'{name} -> {target}')
        self.assertEqual([], broken)


class LegacyTests(unittest.TestCase):
    def test_docs_do_not_mention_removed_apps(self):
        names = [n for n in ENTRY_DOCS if not n.startswith('.agent/')]
        names += ['AGENTS.md', 'CONTEXT.md']
        names += sorted(p.relative_to(ROOT).as_posix() for p in (ROOT / 'docs/agents').glob('*.md'))
        found = []
        for name in names:
            text = (ROOT / name).read_text(encoding='utf-8')
            for word in ('mission' + '-control', 'albert' + '_mvp', 'docs/' + 'legacy.md', 'Tau' + 'ri'):
                if word in text:
                    found.append(f'{name}: {word}')
        self.assertEqual([], found)

    def test_legacy_docs_are_gone(self):
        left = [p.name for p in (ROOT / 'docs').glob('*.md') if p.name == 'legacy.md' or p.name.startswith('albert-')]
        self.assertEqual([], left)

    def test_agent_index_is_not_about_the_desktop_app(self):
        text = (ROOT / '.agent/README.md').read_text(encoding='utf-8')
        self.assertNotIn('Tauri', text)


class GlossaryTests(unittest.TestCase):
    def test_context_is_about_the_terminal(self):
        text = (ROOT / 'CONTEXT.md').read_text(encoding='utf-8')
        for term in ('Frontier', 'Orchestrator', 'Mission Commander', 'Command Deck', 'Albert'):
            self.assertNotIn(term, text)
        for term in ('Autopilot', 'Wayfinder', 'Mission', 'Worker'):
            self.assertIn(f'### {term}', text)


class FlagTests(unittest.TestCase):
    def test_quit_aliases_are_named_in_help_and_reference(self):
        # Issue #482: /exit and /quit quit like Ctrl+Q.
        source = (ROOT / 'alfredo-tui/src/main.rs').read_text(encoding='utf-8')
        reference = (ROOT / 'alfredo-tui/docs/reference.md').read_text(encoding='utf-8')
        self.assertIn('"/exit, /quit quit Alfredo, same as Ctrl+Q"', source)
        for name in ('/exit', '/quit'):
            self.assertIn(f'`{name}`', reference)

    def test_reference_says_line_breaks_in_a_goal_become_spaces(self):
        # Issue #743: a pasted or scripted multi-line goal is joined into one line.
        reference = (ROOT / 'alfredo-tui/docs/reference.md').read_text(encoding='utf-8')
        start = reference.index('## Autopilot')
        end = reference.index('\n## ', start)
        section = reference[start:end]
        for text in ('each run of line breaks in a goal becomes one space', '--go "$(cat task.md)"', '4096 bytes'):
            self.assertIn(text, section)
        # Issue #751: /plan-revise and tabs join too.
        for text in ('`/plan-revise`', 'tabs', 'vertical tabs', 'form feeds'):
            self.assertIn(text, section)

    def test_stream_json_text_event_is_named_in_help_and_reference(self):
        # Issue #762: --chat stream-json emits text {delta} events.
        source = (ROOT / 'alfredo-tui/src/main.rs').read_text(encoding='utf-8')
        reference = (ROOT / 'alfredo-tui/docs/reference.md').read_text(encoding='utf-8')
        self.assertIn('text {delta} for --chat', ' '.join(source.split()))
        start = reference.index('### Event stream: `--output-format stream-json`')
        section = reference[start:reference.index('\n### ', start + 10)]
        self.assertIn('| `text` | `--chat` |', section)
        self.assertIn('`delta`', section)

    def test_reference_states_the_combined_bound_for_plan_revise(self):
        text = " ".join((ROOT / "alfredo-tui/docs/reference.md").read_text(encoding="utf-8").split())
        for phrase in (
            "the draft's prompt, then ` | Revision request: ` (21 bytes), then your request, stay within 8192 bytes",
            "Revision would make the plan prompt N bytes; the limit is 8192. Use a shorter request, or /plan a fresh one",
        ):
            self.assertIn(phrase, text)

    def test_headless_chat_stdin_prompt_is_documented(self):
        # Issue #549: `--chat -` reads the prompt from stdin (256 KiB, UTF-8, nonempty).
        reference = (ROOT / 'alfredo-tui/docs/reference.md').read_text(encoding='utf-8')
        start = reference.index('### Headless chat')
        end = reference.index('\n## ', start)
        section = reference[start:end]
        for text in ('--chat -', 'stdin', '256 KiB', 'UTF-8', 'nonempty', '--chat needs a PROMPT'):
            self.assertIn(text, section)
        readme = (ROOT / 'alfredo-tui/README.md').read_text(encoding='utf-8')
        self.assertIn('--chat -', readme)

    def test_headless_docs_say_a_named_mission_can_be_opened(self):
        # Issue #716: after #705, headless opens the default mission or the one
        # `--mission NAME` / `--new-mission NAME` names; no text may say "default only".
        binary = os.environ.get('ALFREDO_TUI_BINARY')
        if binary and Path(binary).exists():
            help_text = subprocess.run([binary, '--help'], capture_output=True, text=True,
                                       check=True).stdout
        else:
            help_text = (ROOT / 'alfredo-tui/src/main.rs').read_text(encoding='utf-8')
        help_text = ' '.join(help_text.split())
        start = help_text.index('--headless (with --go GOAL)')
        entry = help_text[start:start + 1200]
        self.assertIn("Opens the repository's default mission, or the one --mission NAME / "
                      "--new-mission NAME names.", entry)
        self.assertNotIn("Opens the repository's default mission.", entry)
        reference = (ROOT / 'alfredo-tui/docs/reference.md').read_text(encoding='utf-8')
        for heading, needed in (('### Paused runs: `--resume --headless`', ('--mission NAME',)),
                                ('### Headless chat', ('--mission NAME', '--new-mission NAME'))):
            begin = reference.index(heading)
            section = ' '.join(reference[begin:reference.index('\n#', begin + 4)].split())
            for text in needed:
                self.assertIn(text, section, heading)
            self.assertNotIn("the default mission's paused run", section, heading)
            self.assertNotIn('on the repository\'s default mission,', section, heading)

    def test_both_reentrant_stops_are_named_as_resume_headless_exceptions(self):
        # Issue #722: a context-too-small stop retries like a model-unreachable one.
        readme = (ROOT / 'README.md').read_text(encoding='utf-8')
        line = next(l for l in readme.splitlines() if '`--resume --headless` continues' in l)
        self.assertIn('--num-ctx', line)
        reference = (ROOT / 'alfredo-tui/docs/reference.md').read_text(encoding='utf-8')
        begin = reference.index('### Paused runs: `--resume --headless`')
        section = ' '.join(reference[begin:reference.index('\n#', begin + 4)].split())
        self.assertIn('`model-unreachable` stop', section)
        self.assertIn('`context-too-small` stop', section)
        self.assertIn('(repair budget, sandbox, file too large) still exit 2', section)
        flat = ' '.join(reference.split())
        self.assertIn('the retry itself counts as one repair beyond `--max-repairs`', flat)
        self.assertNotIn('with no worker request and no further repair', flat)

    def test_new_app_wayfinder_rule_is_documented_as_a_verb_rule(self):
        # Issue #741: "the new app" alone is ordinary chat; a creation verb starts Wayfinder.
        reference = ' '.join((ROOT / 'alfredo-tui/docs/reference.md').read_text(encoding='utf-8').split())
        self.assertIn('counts only directly after a creation verb', reference)
        self.assertIn('names the matched phrase', reference)
        self.assertIn('with at most three of `a`, `an`, `another`, `brand`, `my`, `our`, `me`, `us`, `whole`, `completely`, `entirely`, `totally` between them', reference)
        self.assertNotIn('within four words', reference)
        main = ' '.join((ROOT / 'alfredo-tui/src/main.rs').read_text(encoding='utf-8').split())
        self.assertIn('starts Wayfinder only right after a creation verb and articles', main)
        self.assertIn('me, us, whole, completely, entirely, totally', main)

    def test_masked_secret_files_are_documented_as_permission_denied(self):
        # Issue #582: /dev/null is bound nodev, so a masked file fails to open.
        reference = (ROOT / 'alfredo-tui/docs/reference.md').read_text(encoding='utf-8')
        self.assertNotIn('reads as empty', reference)
        self.assertNotIn('(`cat .env` prints nothing)', reference)
        self.assertGreaterEqual(reference.count('fails with `Permission denied`'), 2)
        self.assertIn('are hidden in this sandbox', reference)
        self.assertIn('move them out of the Coding Workspace', reference)

    def test_env_templates_are_documented_as_read_only_in_the_sandbox(self):
        # Issue #598: the read tools' three template names are bound read-only, not masked.
        reference = ' '.join((ROOT / 'alfredo-tui/docs/reference.md').read_text(encoding='utf-8').split())
        for text in ('`.env.example`, `.env.sample` and `.env.template`', 'read-only over its own path',
                     'do not count toward the 128-mask limit', 'past 248 templates', 'looks empty',
                     'the usual `escalate: true` advice is left out'):
            self.assertIn(text, reference)
        self.assertNotIn('fail with Permission denied; they need the owner', reference)

    def test_readme_documents_every_flag(self):
        source = (ROOT / 'alfredo-tui/src/main.rs').read_text(encoding='utf-8')
        flags = set(re.findall(r'^\s*"(--[a-z-]+)"(?: \| "--[a-z-]+")* =>', source, re.M))
        flags |= set(re.findall(r'\| "(--[a-z-]+)"', source))
        self.assertGreater(len(flags), 15)
        readme = (ROOT / 'README.md').read_text(encoding='utf-8')
        missing = sorted(f for f in flags if f'`{f}' not in readme)
        self.assertEqual([], missing)

    def test_readme_default_model_matches_the_code_and_setup_docs(self):
        source = (ROOT / 'alfredo-tui/src/main.rs').read_text(encoding='utf-8')
        code = re.search(r'"ALFREDO_MODEL"\)\.unwrap_or_else\(\|_\| "([^"]+)"\.into\(\)\)', source)
        self.assertIsNotNone(code)
        readme = (ROOT / 'README.md').read_text(encoding='utf-8')
        row = re.search(r'^\| `--model NAME` \| `([^`]+)`', readme, re.M)
        self.assertIsNotNone(row)
        self.assertEqual(code.group(1), row.group(1))
        for name in ('README.md', 'alfredo-tui/INSTALL.md'):
            text = (ROOT / name).read_text(encoding='utf-8')
            self.assertIn(f'ollama pull {code.group(1)}', text, name)

    def test_readme_names_the_current_version_and_toolchain(self):
        cargo = (ROOT / 'alfredo-tui/Cargo.toml').read_text(encoding='utf-8')
        version = re.search(r'^version = "([^"]+)"', cargo, re.M).group(1)
        toolchain = re.search(r'^rust-version = "([^"]+)"', cargo, re.M).group(1)
        readme = (ROOT / 'README.md').read_text(encoding='utf-8')
        self.assertIn(version, readme)
        self.assertIn(f'Rust {toolchain}', readme)


    def test_on_finish_and_max_repairs_name_their_modes(self):
        # Issue #765: explicit flags with --chat/--doctor exit 2; help and reference say so.
        source = (ROOT / 'alfredo-tui/src/main.rs').read_text(encoding='utf-8')
        reference = (ROOT / 'alfredo-tui/docs/reference.md').read_text(encoding='utf-8')
        for text in (' '.join(source.split()), ' '.join(reference.split())):
            self.assertIn('apply only to --go, --resume and the terminal', text.replace('`', ''))


class StatusTests(unittest.TestCase):
    def test_status_is_not_stale_about_branches(self):
        status = (ROOT / '.agent/Tasks/STATUS.md').read_text(encoding='utf-8')
        self.assertNotIn('`main` does not yet', status)


class InstallSchemaTests(unittest.TestCase):
    def test_install_states_the_schema_versions_the_binary_writes(self):
        const = re.compile(r'SCHEMA_VERSION: u32 = (\d+)')
        tasks = const.search((ROOT / 'alfredo-tui/src/tasks.rs').read_text(encoding='utf-8'))
        convs = const.search((ROOT / 'alfredo-tui/src/conversations.rs').read_text(encoding='utf-8'))
        self.assertTrue(tasks and convs, 'SCHEMA_VERSION constant not found')
        install = (ROOT / 'alfredo-tui/INSTALL.md').read_text(encoding='utf-8')
        stated = re.search(r'task schema (\d+), conversation schema (\d+)', install)
        self.assertTrue(stated, 'INSTALL.md must state "task schema N, conversation schema M"')
        self.assertEqual((tasks.group(1), convs.group(1)), stated.groups())


class SourceLayoutTests(unittest.TestCase):
    def test_readme_source_layout_names_every_src_module(self):
        readme = (ROOT / 'alfredo-tui/README.md').read_text(encoding='utf-8')
        layout = readme.split('## Source layout', 1)[1].split('\n## ', 1)[0]
        missing = sorted(
            p.relative_to(ROOT / 'alfredo-tui').as_posix()
            for p in (ROOT / 'alfredo-tui/src').rglob('*.rs')
            if p.relative_to(ROOT / 'alfredo-tui').as_posix() not in layout
        )
        self.assertEqual(
            missing, [],
            'alfredo-tui/README.md "Source layout" does not name these modules: '
            + ', '.join(missing)
            + '. Add each as `src/<file>.rs` to an existing group row or a new row '
            'in that table, with a one-line purpose.')


if __name__ == '__main__':
    unittest.main()
