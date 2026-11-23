"""曲とジングルの元を Google Lyria 3 で作る（OpenRouter 経由、第 2 段階）。

    OPENROUTER_API_KEY=... python3 tools/skin/lyria.py <sounds.json の music の名前> <出力.mp3>

1 回 0.04 ドル（30 秒前後、192kbps の mp3）。音を返させるには `stream: true` が要る。
使った費用は出力と同じ名前の .json に書く（`usage.cost`）。
"""

import base64
import json
import os
import sys
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main() -> None:
    name, out = sys.argv[1], Path(sys.argv[2])
    entry = json.loads((HERE / "sounds.json").read_text(encoding="utf-8"))["music"][name]
    body = {
        "model": entry["model"],
        "messages": [{"role": "user", "content": entry["prompt"]}],
        "modalities": ["text", "audio"],
        "stream": True,
        "audio": {"format": "wav"},
    }
    req = urllib.request.Request(
        "https://openrouter.ai/api/v1/chat/completions",
        data=json.dumps(body).encode(),
        headers={
            "Authorization": "Bearer " + os.environ["OPENROUTER_API_KEY"],
            "Content-Type": "application/json",
        },
    )
    chunks, usage = [], None
    with urllib.request.urlopen(req, timeout=600) as resp:
        for raw in resp:
            line = raw.decode().strip()
            if not line.startswith("data:"):
                continue
            data = line[5:].strip()
            if data == "[DONE]":
                break
            j = json.loads(data)
            usage = j.get("usage") or usage
            for c in j.get("choices", []):
                audio = c.get("delta", {}).get("audio") or {}
                if audio.get("data"):
                    chunks.append(audio["data"])
    out.write_bytes(b"".join(base64.b64decode(c) for c in chunks))
    meta = {"name": name, "model": entry["model"], "prompt": entry["prompt"], "usage": usage}
    out.with_suffix(".json").write_text(json.dumps(meta, ensure_ascii=False, indent=1), encoding="utf-8")
    print(out, (usage or {}).get("cost"))


if __name__ == "__main__":
    main()
