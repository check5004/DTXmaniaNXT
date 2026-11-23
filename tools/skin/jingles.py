"""結果・開始・終了・読み込み中の音（ジングル）を組み立てる（第 2 段階、見本の「Y 今どきの電子音」）。

    python3 tools/skin/jingles.py <Lyria の元のフォルダ> <声のフォルダ> <出力先>

Lyria は数秒だけの音を頼んでも 30 秒前後の曲を返すので、`lyria.py` で作った mp3 から
`sounds.json` の `jingles` に書いた区間を切り出し、終わりを消して、声（`voice.py` の出力）を重ねる。
出力先に wav を書く（スキンへは `build_stage2.py` が ogg にして置く）。

音量は、曲を -14 LUFS にそろえ、声のあいだだけ曲を下げて声を重ね、全体を目標の LUFS にしてから
-1 dBFS を超えないように抑える。
"""

import json
import sys
from pathlib import Path

import numpy as np
import pyloudnorm as pyln
import soundfile as sf
from scipy.signal import resample_poly

SR = 48000
HERE = Path(__file__).resolve().parent
METER = pyln.Meter(SR)


def mp3_48k(path: Path) -> np.ndarray:
    """mp3（44.1kHz）を読んで、48kHz・ステレオ・(2, n) にする"""
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    if sr != SR:
        g = np.gcd(sr, SR)
        x = resample_poly(x, SR // g, sr // g, axis=0).astype(np.float32)
    if x.shape[1] == 1:
        x = np.repeat(x, 2, axis=1)
    return x.T.copy()


def loudness(x: np.ndarray) -> float:
    return METER.integrated_loudness(x.T)


def to_lufs(x: np.ndarray, target: float) -> np.ndarray:
    return x * 10 ** ((target - loudness(x)) / 20)


def limit(x: np.ndarray, ceiling_db: float = -1.0, look: float = 0.005, release: float = 0.08) -> np.ndarray:
    """先読みのリミッター。山の手前で少しずつ下げ、ゆっくり戻す（音を潰さず、天井だけ決める）"""
    c = 10 ** (ceiling_db / 20)
    peak = np.abs(x).max(axis=0)
    g = np.minimum(1.0, c / np.maximum(peak, 1e-9))
    n = max(1, int(SR * look))
    # i から n 先までの一番小さい値（山の手前から下げ始める）
    padded = np.concatenate([g, np.ones(n)])
    ahead = np.lib.stride_tricks.sliding_window_view(padded, n + 1).min(axis=1)[: len(g)]
    # n 個の平均でなめらかにする（山の位置では必ず ahead 以下になる）
    k = np.ones(n) / n
    smooth = np.convolve(np.concatenate([np.ones(n - 1), ahead]), k, mode="valid")
    # 戻りは release 秒で
    a = 1 - np.exp(-1 / (release * SR))
    out = np.empty_like(smooth)
    cur = 1.0
    for i, v in enumerate(smooth):
        cur = v if v < cur else cur + (v - cur) * a
        out[i] = cur
    return (x * out).astype(np.float32)


def cut(music: np.ndarray, start: float, end: float, fade_out: float) -> np.ndarray:
    y = music[:, int(start * SR) : int(end * SR)].copy()
    n = int(0.006 * SR)
    y[:, :n] *= np.linspace(0, 1, n)
    n = int(fade_out * SR)
    y[:, -n:] *= np.cos(np.linspace(0, np.pi / 2, n)) ** 2
    return y


def with_voice(music: np.ndarray, voice: np.ndarray, at: float, duck_db: float) -> np.ndarray:
    """声のあいだだけ曲を duck_db 下げて、声を重ねる"""
    a0 = int(at * SR)
    a1 = a0 + voice.shape[1]
    n = max(music.shape[1], a1)
    g = np.ones(n, np.float32)
    d = 10 ** (duck_db / 20)
    r = int(0.06 * SR)
    g[a0:a1] = d
    g[max(0, a0 - r) : a0] = np.linspace(1, d, a0 - max(0, a0 - r))
    e = min(n, a1 + r)
    g[a1:e] = np.linspace(d, 1, e - a1)
    out = np.zeros((2, n), np.float32)
    out[:, : music.shape[1]] = music * g[: music.shape[1]]
    out[:, a0:a1] += voice
    return out


def build(j: dict, raw: Path, voice: Path) -> np.ndarray:
    x = cut(mp3_48k(raw / f"{j['raw']}.mp3"), j["start"], j["end"], j["fade_out"])
    x = to_lufs(x, -14.0)
    if "voice" in j:
        v, sr = sf.read(voice / f"{j['voice']}.wav", dtype="float32", always_2d=True)
        assert sr == SR
        v = v.T if v.shape[1] == 2 else np.repeat(v.T, 2, axis=0)
        x = with_voice(x, v, j["voice_at"], j.get("duck_db", -5.0))
    return limit(to_lufs(x, j.get("lufs", -13.0)))


def main() -> None:
    raw, voice, out = (Path(a) for a in sys.argv[1:4])
    out.mkdir(parents=True, exist_ok=True)
    cfg = json.loads((HERE / "sounds.json").read_text(encoding="utf-8"))["jingles"]
    for key, j in cfg["cuts"].items():
        y = build(j, raw, voice)
        sf.write(out / f"{key}.wav", y.T, SR, subtype="FLOAT")
        print(key, f"{y.shape[1] / SR:.2f}s", f"{loudness(y):.1f} LUFS", f"peak {20 * np.log10(np.abs(y).max()):.1f} dB")


if __name__ == "__main__":
    main()
