"""README の図（アニメーションする SVG）を作る。

使い方: python3 tools/readme/make_svgs.py
- docs/readme/<名前>.svg（英語）と docs/readme/<名前>.ja.svg（日本語）を書く
- 標準ライブラリだけで動く。動きは CSS の keyframes（GitHub の README でも動く）
- 「動きを減らす」設定の人には止めた絵を見せる（prefers-reduced-motion）
"""

from __future__ import annotations

import math
import random
from pathlib import Path

OUT = Path(__file__).resolve().parents[2] / "docs" / "readme"
# 更新の絵に出す版の見本（実際の版とは関係ない）
SAMPLE_VERSION = "v0.9.0"

# DTX Web Editor の README と同じ色（レーンの色・背景）
LANE = {
    "LC": "#FF5C8A",
    "HH": "#4FC3F7",
    "LP": "#F48FB1",
    "SD": "#FFD54F",
    "HT": "#66BB6A",
    "BD": "#9C84E0",
    "LT": "#EF5350",
    "FT": "#FF9800",
    "CY": "#42A5F5",
    "RD": "#80DEEA",
}
LANES = list(LANE)
CYAN = "#5CE1E6"
MAGENTA = "#C77DFF"
GREEN = "#7CF29A"
YELLOW = "#FFD54F"
ORANGE = "#FFA94D"
RED = "#FF6B6B"
INK = "#E8EEF7"
SUB = "#9AA8BC"
DIM = "#5B6B82"

FONTS = {
    "en": "'Segoe UI','Inter','Helvetica Neue',Arial,'Hiragino Sans','Yu Gothic UI','Meiryo','Noto Sans JP',system-ui,sans-serif",
    "ja": "'Hiragino Sans','Hiragino Kaku Gothic ProN','Yu Gothic UI','Yu Gothic','Meiryo','Noto Sans JP','Noto Sans CJK JP',system-ui,sans-serif",
}
MONO = "'JetBrains Mono','SF Mono','Cascadia Mono',Consolas,Menlo,monospace"


def esc(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


class Svg:
    """1 枚の SVG。keyframes は名前ごとに 1 回だけ書く"""

    def __init__(self, lang: str, w: int, h: int, title: str, desc: str):
        self.lang, self.w, self.h = lang, w, h
        self.title, self.desc = title, desc
        self.css: list[str] = []
        self.defs: list[str] = []
        self.body: list[str] = []
        self._n = 0

    def uid(self, p: str = "a") -> str:
        self._n += 1
        return f"{p}{self._n}"

    def add(self, s: str) -> None:
        self.body.append(s)

    def window(self, period: float, spans: list[tuple[float, float]], fade: float = 0.12) -> str:
        """period 秒のうち spans の間だけ見せるクラスを作って名前を返す（fade=0 なら切り替えるだけ）"""
        name = self.uid("w")
        if fade <= 0:
            frames = ["0%{opacity:0}"]
            for a, b in spans:
                frames.append(f"{a / period * 100:.3f}%{{opacity:1}}")
                frames.append(f"{min(b, period) / period * 100:.3f}%{{opacity:0}}")
            frames.append("100%{opacity:0}")
            self.css.append(f"@keyframes {name}{{{''.join(frames)}}}")
            self.css.append(f".{name}{{opacity:0;animation:{name} {period}s step-end infinite}}")
            return name
        pts: list[tuple[float, float]] = [(0.0, 0.0)]
        for a, b in spans:
            a0, b1 = max(a - fade, 0.0), min(b + fade, period)
            pts += [(a0, 0.0), (a, 1.0), (b, 1.0), (b1, 0.0)]
        pts.append((period, 0.0))
        frames = [f"{t / period * 100:.3f}%{{opacity:{v:g}}}" for t, v in pts]
        self.css.append(f"@keyframes {name}{{{''.join(frames)}}}")
        self.css.append(f".{name}{{opacity:0;animation:{name} {period}s linear infinite}}")
        return name

    def render(self) -> str:
        font = FONTS[self.lang]
        css = "\n".join(
            [
                f"text{{font-family:{font}}}",
                f".mono{{font-family:{MONO}}}",
                ".fb{transform-box:fill-box;transform-origin:50% 50%}",
                *self.css,
                "@media (prefers-reduced-motion:reduce){*{animation:none!important}}",
            ]
        )
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{self.w}" height="{self.h}" '
            f'viewBox="0 0 {self.w} {self.h}" role="img" aria-labelledby="title desc">\n'
            f'<title id="title">{esc(self.title)}</title>\n<desc id="desc">{esc(self.desc)}</desc>\n'
            f"<style>\n{css}\n</style>\n<defs>{''.join(self.defs)}</defs>\n" + "\n".join(self.body) + "\n</svg>\n"
        )


def card_bg(s: Svg, x=0, y=0, w=None, h=None, r=18, glow=None) -> None:
    w = s.w if w is None else w
    h = s.h if h is None else h
    gid = s.uid("bg")
    s.defs.append(
        f'<linearGradient id="{gid}" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#0B1018"/>'
        f'<stop offset="1" stop-color="#151B2E"/></linearGradient>'
    )
    s.add(f'<rect x="{x + 1}" y="{y + 1}" width="{w - 2}" height="{h - 2}" rx="{r}" fill="url(#{gid})" stroke="#263247"/>')
    if glow:
        rid = s.uid("rg")
        s.defs.append(
            f'<radialGradient id="{rid}" cx="{glow[0]}" cy="{glow[1]}" r="0.6"><stop offset="0" stop-color="{glow[2]}" stop-opacity="0.22"/>'
            f'<stop offset="1" stop-color="{glow[2]}" stop-opacity="0"/></radialGradient>'
        )
        s.add(f'<rect x="{x + 1}" y="{y + 1}" width="{w - 2}" height="{h - 2}" rx="{r}" fill="url(#{rid})"/>')


def text(x, y, s, size=14, fill=INK, weight=400, anchor="start", cls="", extra="") -> str:
    c = f' class="{cls}"' if cls else ""
    return (
        f'<text x="{x}" y="{y}" font-size="{size}" fill="{fill}" font-weight="{weight}" '
        f'text-anchor="{anchor}"{c}{extra}>{esc(s)}</text>'
    )


def pill(s: Svg, x, y, label, color, size=13) -> float:
    w = len(label) * size * (0.62 if s.lang == "en" else 1.0) + 34
    s.add(f'<rect x="{x}" y="{y}" width="{w:.0f}" height="28" rx="14" fill="{color}" fill-opacity="0.12" stroke="{color}" stroke-opacity="0.55"/>')
    s.add(f'<circle cx="{x + 14}" cy="{y + 14}" r="4" fill="{color}"/>')
    s.add(text(x + 25, y + 19, label, size, INK, 500))
    return w


# ---------------------------------------------------------------- hero


