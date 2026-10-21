"""Pure helpers for the README recording (no I/O): cuts, labels, budgets, leak scan."""

DASH = '—'
LEAK_MARKERS = ('/home/', '/mnt/c')


def cuts(stamped):
    """[(shown_before_s, cut_s)] from (wall_s, vhs_command) lines.

    VHS stops capturing between Hide and Show, so the cut is the wall time of a
    Hide..Show pair and its position is the time shown so far. A Hide before
    anything was shown (setup) is not a cut.
    """
    out, shown, hidden_at, last = [], 0.0, None, None
    for at, cmd in stamped:
        if hidden_at is None:
            if last is not None:
                shown += at - last
            if cmd == 'Hide':
                hidden_at = at
        elif cmd == 'Show':
            if shown > 0:
                out.append((round(shown, 1), round(at - hidden_at, 1)))
            hidden_at = None
        last = at
    return out


def shown_seconds(stamped):
    """Wall-clock seconds VHS was capturing (after the first Show, minus Hide..Show gaps)."""
    total, hidden, last = 0.0, False, None
    for at, cmd in stamped:
        if last is not None and not hidden:
            total += at - last
        if cmd == 'Hide':
            hidden = True
        elif cmd == 'Show':
            hidden = False
        last = at
    return round(total, 3)


def speed(video_s, shown_s):
    """VHS encodes captured frames at a fixed fps but captures slower than that at
    high resolution, so the raw video runs at this fraction of real time."""
    return round(video_s / shown_s, 3)


def retime(frames, factor):
    """Raw frame index for each real-time output frame."""
    return [min(int(k * factor), frames - 1) for k in range(round(frames / factor))]


def label(what, seconds):
    return f'{what} {DASH} {round(seconds)} s cut'


def snap(diffs, estimate, window):
    """Index of the largest frame-to-frame jump within +-window of estimate."""
    lo, hi = max(0, estimate - window), min(len(diffs), estimate + window + 1)
    return max(range(lo, hi), key=lambda i: (diffs[i], -abs(i - estimate)))


def label_frames(start, fps, hold):
    return range(start, start + round(fps * hold))


def problems(duration, fps, gif_bytes, width, height):
    out = []
    if not 15 <= duration <= 30:
        out.append(f'duration {duration:.1f} s outside 15-30 s')
    if not 15 <= fps <= 25:
        out.append(f'fps {fps} outside 15-25')
    if gif_bytes >= 5_000_000:
        out.append(f'GIF {gif_bytes} B is not under 5 MB')
    if width < 1280:
        out.append(f'width {width} below 1280')
    return out


def leaks(text, extra):
    return [m for m in (*LEAK_MARKERS, *extra) if m in text]


def record(revision, model, goal, duration, fps, gif_bytes, width, height, cuts):
    return {
        'script': 'alfredo-tui/recording/record.py',
        'revision': revision,
        'model': model,
        'goal': goal,
        'duration_s': round(duration, 1),
        'fps': fps,
        'gif_bytes': gif_bytes,
        'width': width,
        'height': height,
        'cuts': [{'what': w, 'seconds': s} for w, s in cuts],
    }
