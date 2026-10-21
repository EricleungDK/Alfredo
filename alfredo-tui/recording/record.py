#!/usr/bin/env python3
"""Reproduce the README recording of alfredo-tui: a real run, captured by VHS.

    python3 alfredo-tui/recording/record.py [--takes 4] [--out docs/assets]

Needs: vhs 0.12.1 + ttyd 1.7.7 on PATH, ffmpeg, Pillow, Ollama running with the model,
a release build (cargo build --release --locked --manifest-path alfredo-tui/Cargo.toml).
Everything shown is a neutral sandbox: HOME=/tmp/demo-home, prompt '$ ', sample repo
~/sample, author "Demo". Model waits are cut (VHS Hide/Show) and labelled here.
A take is kept only if autopilot finished with every task accepted after a repair.
"""
import argparse
import json
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(HERE))
import timeline  # noqa: E402

HOME = Path('/tmp/demo-home')
WORK = Path('/tmp/alfredo-demo-rec')
MODEL = 'qwen2.5-coder:14b'
# order of Hide..Show gaps in demo.tape; None = not labelled (goal typed unrecorded)
CUT_NAMES = [None, 'model planning', 'model coding']
LABEL_HOLD = 1.2
CAPTURE_FPS = 25
GIF_FPS = 20
SIZE = (1280, 720)


def sh(*cmd, **kw):
    return subprocess.run(cmd, check=True, **kw)


def prepare():
    if not subprocess.run(['fc-list', ':charset=2442'], capture_output=True, text=True).stdout.strip():
        sys.exit('no installed font has U+2442 (the repair icon); install one, e.g. copy Segoe UI Symbol into ~/.local/share/fonts, then fc-cache -f')
    shutil.rmtree(HOME, ignore_errors=True)
    (HOME / 'bin').mkdir(parents=True)
    binary = REPO / 'alfredo-tui/target/release/alfredo-tui'
    if not binary.exists():
        sh('cargo', 'build', '--release', '--locked', '--manifest-path', str(REPO / 'alfredo-tui/Cargo.toml'))
    shutil.copy(binary, HOME / 'bin/alfredo-tui')
    sh(str(HERE / 'sample_repo.sh'), str(HOME / 'sample'))
    shutil.rmtree(WORK, ignore_errors=True)
    WORK.mkdir(parents=True)
    # model loaded before timing, so the recording shows "warm" and no cold start
    body = json.dumps({'model': MODEL, 'keep_alive': '30m', 'prompt': '', 'stream': False}).encode()
    urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:11434/api/generate', body), timeout=300).read()


def run_vhs():
    tape = (HERE / 'demo.tape').read_text().replace('Output raw.mp4', f'Output "{WORK}/raw.mp4"\nOutput "{WORK}/raw.txt"')
    (WORK / 'demo.tape').write_text(tape)
    start = time.time()
    stamped = []
    proc = subprocess.Popen(['vhs', str(WORK / 'demo.tape')], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, cwd=WORK)
    for line in proc.stdout:
        stamped.append((time.time() - start, line.strip()))
    if proc.wait() != 0:
        print('\n'.join(c for _, c in stamped[-8:]))
        return None
    # VHS echoes each command as it runs; keep the ones that decide capture state
    return [(t, c.split()[0]) for t, c in stamped if c.split() and c.split()[0] in ('Hide', 'Show', 'Sleep', 'Type', 'Escape', 'Enter', 'Ctrl+Q') or c.startswith('Wait')]


def good_take():
    states = list(HOME.glob('state/rust-tasks-v1/*/tasks.json'))
    if not states:
        return False
    tasks = json.loads(states[0].read_text())['tasks']
    return timeline.keep_take(tasks, timeline.watch_id((HERE / 'demo.tape').read_text()))


def frames(src, dst):
    shutil.rmtree(dst, ignore_errors=True)
    dst.mkdir(parents=True)
    sh('ffmpeg', '-v', 'error', '-i', str(src), '-vf', f'scale={SIZE[0]}:{SIZE[1]}:flags=lanczos,fps={CAPTURE_FPS}', str(dst / 'f%05d.png'))
    return sorted(dst.glob('f*.png'))


def jump_diffs(files):
    from PIL import Image, ImageChops, ImageStat
    prev, out = None, []
    for f in files:
        im = Image.open(f).convert('L').resize((320, 180))
        out.append(0 if prev is None else ImageStat.Stat(ImageChops.difference(im, prev)).sum[0])
        prev = im
    return out


