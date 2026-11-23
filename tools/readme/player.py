"""README の撮影用: 譜面どおりにキーを送って「人が叩いた」プレイを作る（Xvfb の :99 で動くアプリへ XTest で送る）。

要るもの: python-xlib。アプリは 1280x720 の窓で、キーボードの割り当ては既定のまま。
例: python3 player.py "Neon Skyline/mstr.dtx" --start --lag -38 --jitter 14 --bias -6 --miss 24-27:0.4 --miss-rate 0.01 --seed 3

python3 player.py CHART.dtx [--start] [--lag MS] [--jitter MS] [--bias MS] [--miss M1-M2:P] [--miss-rate P] [--seed N]
- --start: 選曲の画面で Enter を送ってから始める
- 演奏の画面に変わった瞬間を画面の色で見て、曲の時計（-2000ms から始まる）を合わせる
"""

import argparse
import random
import re
import sys
import time

from Xlib import X, XK, display
from Xlib.ext import xtest

KEYS = {"11": "s", "12": "d", "13": "f", "14": "g", "15": "h", "16": "k", "17": "j", "18": "q", "19": "l", "1A": "a", "1B": "z", "1C": "z"}
LEAD_IN_MS = 2000.0


def parse(path):
    text = open(path, encoding="utf-8", errors="replace").read()
    bpm = float(re.search(r"^#BPM:?\s*([\d.]+)", text, re.M).group(1))
    lens = {}
    for m in re.finditer(r"^#(\d{3})02:\s*([\d.]+)", text, re.M):
        lens[int(m.group(1))] = float(m.group(2))
    notes = []
    for m in re.finditer(r"^#(\d{3})([0-9A-Z]{2}):\s*([0-9A-Za-z]+)", text, re.M):
        meas, ch, data = int(m.group(1)), m.group(2).upper(), m.group(3)
        if ch not in KEYS:
            continue
        n = len(data) // 2
        for i in range(n):
            if data[2 * i : 2 * i + 2] != "00":
                notes.append((meas, i / n, ch))
    starts = {}
    t = 0.0
    for meas in range(0, 1000):
        starts[meas] = t
        t += lens.get(meas, 1.0) * 4 * 60000 / bpm
    out = []
    for meas, frac, ch in notes:
        ms = starts[meas] + frac * lens.get(meas, 1.0) * 4 * 60000 / bpm
        out.append((ms, meas, ch))
    out.sort()
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("chart")
    ap.add_argument("--start", action="store_true")
    ap.add_argument("--lag", type=float, default=25.0)
    ap.add_argument("--jitter", type=float, default=10.0)
    ap.add_argument("--bias", type=float, default=0.0)
    ap.add_argument("--miss", action="append", default=[])
    ap.add_argument("--miss-rate", type=float, default=0.0)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--until", type=float, default=1e9, help="この ms で止める")
    ap.add_argument("--y1", type=int, default=250)
    ap.add_argument("--y2", type=int, default=450)
    ap.add_argument("--judge-y", type=int, default=564)
    a = ap.parse_args()
    rnd = random.Random(a.seed)
    notes = parse(a.chart)
    miss = []
    for spec in a.miss:
        r, p = spec.split(":")
        m1, m2 = (int(x) for x in r.split("-"))
        miss.append((m1, m2, float(p)))
    d = display.Display(":99")
    root = d.screen().root
    kc = {k: d.keysym_to_keycode(XK.string_to_keysym(k)) for k in set(KEYS.values())}
    ret = d.keysym_to_keycode(XK.string_to_keysym("Return"))

    def whites():
        img = root.get_image(36, 530, 64, 48, X.ZPixmap, 0xFFFFFFFF)
        data = img.data
        return sum(1 for i in range(0, len(data), 4) if data[i] > 200 and data[i + 1] > 200 and data[i + 2] > 200)

    if a.start:
        xtest.fake_input(d, X.KeyPress, ret)
        xtest.fake_input(d, X.KeyRelease, ret)
        d.sync()
    # 読み込みの画面（左下に白が無い）が 0.3 秒続くのを待ち、演奏の画面の難易度の箱（白い字）が出た瞬間を取る
    deadline = time.time() + 30
    calm = time.time()
    while time.time() < deadline:
        if whites() > 10:
            calm = time.time()
        elif time.time() - calm > 0.3:
            break
        time.sleep(0.005)
    while whites() <= 30 and time.time() < deadline:
        pass
    # 最初の音符（0 ms の左ペダル）がレーンを下りるのを 2 か所で見て、判定ラインに着く時刻を出す
    def band(y):
        img = root.get_image(424, y - 2, 40, 4, X.ZPixmap, 0xFFFFFFFF)
        data = img.data
        return sum(1 for i in range(0, len(data), 4) if max(data[i], data[i + 1], data[i + 2]) > 150)

    def cross(y):
        while time.time() < deadline:
            if band(y) > 20:
                return time.perf_counter()
        raise SystemExit("no chip")

    t1 = cross(a.y1)
    t2 = cross(a.y2)
    v = (a.y2 - a.y1) / (t2 - t1)
    t_judge = t2 + (a.judge_y - a.y2) / v
    t0 = t_judge + a.lag / 1000
    start_ms = 0.0
    print(f"chip speed {v:.0f} px/s, judge in {(t_judge - time.perf_counter()) * 1000:.0f} ms", file=sys.stderr)
    print(f"start detected, {len(notes)} notes", file=sys.stderr)
    pending = []
    for ms, meas, ch in notes:
        if ms > a.until:
            break
        if any(m1 <= meas <= m2 and rnd.random() < p for m1, m2, p in miss) or rnd.random() < a.miss_rate:
            continue
        pending.append((ms + a.bias + rnd.gauss(0, a.jitter), KEYS[ch]))
    pending.sort()
    held = []
    for ms, key in pending:
        target = t0 + (ms - start_ms) / 1000
        while True:
            now = time.perf_counter()
            rels = [h for h in held if h[0] <= now]
            for rel in rels:
                xtest.fake_input(d, X.KeyRelease, kc[rel[1]])
                held.remove(rel)
            if rels:
                d.sync()
            if now >= target - 0.0005:
                break
            time.sleep(min(0.001, max(0, target - now - 0.001)))
        for h in [h for h in held if h[1] == key]:
            xtest.fake_input(d, X.KeyRelease, kc[key])
            held.remove(h)
        xtest.fake_input(d, X.KeyPress, kc[key])
        d.sync()
        held.append((time.perf_counter() + 0.025, key))
    time.sleep(0.05)
    for _, key in held:
        xtest.fake_input(d, X.KeyRelease, kc[key])
    d.sync()


if __name__ == "__main__":
    main()
