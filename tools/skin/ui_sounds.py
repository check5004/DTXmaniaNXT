"""操作音（移動・切り替え・決定・戻る）を合成する（第 2 段階、見本の「B きらっとした音」）。

    python3 tools/skin/ui_sounds.py <出力先のフォルダ>

FM 合成のベルのような短い音に、響きと左右の広がりを足す。乱数は音ごとに種を決めているので、何度作っても同じ音になる。
"""

import sys
from pathlib import Path

import numpy as np
import soundfile as sf
from pedalboard import Chorus, HighpassFilter, Limiter, Pedalboard, Reverb
from scipy import signal

SR = 48000


def t(d):
    return np.arange(int(SR * d)) / SR


def expenv(d, tau):
    return np.exp(-t(d) / tau)


def att(x, a=0.002, rel=0.012):
    """立ち上がりと終わりをなめらかにする（ぷつっという音を出さない）"""
    n = max(1, int(SR * a))
    e = np.ones(len(x))
    e[:n] = np.linspace(0, 1, n)
    r = min(len(x) // 3, int(SR * rel))
    e[len(x) - r :] *= np.cos(np.linspace(0, np.pi / 2, r)) ** 2
    return x * e


def bp(x, lo, hi, order=2):
    return signal.sosfilt(signal.butter(order, [lo, hi], "bandpass", fs=SR, output="sos"), x)


def lp(x, f, order=2):
    return signal.sosfilt(signal.butter(order, f, "lowpass", fs=SR, output="sos"), x)


def at(x, sec, total):
    o = np.zeros(int(SR * total))
    s = int(SR * sec)
    o[s : s + len(x)] += x[: len(o) - s]
    return o


def stereo(x, haas_ms=0.0, pan=0.0):
    left = x * np.sqrt(1 - pan)
    right = x * np.sqrt(1 + pan)
    if haas_ms:
        d = int(SR * haas_ms / 1000)
        right = np.concatenate([np.zeros(d), right])[: len(right)]
    return np.stack([left, right])


def finish(st, board, tail, peak_db=-3.0):
    st = np.concatenate([st, np.zeros((2, int(SR * tail)))], axis=1).astype(np.float32)
    y = board(st, SR)
    a = np.abs(y).max(0)
    idx = np.where(a > 10 ** (-66 / 20))[0]
    y = y[:, : idx[-1] + 1]
    n = int(SR * 0.01)
    y[:, -n:] *= np.linspace(1, 0, n)
    return y / np.abs(y).max() * 10 ** (peak_db / 20)


def fm(fc, ratio, index, d, tau, itau=None):
    tt = t(d)
    itau = itau or tau * 0.5
    mod = index * np.exp(-tt / itau) * np.sin(2 * np.pi * fc * ratio * tt)
    return att(np.sin(2 * np.pi * fc * tt + mod) * expenv(d, tau), 0.001)


def note(n):
    return 440 * 2 ** ((n - 69) / 12)


def move():
    x = fm(note(88), 3.5, 1.8, 0.12, 0.03) * 0.8 + fm(note(76), 1.0, 0.6, 0.12, 0.04) * 0.4
    return finish(stereo(x, haas_ms=0.5), Pedalboard([HighpassFilter(400), Reverb(0.25, 0.4, 0.14, 1.0, 1.0)]), 0.35)


def change():
    total = 0.25
    x = at(fm(note(86), 2.0, 1.5, 0.1, 0.035), 0.0, total) + at(fm(note(91), 2.0, 1.5, 0.14, 0.05), 0.055, total)
    return finish(stereo(x, haas_ms=0.7), Pedalboard([HighpassFilter(400), Reverb(0.3, 0.4, 0.16, 1.0, 1.0)]), 0.45)


def decide():
    rng = np.random.default_rng(7)
    total = 1.0
    x = np.zeros(int(SR * total))
    for i, n in enumerate((72, 76, 79, 84, 88)):
        x += at(fm(note(n), 3.0 if i % 2 else 2.0, 2.2, 0.7, 0.22, 0.05) * (0.9 - 0.08 * i), 0.022 * i, total)
    sub = at(att(np.sin(2 * np.pi * note(48) * t(0.25)) * expenv(0.25, 0.07)), 0.0, total) * 0.5
    noise = bp(rng.standard_normal(int(SR * 0.4)), 4000, 12000) * np.hanning(int(SR * 0.4)) * 0.12
    air = at(att(noise), 0.0, total)
    st = stereo(x * 0.7 + air) + stereo(x * 0.3, haas_ms=11) + stereo(sub)
    board = Pedalboard(
        [
            HighpassFilter(80),
            Chorus(rate_hz=0.8, depth=0.15, mix=0.25),
            Reverb(0.55, 0.35, 0.22, 0.9, 1.0),
            Limiter(-1),
        ]
    )
    return finish(st, board, 1.0)


def cancel():
    total = 0.4
    x = at(fm(note(79), 1.0, 1.2, 0.18, 0.06), 0.0, total) * 0.8 + at(fm(note(72), 1.0, 1.2, 0.3, 0.09), 0.07, total)
    return finish(stereo(lp(x, 5000), haas_ms=0.7), Pedalboard([HighpassFilter(200), Reverb(0.3, 0.55, 0.14, 1.0, 1.0)]), 0.5)


SOUNDS = {"move": move, "change": change, "decide": decide, "cancel": cancel}


def main() -> None:
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    for name, fn in SOUNDS.items():
        y = fn()
        sf.write(out / f"{name}.wav", y.T, SR)
        print(name, f"{y.shape[1] / SR:.2f}s")


if __name__ == "__main__":
    main()
