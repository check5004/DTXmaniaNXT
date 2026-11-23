#!/usr/bin/env python3
"""スキンの絵を OpenRouter の画像生成で 1 枚作る（費用がかかる）。

使い方: OPENROUTER_API_KEY=... python3 tools/skin/generate.py <名前> <出力.png>
<名前> は prompts.json の name。同じ指示文でも毎回ちがう絵になる。
出力の横に、送った内容と費用（usage.cost）を .json で残す。
"""
import base64, json, os, sys, time, urllib.error, urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))


def main():
    name, out = sys.argv[1], sys.argv[2]
    spec = next(p for p in json.load(open(os.path.join(HERE, "prompts.json"), encoding="utf-8")) if p["name"] == name)
    body = {
        "model": spec["model"],
        "prompt": spec["prompt"],
        "n": 1,
        "size": spec["size"],
        "quality": spec["quality"],
        "output_format": "png",
    }
    if spec.get("background") == "transparent":
        body["background"] = "transparent"
    req = urllib.request.Request(
        "https://openrouter.ai/api/v1/images",
        data=json.dumps(body).encode(),
        headers={"Authorization": "Bearer " + os.environ["OPENROUTER_API_KEY"], "Content-Type": "application/json"},
        method="POST",
    )
    t = time.time()
    try:
        with urllib.request.urlopen(req, timeout=600) as r:
            res = json.load(r)
    except urllib.error.HTTPError as e:
        sys.exit(f"HTTP {e.code}: {e.read().decode()[:2000]}")
    with open(out, "wb") as f:
        f.write(base64.b64decode(res["data"][0]["b64_json"]))
    meta = {"request": body, "usage": res.get("usage"), "seconds": round(time.time() - t, 1)}
    with open(out + ".json", "w", encoding="utf-8") as f:
        json.dump(meta, f, ensure_ascii=False, indent=1)
    print(out, (res.get("usage") or {}).get("cost"), "USD")


if __name__ == "__main__":
    main()