def hero(lang: str) -> Svg:
    en = lang == "en"
    s = Svg(
        lang,
        960,
        420,
        "DTXmaniaNXT",
        "A drum game and practice app for DTX charts. Notes fall into the NX-style lanes and are judged as you hit."
        if en
        else "DTX の譜面で遊べるドラムのゲーム兼練習アプリ。NX と同じレーンに音符が落ちてきて、叩くと判定が出る。",
    )
    card_bg(s, glow=("0.82", "0.35", MAGENTA))
    # 題
    gid = s.uid("tg")
    s.defs.append(
        f'<linearGradient id="{gid}" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{CYAN}"/>'
        f'<stop offset="1" stop-color="{MAGENTA}"/></linearGradient>'
    )
    s.add(f'<text x="52" y="122" font-size="58" font-weight="800" fill="{INK}" letter-spacing="-1">DTXmania<tspan fill="url(#{gid})">NXT</tspan></text>')
    if en:
        s.add(text(54, 160, "Play DTX drum charts.", 25, INK, 700))
        s.add(text(54, 192, "Practice the parts you miss.", 25, INK, 700))
        s.add(text(54, 226, "Written from scratch in Rust, with the DTXManiaNX v2.0 look.", 15, SUB))
        s.add(text(54, 248, "Made for electronic drums. Windows first, macOS and Linux too.", 15, SUB))
        tags = [("E-drums (MIDI)", CYAN), ("Practice mode", GREEN), ("Timing tools", YELLOW), ("Free", MAGENTA)]
    else:
        s.add(text(54, 160, "DTX の譜面で遊んで、", 25, INK, 700))
        s.add(text(54, 192, "苦手な所を練習する。", 25, INK, 700))
        s.add(text(54, 226, "Rust で一から書いたアプリ。見た目は DTXManiaNX v2.0 のまま。", 15, SUB))
        s.add(text(54, 248, "電子ドラムのためのアプリ。Windows を主に、macOS・Linux でも。", 15, SUB))
        tags = [("電子ドラム（MIDI）", CYAN), ("練習モード", GREEN), ("ずれを合わせる", YELLOW), ("無料", MAGENTA)]
    x = 52
    for label, col in tags:
        x += pill(s, x, 274, label, col) + 10
    for i, name in enumerate(LANES):
        s.add(f'<rect x="{54 + i * 26}" y="338" width="20" height="6" rx="3" fill="{LANE[name]}"/>')
    s.add(text(54 + 10 * 26 + 10, 345, "10 lanes · drums only" if en else "10 レーン・ドラム専用", 13, SUB))

    # 右: NX 風のレーン
    lx, lw, top, judge = 600, 31, 34, 330
    period = 4.8
    beat = 0.6
    s.add(f'<rect x="{lx - 6}" y="{top - 8}" width="{lw * 10 + 12}" height="{judge - top + 64}" rx="10" fill="#05070C" stroke="#20293A"/>')
    for i, name in enumerate(LANES):
        x0 = lx + i * lw
        s.add(f'<rect x="{x0}" y="{top}" width="{lw - 1}" height="{judge - top}" fill="{LANE[name]}" fill-opacity="0.05"/>')
    for k in range(5):
        y = judge - k * 74
        s.add(f'<line x1="{lx}" y1="{y}" x2="{lx + lw * 10}" y2="{y}" stroke="#2A3448" stroke-width="1"/>')
    # 叩く譜面（2 小節）
    hits: list[tuple[float, str]] = []
    for k in range(16):
        hits.append((k * beat / 2, "HH"))
    for t in (0, 1.5, 2.4, 3.9):
        hits.append((t, "BD"))
    for t in (0.6, 1.8, 3.0, 4.2):
        hits.append((t, "SD"))
    hits += [(0.0, "CY"), (2.4, "LC"), (3.9, "HT"), (4.05, "LT"), (4.2, "FT"), (4.35, "FT")]
    hits = [(t % period, n) for t, n in hits]
    fall = 1.4
    s.css.append(
        f"@keyframes fall{{0%{{transform:translateY(0);opacity:1}}{fall / period * 100:.3f}%{{transform:translateY({judge - top - 6}px);opacity:1}}"
        f"{fall / period * 100 + 0.6:.3f}%{{transform:translateY({judge - top - 6}px);opacity:0}}100%{{transform:translateY({judge - top - 6}px);opacity:0}}}}"
    )
    s.css.append(f".fa{{animation:fall {period}s linear infinite}}")
    s.css.append(f"@keyframes blip{{0%{{opacity:.9}}8%{{opacity:0}}100%{{opacity:0}}}}.bl{{opacity:0;animation:blip {period}s ease-out infinite}}")
    s.css.append(
        f"@keyframes pop{{0%{{opacity:1;transform:scale(1.25)}}2%{{transform:scale(1)}}4%{{opacity:1}}5.5%{{opacity:0}}100%{{opacity:0}}}}"
        f".pp{{opacity:0;animation:pop {period}s ease-out infinite;transform-box:fill-box;transform-origin:50% 50%}}"
    )
    for t, name in sorted(hits):
        i = LANES.index(name)
        x0 = lx + i * lw
        delay = (t - fall) % period - period
        s.add(
            f'<rect class="fa" style="animation-delay:{delay:.3f}s" x="{x0 + 3}" y="{top}" width="{lw - 7}" height="7" rx="2" '
            f'fill="{LANE[name]}"/>'
        )
    for t in sorted({t for t, _ in hits}):
        lanes_at = [n for tt, n in hits if tt == t]
        for name in lanes_at:
            i = LANES.index(name)
            s.add(
                f'<rect class="bl" style="animation-delay:{t - period:.3f}s" x="{lx + i * lw}" y="{judge - 120}" width="{lw - 1}" height="120" '
                f'fill="{LANE[name]}" fill-opacity="0.35"/>'
            )
    s.add(f'<rect x="{lx - 4}" y="{judge - 2}" width="{lw * 10 + 8}" height="4" rx="2" fill="#FFFFFF" fill-opacity="0.85"/>')
    for i, name in enumerate(LANES):
        cx = lx + i * lw + lw / 2
        s.add(f'<ellipse cx="{cx}" cy="{judge + 26}" rx="12" ry="9" fill="#141B28" stroke="{LANE[name]}" stroke-width="2.5"/>')
    # 判定の文字（叩くたびに出す）
    gid2 = s.uid("pg")
    s.defs.append(
        f'<linearGradient id="{gid2}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFF6B0"/>'
        f'<stop offset="1" stop-color="#FFB300"/></linearGradient>'
    )
    times = sorted({t for t, _ in hits})
    for t in times:
        s.add(
            f'<text class="pp" style="animation-delay:{t - period:.3f}s" x="{lx + lw * 5}" y="{judge - 150}" font-size="30" '
            f'font-weight="800" font-style="italic" fill="url(#{gid2})" text-anchor="middle" stroke="#3A2400" stroke-width="1">PERFECT</text>'
        )
    # コンボの数（叩くたびに 1 つ増える）
    for k, t in enumerate(times):
        nxt = times[k + 1] if k + 1 < len(times) else period
        cls = s.window(period, [(t, nxt - 0.001)], fade=0.0)
        s.add(f'<text class="{cls}" x="{lx + lw * 5}" y="{judge - 200}" font-size="22" font-weight="800" fill="{CYAN}" text-anchor="middle" fill-opacity="0.9">{100 + k} COMBO</text>')
    return s