def label_frame(src, text, dst):
    from PIL import Image, ImageDraw, ImageFont
    im = Image.open(src).convert('RGB')
    font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf', 30)
    d = ImageDraw.Draw(im)
    w = d.textlength(text, font=font)
    x, y = (SIZE[0] - w) / 2, SIZE[1] - 110
    d.rounded_rectangle((x - 22, y - 12, x + w + 22, y + 52), radius=14, fill=(250, 200, 80))
    d.text((x, y), text, font=font, fill=(20, 20, 20))
    im.save(dst)


def assemble(stamped, out_dir):
    files = frames(WORK / 'raw.mp4', WORK / 'raw_frames')
    shown = timeline.shown_seconds(stamped)
    factor = timeline.speed(len(files) / CAPTURE_FPS, shown)
    print(f'capture ran at {factor:.2f}x real time; retimed to wall clock')
    idx = timeline.retime(len(files), factor)
    raw_diffs = jump_diffs(files)
    diffs = [raw_diffs[i] if k == 0 or i != idx[k - 1] else 0 for k, i in enumerate(idx)]
    final = WORK / 'final'
    shutil.rmtree(final, ignore_errors=True)
    final.mkdir()
    labelled = {}
    for (before, seconds), name in zip(timeline.cuts(stamped), CUT_NAMES):
        if name:
            labelled[timeline.snap(diffs, round(before * CAPTURE_FPS), CAPTURE_FPS)] = (name, seconds)
    n = 0
    for k, i in enumerate(idx):
        if k in labelled:
            name, seconds = labelled[k]
            for _ in timeline.label_frames(0, CAPTURE_FPS, LABEL_HOLD):
                label_frame(files[idx[k - 1]], timeline.label(name, seconds), final / f'f{n:05d}.png')
                n += 1
        shutil.copy(files[i], final / f'f{n:05d}.png')
        n += 1
    # poster: summary screen just before the TUI is left (last big jump)
    tail = max(range(len(diffs) - 8 * CAPTURE_FPS, len(diffs)), key=lambda k: diffs[k])
    shutil.copy(files[idx[tail - 3]], out_dir / 'tui-recording-poster.png')
    pattern = str(final / 'f%05d.png')
    base = ['ffmpeg', '-v', 'error', '-y', '-framerate', str(CAPTURE_FPS), '-i', pattern]
    sh(*base, '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-crf', '20', '-movflags', '+faststart', str(out_dir / 'tui-recording.mp4'))
    sh(*base, '-c:v', 'libvpx-vp9', '-b:v', '0', '-crf', '32', str(out_dir / 'tui-recording.webm'))
    vf = f'fps={GIF_FPS},split[a][b];[a]palettegen=stats_mode=diff:max_colors=64[p];[b][p]paletteuse=dither=none:diff_mode=rectangle'
    sh(*base, '-vf', vf, '-loop', '0', str(out_dir / 'tui-recording.gif'))
    return n / CAPTURE_FPS, [(name, seconds) for name, seconds in labelled.values()]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--takes', type=int, default=4)
    ap.add_argument('--reuse', action='store_true', help='re-encode the last capture')
    ap.add_argument('--out', default=str(REPO / 'docs/assets'))
    args = ap.parse_args()
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    if args.reuse:
        stamped = [tuple(x) for x in json.loads((WORK / 'stamps.json').read_text())]
    for take in range(1, 1 if args.reuse else args.takes + 1):
        prepare()
        stamped = run_vhs()
        ok = stamped is not None and good_take()
        print(f'take {take}: {"kept" if ok else "discarded (no repaired failure, or vhs error)"}')
        if ok:
            (WORK / 'stamps.json').write_text(json.dumps(stamped))
            break
    else:
        if not args.reuse:
            sys.exit('no usable take: autopilot did not repair a failure in any take')
    duration, cut_list = assemble(stamped, out_dir)
    gif = out_dir / 'tui-recording.gif'
    probe = json.loads(subprocess.check_output(['ffprobe', '-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=width,height', '-of', 'json', str(gif)]))['streams'][0]
    leaked = timeline.leaks((WORK / 'raw.txt').read_text(), extra=[])
    bad = timeline.problems(duration, GIF_FPS, gif.stat().st_size, probe['width'], probe['height']) + [f'leak {x}' for x in leaked]
    rev = subprocess.check_output(['git', '-C', str(REPO), 'rev-parse', '--short', 'HEAD'], text=True).strip()
    goal = [l for l in (HERE / 'demo.tape').read_text().splitlines() if l.startswith('Type "/go ')][0][len('Type "/go '):-1]
    rec = timeline.record(rev, MODEL, goal, duration, GIF_FPS, gif.stat().st_size, probe['width'], probe['height'], cut_list)
    (out_dir / 'tui-recording.json').write_text(json.dumps(rec, indent=2) + '\n')
    print(json.dumps(rec, indent=2))
    if bad:
        sys.exit('budget problems: ' + '; '.join(bad))


if __name__ == '__main__':
    main()
