"""Pure helpers of the README recording script (alfredo-tui/recording/timeline.py)."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'recording'))
import timeline  # noqa: E402


class CutTests(unittest.TestCase):
    def test_cuts_from_stamped_vhs_output(self):
        lines = [
            (0.0, 'Hide'), (2.0, 'Show'),            # setup, nothing recorded yet
            (2.0, 'Type "x"'), (5.0, 'Hide'),        # 3 s shown
            (14.4, 'Show'), (14.4, 'Sleep 2s'),      # 9.4 s cut
            (16.4, 'Hide'), (20.0, 'Show'),          # 2 s shown, 3.6 s cut
        ]
        self.assertEqual([(3.0, 9.4), (5.0, 3.6)], timeline.cuts(lines))

    def test_time_before_first_show_is_not_recorded(self):
        lines = [(3.0, 'Hide'), (5.0, 'Show'), (5.0, 'Type "x"'), (7.0, 'Hide'), (10.0, 'Show')]
        self.assertEqual([(2.0, 3.0)], timeline.cuts(lines))

    def test_no_cut_when_nothing_hidden_after_start(self):
        self.assertEqual([], timeline.cuts([(0.0, 'Hide'), (1.0, 'Show'), (4.0, 'Sleep 1s')]))

    def test_label_names_what_was_cut(self):
        self.assertEqual('model planning — 9 s cut', timeline.label('model planning', 9.4))
        self.assertEqual('model thinking — 40 s cut', timeline.label('model thinking', 39.6))

    def test_snap_to_largest_jump_in_window(self):
        diffs = [0, 0, 1, 0, 50, 0, 0, 9, 0]
        self.assertEqual(4, timeline.snap(diffs, 5, 3))
        self.assertEqual(7, timeline.snap(diffs, 7, 1))

    def test_label_frames_start_at_jump_and_last_hold(self):
        self.assertEqual(range(40, 65), timeline.label_frames(40, fps=25, hold=1.0))


class TakeTests(unittest.TestCase):
    @staticmethod
    def task(i, status='accepted', title=None):
        return {'id': i, 'status': status, 'title': title or f'Task {i}'}

    def test_two_tasks_one_repair_is_kept_when_tape_watches_the_repair(self):
        tasks = [self.task(1), self.task(2, 'failed'), self.task(3, title='Repair #2: x')]
        self.assertTrue(timeline.keep_take(tasks, watch_id=3))

    def test_three_tasks_rejected_because_repair_id_is_not_watched(self):
        tasks = [self.task(1), self.task(2), self.task(3, 'failed'), self.task(4, title='Repair #3: x')]
        self.assertFalse(timeline.keep_take(tasks, watch_id=3))

    def test_rejects_unaccepted_repair_no_repair_or_single_task(self):
        self.assertFalse(timeline.keep_take([self.task(1), self.task(2), self.task(3, 'failed', 'Repair #2')], 3))
        self.assertFalse(timeline.keep_take([self.task(1), self.task(2)], 3))
        self.assertFalse(timeline.keep_take([self.task(1, 'failed'), self.task(2, title='Repair #1')], 2))

    def test_watch_id_is_read_from_the_tape(self):
        self.assertEqual(3, timeline.watch_id('Type "/watch 3"\nEnter\n'))


class SpeedTests(unittest.TestCase):
    LINES = [(2.0, 'Hide'), (4.0, 'Show'), (10.0, 'Hide'), (20.0, 'Show'), (24.0, 'Hide')]

    def test_shown_seconds_excludes_setup_and_cuts(self):
        self.assertEqual(10.0, timeline.shown_seconds(self.LINES))

    def test_speed_is_video_over_wall_clock(self):
        self.assertEqual(0.65, timeline.speed(video_s=6.5, shown_s=10.0))

    def test_retime_repeats_frames_to_real_time(self):
        # capture ran at half speed: 4 captured frames cover 8 real frames
        self.assertEqual([0, 0, 1, 1, 2, 2, 3, 3], timeline.retime(4, 0.5))
        self.assertEqual([0, 2], timeline.retime(4, 2.0))


class BudgetTests(unittest.TestCase):
    GOOD = dict(duration=25.0, fps=20, gif_bytes=3_000_000, width=1280, height=720)

    def test_accepts_readme_grade(self):
        self.assertEqual([], timeline.problems(**self.GOOD))

    def test_flags_each_budget(self):
        for key, bad, word in [('duration', 31.0, 'duration'), ('duration', 14.0, 'duration'),
                               ('fps', 30, 'fps'), ('fps', 10, 'fps'),
                               ('gif_bytes', 5_000_000, 'GIF'), ('width', 900, 'width')]:
            found = timeline.problems(**{**self.GOOD, key: bad})
            self.assertEqual(1, len(found), (key, bad, found))
            self.assertIn(word, found[0])


class LeakTests(unittest.TestCase):
    def test_flags_personal_strings(self):
        text = 'cwd /home/someone/x and /mnt/c/Users/y host box'
        self.assertEqual(['/home/', '/mnt/c'], timeline.leaks(text, extra=[]))

    def test_clean_text(self):
        self.assertEqual([], timeline.leaks('~/sample $ alfredo-tui', extra=['ericl']))

    def test_extra_names_are_flagged(self):
        self.assertEqual(['ericl'], timeline.leaks('user ericl', extra=['ericl']))


class RecordTests(unittest.TestCase):
    def test_record_names_script_and_numbers(self):
        rec = timeline.record(revision='abc1234', model='m', goal='g', duration=24.84, fps=20,
                              gif_bytes=123, width=1280, height=720, cuts=[('model planning', 9.4)])
        self.assertEqual('alfredo-tui/recording/record.py', rec['script'])
        self.assertEqual(24.8, rec['duration_s'])
        self.assertEqual([{'what': 'model planning', 'seconds': 9.4}], rec['cuts'])
        self.assertEqual('abc1234', rec['revision'])


if __name__ == '__main__':
    unittest.main()