# ---------------------------------------------------------------- download


def download(lang: str) -> Svg:
    en = lang == "en"
    label = "Download for Windows" if en else "Windows 版をダウンロード"
    s = Svg(lang, 384, 64, label, "Downloads DTXmaniaNXT-Setup.exe" if en else "DTXmaniaNXT-Setup.exe をダウンロードする")
    gid = s.uid("bt")
    s.defs.append(
        f'<linearGradient id="{gid}" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{CYAN}"/>'
        f'<stop offset="1" stop-color="{MAGENTA}"/></linearGradient>'
        '<linearGradient id="sh" x1="0" x2="1"><stop offset="0" stop-color="#fff" stop-opacity="0"/>'
        '<stop offset="0.5" stop-color="#fff" stop-opacity="0.45"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient>'
        '<clipPath id="bc"><rect x="2" y="2" width="380" height="60" rx="30"/></clipPath>'
    )
    s.css.append("@keyframes shine{0%,55%{transform:translateX(-140px)}85%,100%{transform:translateX(380px)}}.shine{animation:shine 4.5s ease-in-out infinite}")
    s.add(f'<rect x="2" y="2" width="380" height="60" rx="30" fill="url(#{gid})"/>')
    s.add('<g clip-path="url(#bc)"><rect class="shine" x="0" y="0" width="90" height="64" fill="url(#sh)" transform="skewX(-20)"/></g>')
    s.add('<circle cx="38" cy="32" r="16" fill="#07131D" fill-opacity="0.85"/>')
    s.add(f'<path d="M38 22 v13 M32 30 l6 6 l6 -6 M30 40 h16" stroke="{CYAN}" stroke-width="2.6" fill="none" stroke-linecap="round" stroke-linejoin="round"/>')
    s.add(text(64, 40, label, 19 if en else 18, "#06121B", 800))
    w = 52 if en else 50
    s.add(f'<rect x="{366 - w}" y="20" width="{w}" height="24" rx="12" fill="#06121B" fill-opacity="0.82"/>')
    s.add(text(366 - w / 2, 36.5, "Free" if en else "無料", 13, CYAN, 700, "middle"))
    return s


# ---------------------------------------------------------------- steps


def steps(lang: str) -> Svg:
    en = lang == "en"
    s = Svg(
        lang,
        960,
        250,
        "3 steps to play" if en else "遊ぶまでの 3 ステップ",
        "1. Download and run Setup.exe 2. Add your song folder 3. Plug in your drums and play"
        if en
        else "1. Setup.exe を入れる 2. 曲のフォルダを足す 3. 電子ドラムをつないで遊ぶ",
    )
    period = 6.0
    cards = [
        ("Install" if en else "入れる", ["Run DTXmaniaNXT-Setup.exe.", "No admin rights, no NX needed."] if en else ["Setup.exe を実行するだけ。", "管理者の権限も NX も要りません。"]),
        ("Add your songs" if en else "曲を足す", ["Pick your DTX song folder", "or drop it on the window."] if en else ["DTX の曲のフォルダを選ぶか、", "窓にドロップします。"]),
        ("Plug in and play" if en else "つないで遊ぶ", ["Connect your e-drum by USB-MIDI.", "Found automatically."] if en else ["電子ドラムを USB-MIDI で", "つなげば、自動で見つけます。"]),
    ]
    for i, (title, lines) in enumerate(cards):
        x = 8 + i * 318
        card_bg(s, x, 6, 306, 238, 16)
        # 絵の場所
        s.add(f'<rect x="{x + 16}" y="22" width="274" height="122" rx="10" fill="#0E1522" stroke="#1F2A3D"/>')
        hl = s.window(period, [(i * 2.0, i * 2.0 + 2.0)], fade=0.25)
        s.add(f'<rect class="{hl}" x="{x + 2}" y="8" width="302" height="234" rx="15" fill="none" stroke="{CYAN}" stroke-width="2"/>')
        cx, cy = x + 153, 83
        if i == 0:
            # インストーラーと進み具合
            s.add(f'<rect x="{cx - 70}" y="{cy - 40}" width="140" height="84" rx="8" fill="#121B2B" stroke="#2C3A52"/>')
            s.add(f'<rect x="{cx - 70}" y="{cy - 40}" width="140" height="18" rx="8" fill="#1B2638"/>')
            for k, c in enumerate(("#FF6B6B", "#FFD54F", "#66BB6A")):
                s.add(f'<circle cx="{cx - 60 + k * 10}" cy="{cy - 31}" r="3" fill="{c}"/>')
            s.add(text(cx, cy - 2, "DTXmaniaNXT-Setup.exe", 10, SUB, 600, "middle", "mono"))
            s.add(f'<rect x="{cx - 52}" y="{cy + 14}" width="104" height="10" rx="5" fill="#0A0F18" stroke="#2C3A52"/>')
            s.css.append(f"@keyframes prog{{0%{{width:0}}25%{{width:100px}}100%{{width:100px}}}}.pg{{animation:prog {period}s ease-in-out infinite}}")
            s.add(f'<rect class="pg" x="{cx - 50}" y="{cy + 16}" height="6" rx="3" width="0" fill="{CYAN}"/>')
            ok = s.window(period, [(1.6, period - 0.2)], fade=0.2)
            s.add(f'<g class="{ok}"><circle cx="{cx + 58}" cy="{cy - 40}" r="13" fill="{GREEN}"/><path d="M{cx + 52} {cy - 40} l4 4 l8 -8" stroke="#06121B" stroke-width="3" fill="none" stroke-linecap="round"/></g>')
        elif i == 1:
            # フォルダが窓に落ちる
            s.add(f'<rect x="{cx - 78}" y="{cy - 30}" width="156" height="72" rx="8" fill="none" stroke="#3A4A66" stroke-dasharray="5 5"/>')
            s.add(text(cx, cy + 26, "DTXFiles", 11, SUB, 600, "middle", "mono"))
            s.css.append(
                f"@keyframes drop{{0%,30%{{transform:translate(70px,-46px);opacity:0}}38%{{opacity:1}}55%,85%{{transform:translate(0,0);opacity:1}}95%,100%{{transform:translate(0,0);opacity:0}}}}"
                f".dp{{animation:drop {period}s ease-in-out infinite}}"
            )
            s.add(
                f'<g class="dp"><path d="M{cx - 26} {cy - 14} h18 l6 6 h28 v32 h-52 z" fill="{YELLOW}" stroke="#B8901C"/>'
                f'<path d="M{cx - 26} {cy - 4} h52 v22 h-52 z" fill="#FFE38A"/></g>'
            )
            n = s.window(period, [(3.4, period - 0.3)], fade=0.2)
            s.add(f'<g class="{n}"><rect x="{cx + 30}" y="{cy - 46}" width="64" height="22" rx="11" fill="{GREEN}"/>' + text(cx + 62, cy - 31, "+6 songs" if en else "6 曲", 12, "#06121B", 800, "middle") + "</g>")
        else:
            # パッドと MIDI の波
            for k, (name, dx, dy, r) in enumerate((("HH", -62, -8, 18), ("SD", -22, 18, 20), ("HT", 6, -18, 16), ("FT", 52, 18, 20), ("CY", 62, -26, 20))):
                s.add(f'<circle cx="{cx + dx}" cy="{cy + dy}" r="{r}" fill="#141B28" stroke="{LANE[name]}" stroke-width="3"/>')
                t0 = 4.2 + k * 0.28
                b = s.window(period, [(t0, t0 + 0.18)], fade=0.1)
                s.add(f'<circle class="{b}" cx="{cx + dx}" cy="{cy + dy}" r="{r}" fill="{LANE[name]}" fill-opacity="0.75"/>')
            s.add(f'<path d="M{cx - 118} {cy + 44} q 40 -10 70 -18" stroke="#3A4A66" stroke-width="3" fill="none"/>')
            s.add(text(cx - 118, cy + 58, "USB-MIDI", 10, SUB, 700, "start", "mono"))
        # 番号と文
        s.add(f'<circle cx="{x + 34}" cy="178" r="14" fill="none" stroke="{CYAN}" stroke-width="2"/>')
        s.add(text(x + 34, 183, str(i + 1), 14, CYAN, 800, "middle"))
        s.add(text(x + 58, 184, title, 19, INK, 800))
        for k, line in enumerate(lines):
            s.add(text(x + 22, 210 + k * 19, line, 13, SUB))
    return s


