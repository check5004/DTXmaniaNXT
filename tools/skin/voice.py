"""読み上げの声を VOICEVOX で作る（第 2 段階）。

VOICEVOX のエンジンを 127.0.0.1:50021 で動かしておく。例:
    docker run -d --rm -p 50021:50021 voicevox/voicevox_engine:cpu-latest

    python3 tools/skin/voice.py <出力先のフォルダ>

`sounds.json` の `voice` の行ごとに、声だけの wav（48kHz・ステレオ・響きを足したもの）を書く。
"""

import io
import json
import sys
import urllib.parse
import urllib.request
from pathlib import Path

import numpy as np
import soundfile as sf
from pedalboard import Compressor, Delay, Gain, HighpassFilter, Limiter, Pedalboard, PeakFilter, Reverb

SR = 48000
BASE = "http://127.0.0.1:50021"
HERE = Path(__file__).resolve().parent
_open = urllib.request.build_opener(urllib.request.ProxyHandler({})).open


def tts(text: str, speaker: int, speed: float, pitch: float, inton: float) -> np.ndarray:
    q = json.load(
        _open(
            urllib.request.Request(
                f"{BASE}/audio_query?" + urllib.parse.urlencode({"text": text, "speaker": speaker}),
                method="POST",
            )
        )
    )
    q.update(
        speedScale=speed,
        pitchScale=pitch,
        intonationScale=inton,
        prePhonemeLength=0.03,
        postPhonemeLength=0.12,
        outputSamplingRate=SR,
        outputStereo=False,
    )
    req = urllib.request.Request(
        f"{BASE}/synthesis?speaker={speaker}",
        data=json.dumps(q).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    wav, sr = sf.read(io.BytesIO(_open(req).read()), dtype="float32")
    assert sr == SR
    return wav


# 響きは 2 通り: call は結果や開始の掛け声（広い響き）、ui は難易度や並べ替え（短い響き）
def fx(x: np.ndarray, kind: str) -> np.ndarray:
    st = np.stack([x, x])
    common = [
        HighpassFilter(100),
        PeakFilter(250, -2.0, 1.0),
        PeakFilter(3500, 3.0, 0.7),
        PeakFilter(9000, 1.5, 0.7),
        Compressor(threshold_db=-22, ratio=3.5, attack_ms=2, release_ms=80),
        Gain(5),
    ]
    if kind == "call":
        tail = [
            Delay(delay_seconds=0.11, feedback=0.15, mix=0.08),
            Reverb(room_size=0.45, damping=0.45, wet_level=0.16, dry_level=0.9, width=1.0),
        ]
        pad = 0.9
    else:
        tail = [Reverb(room_size=0.18, damping=0.6, wet_level=0.08, dry_level=0.95, width=0.8)]
        pad = 0.35
    board = Pedalboard(common + tail + [Limiter(threshold_db=-1.0, release_ms=60)])
    y = board(np.concatenate([st, np.zeros((2, int(SR * pad)), np.float32)], axis=1), SR)
    a = np.abs(y).max(axis=0)
    idx = np.where(a > 10 ** (-62 / 20))[0]
    y = y[:, : idx[-1] + 1]
    n = int(SR * 0.01)
    y[:, -n:] *= np.linspace(1, 0, n)
    return y


def main() -> None:
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    cfg = json.loads((HERE / "sounds.json").read_text(encoding="utf-8"))["voice"]
    for key, line in cfg["lines"].items():
        x = tts(line["text"], cfg["speaker"], line.get("speed", 1.0), line.get("pitch", 0.0), line.get("inton", 1.2))
        y = fx(x, line["fx"])
        sf.write(out / f"{key}.wav", y.T, SR)
        print(key, line["text"], f"{y.shape[1] / SR:.2f}s")


if __name__ == "__main__":
    main()
