"""第 2 段階の音を組み立てて、スキンのフォルダに書く。

    python3 tools/skin/voice.py <声のフォルダ>
    python3 tools/skin/ui_sounds.py <操作音のフォルダ>
    python3 tools/skin/build_stage2.py <声のフォルダ> <操作音のフォルダ> <Lyria の元のフォルダ> assets/skin

Lyria の元のフォルダには `lyria.py` で作った mp3 を、`sounds.json` の music の名前で置く（title.mp3 など）。
結果・開始・終了・読み込み中の音は `jingles.py` で切り出して声を重ねる。
ogg は Vorbis（品質 0.6 相当）、並べ替えの読み上げは上流と同じ wav（16 bit）で書く。
"""

import json
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

import jingles

SR = 48000
HERE = Path(__file__).resolve().parent


def load(path: Path) -> np.ndarray:
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    if sr != SR:
        raise SystemExit(f"{path}: {sr} Hz（{SR} Hz にしてから置く）")
    if x.shape[1] == 1:
        x = np.repeat(x, 2, axis=1)
    return x


def write(skin: Path, rel: str, x: np.ndarray) -> None:
    dst = skin / rel
    dst.parent.mkdir(parents=True, exist_ok=True)
    if dst.suffix == ".ogg":
        sf.write(dst, x, SR, format="OGG", subtype="VORBIS", compression_level=0.4)
    else:
        sf.write(dst, x, SR, subtype="PCM_16")
    print(rel, f"{len(x) / SR:.2f}s")


def mp3_48k(path: Path) -> np.ndarray:
    """mp3（44.1kHz）を読んで 48kHz にする"""
    from scipy.signal import resample_poly

    x, sr = sf.read(path, dtype="float32", always_2d=True)
    if sr != SR:
        g = np.gcd(sr, SR)
        x = resample_poly(x, SR // g, sr // g, axis=0).astype(np.float32)
    return x


def main() -> None:
    voice, ui, raw, skin = (Path(a) for a in sys.argv[1:5])
    cfg = json.loads((HERE / "sounds.json").read_text(encoding="utf-8"))
    for key, rel in cfg["voice"]["files"].items():
        write(skin, rel, load(voice / f"{key}.wav"))
    for key, rel in cfg["ui"]["files"].items():
        write(skin, rel, load(ui / f"{key}.wav"))
    m = cfg["music"]["title"]
    x = mp3_48k(raw / "title.mp3")
    n = int(SR * m["fade_out_sec"])
    x[-n:] *= (np.cos(np.linspace(0, np.pi / 2, n)) ** 2)[:, None]
    peak = float(np.abs(x).max())
    if peak > 0.98:
        x *= 0.98 / peak
    write(skin, "Sounds/Title.ogg", x)
    for j in cfg["jingles"]["cuts"].values():
        write(skin, j["file"], jingles.build(j, raw, voice).T)


if __name__ == "__main__":
    main()