# ---------------------------------------------------------------- timing


def timing(lang: str) -> Svg:
    en = lang == "en"
    s = Svg(
        lang,
        960,
        300,
        "Judged by when the hit arrives" if en else "叩いた時刻で判定する",
        "Each MIDI hit is stamped the moment it arrives and compared with where the music is playing, not rounded to the next screen frame."
        if en
        else "MIDI の打撃は届いた瞬間の時刻を付け、曲の音の再生位置と比べて判定する。画面のフレームに丸めない。",
    )
    card_bg(s)
    period = 4.0
    ms = 7.0  # 1 ms の幅
    cx = 520
    hit_ms = 3
    s.add(text(40, 46, "Timing is measured, not guessed" if en else "画面のフレームではなく、叩いた時刻で判定", 22, INK, 800))
    s.add(
        text(
            40,
            72,
            "Each MIDI hit gets a timestamp on arrival and is compared with the audio playback position."
            if en
            else "MIDI の打撃は届いた瞬間に時刻を付け、曲の音がいま鳴っている位置と比べます。",
            14,
            SUB,
        )
    )
    rows = [
        (130, "If judged per frame" if en else "フレームで見る場合", False),
        (226, "DTXmaniaNXT", True),
    ]
    frame_ms = 1000 / 60
    for y, label, ours in rows:
        s.add(text(40, y + 5, label, 15, CYAN if ours else SUB, 700))
        x0, x1 = cx - 40 * ms, cx + 40 * ms
        s.add(f'<line x1="{x0}" y1="{y}" x2="{x1}" y2="{y}" stroke="#33415A" stroke-width="2"/>')
        # 判定の幅（PERFECT ±34 の一部）
        s.add(f'<rect x="{cx - 34 * ms}" y="{y - 14}" width="{68 * ms}" height="28" rx="6" fill="{GREEN}" fill-opacity="0.06"/>')
        s.add(f'<line x1="{cx}" y1="{y - 22}" x2="{cx}" y2="{y + 22}" stroke="{INK}" stroke-width="2"/>')
        s.add(text(cx, y - 28, "note" if en else "音符", 11, SUB, 600, "middle"))
        if not ours:
            first = -37.0
            k = 0
            while first + k * frame_ms <= 40:
                fx = cx + (first + k * frame_ms) * ms
                s.add(f'<line x1="{fx:.1f}" y1="{y - 10}" x2="{fx:.1f}" y2="{y + 10}" stroke="#56657E" stroke-width="2"/>')
                k += 1
            s.add(text(x1 + 12, y + 5, "frames (60 fps)" if en else "フレーム（60fps）", 11, DIM))
        else:
            s.add(f'<path d="M{x0} {y + 30} ' + " ".join(f"L{x0 + k * 4} {y + 30 + 5 * math.sin(k * 0.9) * math.sin(k * 0.13)}" for k in range(int((x1 - x0) / 4))) + f'" stroke="{MAGENTA}" stroke-opacity="0.6" fill="none"/>')
            s.add(text(x1 + 12, y + 34, "audio position" if en else "音の再生位置", 11, MAGENTA))
    # 打撃の印（両方の行に同時に届く）
    hx = cx + hit_ms * ms
    arrive = 1.0
    # フレームで見る場合: 次のフレームまで待つ
    next_frame = -37.0 + math.ceil((hit_ms + 37.0) / frame_ms) * frame_ms
    fx = cx + next_frame * ms
    s.css.append(
        f"@keyframes inA{{0%{{transform:translateY(-34px);opacity:0}}{arrive / period * 100:.2f}%{{transform:translateY(0);opacity:1}}100%{{transform:translateY(0);opacity:1}}}}"
        f".ia{{animation:inA {period}s ease-out infinite}}"
    )
    for y, _, ours in rows:
        s.add(f'<g class="ia"><circle cx="{hx}" cy="{y}" r="7" fill="{ORANGE}"/></g>')
    w1 = s.window(period, [(arrive, period - 0.3)], fade=0.1)
    w2 = s.window(period, [(arrive + 0.6, period - 0.3)], fade=0.1)
    yA, yB = rows[0][0], rows[1][0]
    s.add(f'<g class="{w2}"><circle cx="{fx}" cy="{yA}" r="9" fill="none" stroke="{SUB}" stroke-width="2"/>'
          f'<path d="M{hx + 8} {yA - 16} Q {(hx + fx) / 2} {yA - 30} {fx} {yA - 12}" stroke="{SUB}" fill="none" stroke-dasharray="3 3"/>'
          + text(fx + 16, yA - 18, (f"seen at the next frame: +{next_frame:.0f} ms" if en else f"次のフレームで気づく: +{next_frame:.0f} ms"), 13, SUB, 700)
          + "</g>")
    s.add(f'<g class="{w1}">' + text(hx + 16, yB - 18, (f"stamped on arrival: +{hit_ms} ms" if en else f"届いた時刻で: +{hit_ms} ms"), 14, ORANGE, 800) + "</g>")
    s.add(text(40, 284, "Up to 16.7 ms of error at 60 fps is avoided. Window size and frame rate do not change your judgement." if en else "60fps なら最大 16.7ms の誤差が出ない所です。窓の大きさや画面の速さで判定が変わりません。", 13, DIM))
    return s


# ---------------------------------------------------------------- practice ladder


def practice(lang: str) -> Svg:
    en = lang == "en"
    s = Svg(
        lang,
        960,
        380,
        "Climb the steps to clear" if en else "段をのぼってクリア",
        "Practice a section from 70% speed up to 100%. Four laps per step; missing the same beat twice means trying the step again."
        if en
        else "区間を 70% から 100% まで 10% ずつのぼる。1 段 4 周。同じ拍で 2 回ミスしたら、その段をもう一度。",
    )
    card_bg(s, glow=("0.25", "0.9", GREEN))
    period = 16.0
    lap = 0.6
    # 時刻の段取り
    steps_def = [70, 80, 90, 100]
    # (段, 周の結果の並び, 始まり)
    timeline = []
    t = 0.4
    timeline.append((0, ["g", "g", "g", "g"], t))
    t += 4 * lap + 0.9
    timeline.append((1, ["g", "y", "r"], t))
    t += 3 * lap + 1.0
    timeline.append((1, ["g", "g", "g", "g"], t))
    t += 4 * lap + 0.9
    timeline.append((2, ["g", "g", "g", "g"], t))
    t += 4 * lap + 0.9
    timeline.append((3, ["g", "g", "g", "g"], t))
    t_clear = t + 4 * lap
    # 左: 段の絵
    bx, by = 60, 330
    sw, sh = 250, 56
    s.add(text(40, 46, "Climb the steps to clear" if en else "段をのぼってクリア", 22, INK, 800))
    s.add(text(40, 70, "Practice mode · A–B section loop" if en else "練習モード・区間をくり返す", 13, SUB))
    for k, pct in enumerate(steps_def):
        x = bx + k * 34
        y = by - (k + 1) * sh
        s.add(f'<rect x="{x}" y="{y}" width="{sw}" height="{sh - 8}" rx="8" fill="#121A29" stroke="#2A3650"/>')
        s.add(text(x + 16, y + 31, f"{pct}%", 18, INK, 800))
        if pct == 100:
            s.add(f'<path d="M{x + sw - 30} {y - 2} v-30 l22 8 l-22 8" fill="{ORANGE}" stroke="{ORANGE}" stroke-width="2"/>')
    def lap_xy(step, i):
        x = bx + step * 34 + 92 + i * 30
        y = by - (step + 1) * sh + 24
        return x, y
    # 周の丸（はじめは空）
    for k in range(4):
        for i in range(4):
            x, y = lap_xy(k, i)
            s.add(f'<circle cx="{x}" cy="{y}" r="10" fill="none" stroke="#3A4A66" stroke-width="2"/>')
    color = {"g": GREEN, "y": YELLOW, "r": RED}
    for idx, (step, results, start) in enumerate(timeline):
        # この挑戦の丸が見えている間（次の挑戦が同じ段なら、そこで消える）
        end = period
        for step2, _, start2 in timeline[idx + 1 :]:
            if step2 == step:
                end = start2
                break
        for i, r in enumerate(results):
            x, y = lap_xy(step, i)
            w = s.window(period, [(start + (i + 1) * lap, min(end, period) - 0.01)], fade=0.08)
            s.add(f'<circle class="{w}" cx="{x}" cy="{y}" r="10" fill="{color[r]}"/>')
    # 越えた段の ✓
    passes = [(0, timeline[1][2] - 0.6), (1, timeline[3][2] - 0.6), (2, timeline[4][2] - 0.6), (3, t_clear)]
    for step, at in passes:
        x = bx + step * 34 + sw - 22
        y = by - (step + 1) * sh + 24
        w = s.window(period, [(at, period - 0.3)], fade=0.15)
        s.add(f'<g class="{w}"><circle cx="{x}" cy="{y}" r="11" fill="{GREEN}"/><path d="M{x - 5} {y} l4 4 l7 -8" stroke="#06121B" stroke-width="2.6" fill="none" stroke-linecap="round"/></g>')
    # いまここ
    here = [(0, 0.0, timeline[1][2] - 0.2), (1, timeline[1][2] - 0.2, timeline[3][2] - 0.2), (2, timeline[3][2] - 0.2, timeline[4][2] - 0.2), (3, timeline[4][2] - 0.2, t_clear)]
    for step, a, b in here:
        x = bx + step * 34 - 8
        y = by - (step + 1) * sh
        w = s.window(period, [(a, b)], fade=0.15)
        s.add(f'<g class="{w}"><rect x="{x}" y="{y - 4}" width="{sw + 16}" height="{sh}" rx="10" fill="none" stroke="{ORANGE}" stroke-width="2.5"/>'
              + text(x - 4, y + 30, "you" if en else "いまここ", 12, ORANGE, 800, "end") + "</g>")

    # 右: レーンとお知らせ
    rx, ry, rw, rh = 470, 96, 450, 250
    s.add(f'<rect x="{rx}" y="{ry}" width="{rw}" height="{rh}" rx="12" fill="#070A11" stroke="#20293A"/>')
    # ミスの地図（4 小節 × 4 拍）
    mx, my = rx + 24, ry + 52
    s.add(text(mx, ry + 32, "Misses in this step" if en else "この段でミスした所", 13, SUB, 700))
    for m in range(4):
        s.add(text(mx - 4, my + m * 30 + 18, f"{m + 24}", 11, DIM, 600, "end", "mono"))
        for b in range(4):
            s.add(f'<rect x="{mx + 6 + b * 30}" y="{my + m * 30}" width="26" height="26" rx="4" fill="#141C2B" stroke="#25324A"/>')
    t1 = timeline[1][2]
    cell_x, cell_y = mx + 6 + 2 * 30, my + 1 * 30
    wy = s.window(period, [(t1 + 2 * lap, t1 + 3 * lap)], fade=0.06)
    wr = s.window(period, [(t1 + 3 * lap, timeline[2][2] + 4 * lap)], fade=0.06)
    s.add(f'<rect class="{wy}" x="{cell_x}" y="{cell_y}" width="26" height="26" rx="4" fill="{YELLOW}"/>')
    s.add(f'<g class="{wr}"><rect x="{cell_x}" y="{cell_y}" width="26" height="26" rx="4" fill="{RED}"/>' + text(cell_x + 13, cell_y + 18, "×2", 11, "#fff", 800, "middle") + "</g>")
    # レーン（赤く光る拍）
    lx0 = rx + 190
    for i, name in enumerate(LANES):
        s.add(f'<rect x="{lx0 + i * 24}" y="{ry + 14}" width="23" height="{rh - 28}" fill="{LANE[name]}" fill-opacity="0.05"/>')
    s.add(f'<rect x="{lx0}" y="{ry + rh - 40}" width="{24 * 10}" height="3" fill="#fff" fill-opacity="0.8"/>')
    wg = s.window(period, [(timeline[2][2], timeline[2][2] + 4 * lap)], fade=0.1)
    s.add(f'<rect class="{wg}" x="{lx0}" y="{ry + 70}" width="{24 * 10}" height="24" fill="{RED}" fill-opacity="0.35"/>')
    # 落ちてくる音符（雰囲気）
    s.css.append(f"@keyframes nf{{0%{{transform:translateY(0)}}100%{{transform:translateY({rh - 54}px)}}}}.nf{{animation:nf 1.2s linear infinite}}")
    rnd = random.Random(3)
    for k in range(10):
        name = LANES[rnd.randrange(10)]
        i = LANES.index(name)
        s.add(f'<rect class="nf" style="animation-delay:{-k * 0.12:.2f}s" x="{lx0 + i * 24 + 3}" y="{ry + 14}" width="17" height="5" rx="2" fill="{LANE[name]}" fill-opacity="0.8"/>')
    # お知らせ
    def banner(label, col, a, b):
        w = s.window(period, [(a, b)], fade=0.12)
        s.add(f'<g class="{w}"><rect x="{lx0 + 10}" y="{ry + 104}" width="220" height="44" rx="10" fill="#0B1220" stroke="{col}" stroke-width="2"/>'
              + text(lx0 + 120, ry + 133, label, 18, col, 800, "middle") + "</g>")
    nxt = "OK! Next: {}%" if en else "OK! 次は {}%"
    again = "Again: {}%" if en else "もう一度 {}%"
    banner(nxt.format(80), GREEN, timeline[1][2] - 0.8, timeline[1][2] + 0.2)
    banner(again.format(80), ORANGE, timeline[2][2] - 0.8, timeline[2][2] + 0.2)
    banner(nxt.format(90), GREEN, timeline[3][2] - 0.8, timeline[3][2] + 0.2)
    banner(nxt.format(100), GREEN, timeline[4][2] - 0.8, timeline[4][2] + 0.2)
    wc = s.window(period, [(t_clear, period - 0.2)], fade=0.15)
    s.add(f'<g class="{wc}"><rect x="{lx0 - 6}" y="{ry + 92}" width="252" height="68" rx="12" fill="#0B1220" stroke="{YELLOW}" stroke-width="3"/>'
          + text(lx0 + 120, ry + 138, "CLEAR!" if en else "クリア！", 34, YELLOW, 900, "middle") + "</g>")
    # 下の決まり
    rule = (
        "4 laps per step · the same beat missed twice → try the step again · two agains in a row → one step down"
        if en
        else "1 段 4 周 ・ 同じ拍で 2 回ミス → その段をもう一度 ・ もう一度が 2 回続いたら 1 段下げる"
    )
    s.add(text(40, 366, rule, 13, SUB))
    return s


# ---------------------------------------------------------------- hit error bar


def hit_error(lang: str) -> Svg:
    en = lang == "en"
    s = Svg(
        lang,
        960,
        210,
        "Hit error bar" if en else "ずれのバー",
        "Each hit leaves a line where it landed, early to the left and late to the right. The marker shows the average of the last 20 hits."
        if en
        else "叩くたびに、ずれた所に線が出る。左が早い、右が遅い。▼ は直近 20 打の平均。",
    )
    card_bg(s)
    s.add(text(40, 44, "See how early or late you are, while you play" if en else "叩きながら、早い・遅いが分かる", 20, INK, 800))
    period = 8.0
    cx, y = 480, 120
    scale = 3.2
    zones = [(117, "#FF9800", 0.10), (84, "#FFD54F", 0.12), (67, "#7CF29A", 0.14), (34, "#5CE1E6", 0.22)]
    for w, col, op in zones:
        s.add(f'<rect x="{cx - w * scale}" y="{y - 14}" width="{2 * w * scale}" height="28" rx="4" fill="{col}" fill-opacity="{op}"/>')
    s.add(f'<line x1="{cx}" y1="{y - 22}" x2="{cx}" y2="{y + 22}" stroke="#fff" stroke-width="2"/>')
    s.add(text(cx - 117 * scale - 12, y + 5, "EARLY" if en else "早い", 14, "#7FB8FF", 800, "end"))
    s.add(text(cx + 117 * scale + 12, y + 5, "LATE" if en else "遅い", 14, "#FF8F8F", 800))
    rnd = random.Random(11)
    hits = []
    t = 0.1
    while t < period - 0.1:
        off = rnd.gauss(-8, 17)
        hits.append((t, off))
        t += rnd.uniform(0.18, 0.36)
    # 1 本ずつ出て 4 秒で消える
    s.css.append(f"@keyframes tk{{0%{{opacity:1}}{4 / period * 100:.2f}%{{opacity:0}}100%{{opacity:0}}}}.tk{{opacity:0;animation:tk {period}s linear infinite}}")
    for t, off in hits:
        a = abs(off)
        col = "#5CE1E6" if a <= 34 else "#7CF29A" if a <= 67 else "#FFD54F"
        x = cx + off * scale
        s.add(f'<rect class="tk" style="animation-delay:{t - period:.3f}s" x="{x - 1.5:.1f}" y="{y - 16}" width="3" height="32" fill="{col}"/>')
    # 平均の ▼（ゆっくり動く）
    avg_frames = []
    for k in range(9):
        tt = k / 8 * period
        recent = [o for t, o in hits if t <= tt][-20:] or [0]
        m = sum(recent) / len(recent)
        avg_frames.append(f"{k / 8 * 100:.1f}%{{transform:translateX({m * scale:.1f}px)}}")
    s.css.append(f"@keyframes av{{{''.join(avg_frames)}}}.av{{animation:av {period}s ease-in-out infinite}}")
    s.add(f'<g class="av"><path d="M{cx - 8} {y - 34} h16 l-8 11 z" fill="#fff"/></g>')
    mean = sum(o for _, o in hits[-20:]) / 20
    s.add(text(cx, y + 50, (f"▼ average of the last 20 hits: {mean:+.0f} ms (early)" if en else f"▼ 直近 20 打の平均: {mean:+.0f} ms（早め）"), 14, INK, 700, "middle"))
    s.add(text(cx, y + 74, "Bands = judgement windows (PERFECT / GREAT / GOOD / POOR). Shown under the score, never over the lanes." if en else "帯は判定の幅（PERFECT・GREAT・GOOD・POOR）。レーンには重ねず、左のスコアの下に出ます。", 12, SUB, 400, "middle"))
    return s


# ---------------------------------------------------------------- MIDI devices


def devices(lang: str) -> Svg:
    en = lang == "en"
    s = Svg(
        lang,
        960,
        330,
        "One setting per device" if en else "機器ごとの設定",
        "Each MIDI device keeps its own preset, channel, minimum velocity and pad mapping, and they all feed the same 10 lanes."
        if en
        else "MIDI の機器ごとに、プリセット・チャンネル・最小ベロシティ・パッドの割り当てを持ち、同じ 10 レーンにつながる。",
    )
    card_bg(s)
    s.add(text(40, 44, "Every drum kit keeps its own settings" if en else "つなぐ機器ごとに、設定を分けて持てる", 20, INK, 800))
    s.add(text(40, 68, "Devices are told apart by port, not channel. Hi-hat openness (CC4), chokes and rim hits are read." if en else "機器はチャンネルではなくポートで見分けます。ハイハットの開き具合（CC4）・チョーク・リムも読みます。", 13, SUB))
    devs = [
        ("E-drum module · USB-MIDI" if en else "電子ドラムの音源 · USB-MIDI", ["Roland", "ch 10", "min vel 8" if en else "最小 8"], CYAN),
        ("Pad controller · USB-MIDI" if en else "パッドコントローラー · USB-MIDI", ["GM", "ch 10", "custom notes" if en else "独自の割り当て"], MAGENTA),
        ("Keyboard" if en else "キーボード", ["A S Z D G F H J K L"], YELLOW),
    ]
    lx, ly = 640, 110
    lw = 28
    for i, name in enumerate(LANES):
        s.add(f'<rect x="{lx + i * lw}" y="{ly}" width="{lw - 2}" height="170" rx="3" fill="{LANE[name]}" fill-opacity="0.07"/>')
        s.add(text(lx + i * lw + lw / 2 - 1, ly + 192, name, 10, LANE[name], 700, "middle", "mono"))
    s.add(f'<rect x="{lx}" y="{ly + 150}" width="{lw * 10 - 2}" height="3" fill="#fff" fill-opacity="0.8"/>')
    period = 3.6
    rnd = random.Random(5)
    for k, (label, chips, col) in enumerate(devs):
        y = 100 + k * 72
        s.add(f'<rect x="40" y="{y}" width="370" height="58" rx="10" fill="#111827" stroke="{col}" stroke-opacity="0.6"/>')
        s.add(text(56, y + 23, label, 14, INK, 700))
        x = 56
        for c in chips:
            w = len(c) * (7.2 if en or c.isascii() else 12.5) + 16
            s.add(f'<rect x="{x}" y="{y + 32}" width="{w:.0f}" height="18" rx="9" fill="{col}" fill-opacity="0.14"/>')
            s.add(text(x + 8, y + 45, c, 11, col, 700, "start", "mono" if c.isascii() else ""))
            x += w + 6
        # 線
        path_id = s.uid("p")
        s.defs.append(f'<path id="{path_id}" d="M410 {y + 29} C 520 {y + 29}, 540 {ly + 80}, {lx - 6} {ly + 80}"/>')
        s.add(f'<use href="#{path_id}" stroke="{col}" stroke-opacity="0.35" stroke-width="2" fill="none"/>')
        for j in range(3):
            begin = k * 0.4 + j * 1.2
            s.add(
                f'<circle r="5" fill="{col}"><animateMotion dur="{period}s" begin="{begin:.2f}s" repeatCount="indefinite" keyPoints="0;1;1" keyTimes="0;0.25;1" calcMode="linear">'
                f'<mpath href="#{path_id}"/></animateMotion><animate attributeName="opacity" values="1;1;0;0" keyTimes="0;0.24;0.26;1" dur="{period}s" begin="{begin:.2f}s" repeatCount="indefinite"/></circle>'
            )
            lane = rnd.randrange(10)
            fb = begin + period * 0.25
            s.add(
                f'<rect x="{lx + lane * lw}" y="{ly + 90}" width="{lw - 2}" height="60" fill="{LANE[LANES[lane]]}" opacity="0">'
                f'<animate attributeName="opacity" values="0;0.6;0;0" keyTimes="0;0.02;0.12;1" dur="{period}s" begin="{fb:.2f}s" repeatCount="indefinite"/></rect>'
            )
    return s


# ---------------------------------------------------------------- update


def update(lang: str) -> Svg:
    en = lang == "en"
    s = Svg(
        lang,
        960,
        250,
        "Updates when you choose" if en else "更新は自分のタイミングで",
        "A new version appears as a row under the title menu. Choose it to download while you keep playing, then restart to switch."
        if en
        else "新しい版はタイトルのメニューの下に行で出る。選ぶと遊びながら落とし、再起動で入れ替わる。",
    )
    period = 8.0
    titles = (
        [("New version", "Choose the new row on the title."), ("Downloading", "Keep playing; % in the corner."), ("Restart", "Choose it to switch versions."), ("Done", "Settings and records are kept.")]
        if en
        else [("新しい版", "タイトルの下に行が出ます。"), ("落とす", "遊びながら。隅に % が出ます。"), ("再起動", "再起動で新しい版に入れ替え。"), ("できあがり", "設定と記録はそのままです。")]
    )
    for i, (t, sub) in enumerate(titles):
        x = 8 + i * 238
        card_bg(s, x, 6, 228, 238, 16)
        hl = s.window(period, [(i * 2.0, i * 2.0 + 2.0)], fade=0.2)
        s.add(f'<rect class="{hl}" x="{x + 2}" y="8" width="224" height="234" rx="15" fill="none" stroke="{CYAN}" stroke-width="2"/>')
        sx, sy, sw, sh = x + 16, 22, 196, 118
        s.add(f'<rect x="{sx}" y="{sy}" width="{sw}" height="{sh}" rx="8" fill="#0A0F18" stroke="#24314A"/>')
        mx = sx + sw / 2
        if i in (0, 2, 3):
            for k, item in enumerate(("Start", "Config", "Quit")):
                s.add(text(mx, sy + 34 + k * 20, item, 13, "#C8D3E3", 700, "middle"))
        if i == 0:
            s.add(f'<rect x="{sx + 28}" y="{sy + 84}" width="{sw - 56}" height="24" rx="6" fill="{GREEN}" fill-opacity="0.18" stroke="{GREEN}"/>')
            s.add(text(mx, sy + 101, f"新しい版 {SAMPLE_VERSION}", 12, GREEN, 800, "middle"))
        elif i == 1:
            # 演奏の画面と隅の %
            for k, name in enumerate(LANES):
                s.add(f'<rect x="{sx + 48 + k * 10}" y="{sy + 8}" width="9" height="{sh - 16}" fill="{LANE[name]}" fill-opacity="0.12"/>')
            s.add(f'<rect x="{sx + sw - 54}" y="{sy + sh - 26}" width="46" height="18" rx="9" fill="#111827" stroke="{CYAN}"/>')
            for k in range(0, 101, 20):
                w = s.window(period, [(2.0 + k / 100 * 1.8, 2.0 + (k + 20) / 100 * 1.8 if k < 100 else period - 0.1)], fade=0.0)
                s.add(text(sx + sw - 31, sy + sh - 13, f"{k}%", 11, CYAN, 800, "middle", f"{w} mono"))
        elif i == 2:
            s.add(f'<rect x="{sx + 22}" y="{sy + 84}" width="{sw - 44}" height="24" rx="6" fill="{ORANGE}" fill-opacity="0.18" stroke="{ORANGE}"/>')
            s.add(text(mx, sy + 101, "再起動して入れ替える", 12, ORANGE, 800, "middle"))
        else:
            s.add(f'<rect x="{sx + 22}" y="{sy + 84}" width="{sw - 44}" height="24" rx="12" fill="{CYAN}" fill-opacity="0.18"/>')
            s.add(text(mx, sy + 101, f"{SAMPLE_VERSION} になりました", 12, CYAN, 800, "middle"))
        s.add(f'<circle cx="{x + 32}" cy="172" r="13" fill="none" stroke="{CYAN}" stroke-width="2"/>')
        s.add(text(x + 32, 177, str(i + 1), 13, CYAN, 800, "middle"))
        s.add(text(x + 54, 178, t, 17, INK, 800))
        s.add(text(x + 18, 206, sub, 12, SUB))
    return s


# ---------------------------------------------------------------- lineage


def lineage(lang: str) -> Svg:
    en = lang == "en"
    s = Svg(
        lang,
        960,
        300,
        "Where DTXmaniaNXT comes from" if en else "DTXmaniaNXT の系譜",
        "DTXMania, DTXManiaXG verK, DTXManiaNX by limyz, DTXManiaNX v2.0 by BocuD, and DTXmaniaNXT. NXT is new code; only the v2.0 skin (images and sounds) is bundled unchanged."
        if en
        else "DTXMania、DTXManiaXG verK、limyz 版 DTXManiaNX、BocuD 版 DTXManiaNX v2.0、DTXmaniaNXT。NXT は新しいコードで、v2.0 のスキン（画像と効果音）だけをそのまま同梱。",
    )
    card_bg(s)
    nodes = [
        ("DTXMania", "FROM · DTXMania Group", "2000 –", "#8FA3BF"),
        ("DTXManiaXG verK", "kairera0467", "", "#8FA3BF"),
        ("DTXManiaNX", "limyz · fisyher", "", "#8FA3BF"),
        ("DTXManiaNX v2.0", "BocuD", "", MAGENTA),
        ("DTXmaniaNXT", "this project · Rust" if en else "このアプリ · Rust", "2026 –", CYAN),
    ]
    w, gap, y = 166, 22, 92
    for i, (name, who, when, col) in enumerate(nodes):
        x = 30 + i * (w + gap)
        s.add(f'<rect x="{x}" y="{y}" width="{w}" height="78" rx="12" fill="#111827" stroke="{col}" stroke-width="{2.5 if i >= 3 else 1.5}"/>')
        s.add(text(x + w / 2, y + 30, name, 15, INK, 800, "middle"))
        s.add(text(x + w / 2, y + 52, who, 12, col, 600, "middle"))
        if when:
            s.add(text(x + w / 2, y + 69, when, 11, DIM, 600, "middle"))
        if i < 3:
            ax = x + w + 2
            s.add(f'<path d="M{ax} {y + 39} h{gap - 6} m-6 -5 l6 5 l-6 5" stroke="#8FA3BF" stroke-width="2" fill="none"/>')
    # v2.0 → NXT は、スキンだけ（点線）
    x3 = 30 + 3 * (w + gap) + w
    s.add(f'<path d="M{x3 + 2} {y + 39} h{gap - 6} m-6 -5 l6 5 l-6 5" stroke="{MAGENTA}" stroke-width="2" fill="none" stroke-dasharray="4 3"/>')
    s.add(text(40, 44, "Where it comes from" if en else "どこから来たか", 20, INK, 800))
    s.add(text(40, 68, "Code lineage of the DTXMania family, as described in the DTXManiaNX READMEs." if en else "DTXManiaNX の README に書かれている、DTXMania の流れ。", 13, SUB))
    # 下の説明
    notes = (
        [
            ("— code", "Each project was forked from the one before it.", "#8FA3BF", False),
            ("- - skin only", "NXT bundles the v2.0 images and sounds unchanged (tag V2.0.0-20260121-5).", MAGENTA, True),
            ("", "NXT's code is written from scratch in Rust. No upstream code is copied.", CYAN, False),
        ]
        if en
        else [
            ("— コード", "それぞれ、前のプロジェクトからのフォークです。", "#8FA3BF", False),
            ("- - スキンだけ", "NXT は v2.0 の画像と効果音を変えずに同梱しています（タグ V2.0.0-20260121-5）。", MAGENTA, True),
            ("", "NXT のコードは Rust で一から書いたもので、上流のコードは写していません。", CYAN, False),
        ]
    )
    for k, (lab, body, col, dashed) in enumerate(notes):
        yy = 210 + k * 26
        if lab:
            s.add(f'<line x1="40" y1="{yy - 4}" x2="70" y2="{yy - 4}" stroke="{col}" stroke-width="2" {"stroke-dasharray=\"4 3\"" if dashed else ""}/>')
            s.add(text(78, yy, lab.replace("— ", "").replace("- - ", ""), 12, col, 700))
        else:
            s.add(f'<circle cx="55" cy="{yy - 4}" r="5" fill="{col}"/>')
        s.add(text(200 if lab or True else 78, yy, body, 13, INK if not lab else SUB))
    return s


ALL = {
    "hero": hero,
    "download": download,
    "steps": steps,
    "timing": timing,
    "practice": practice,
    "hit-error": hit_error,
    "devices": devices,
    "update": update,
    "lineage": lineage,
}


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, make in ALL.items():
        for lang in ("en", "ja"):
            svg = make(lang).render()
            path = OUT / (f"{name}.svg" if lang == "en" else f"{name}.ja.svg")
            path.write_text(svg, encoding="utf-8")
            print(f"{path.relative_to(OUT.parents[1])}  {len(svg) / 1024:.1f} KB")


if __name__ == "__main__":
    main()
