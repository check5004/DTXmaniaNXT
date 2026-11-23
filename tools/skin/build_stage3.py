"""第 3 段階: 残りの画像（演奏・選曲・結果・読み込み・設定・共通）を「A スプレー」の絵柄で作る。

    python3 tools/skin/build_stage3.py <第 1 段階の生成の dir> <第 3 段階の生成の dir> <フォントの dir> assets/skin/Graphics

ほとんどは合成とフォントで描く（費用なし）。画像生成の絵は 2 枚だけ使う（prompts.json の result_words と badges）。
第 1 段階で作った絵（8_rank*.png、1_background.jpg、logo_a.png）も材料にする。

フォント（どれも SIL Open Font License。フォントそのものは配らず、描いた絵だけを入れる）:
- Rubik Spray Paint（<フォントの dir>/RubikSprayPaint-Regular.ttf。https://github.com/google/fonts/tree/main/ofl/rubikspraypaint ）
- Inter / Inter Display（fonts-inter のもの。fc-match で探す）

数字や文字を並べた絵は、アプリが切り出す位置（play_view.rs・select.rs・loading.rs・gfx.rs の表）を変えずに描く。
"""

import math
import os
import subprocess
import sys

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageEnhance, ImageFilter, ImageFont

RAW1, RAW3, FONTDIR, OUT = sys.argv[1:5]
SPRAY = os.path.join(FONTDIR, "RubikSprayPaint-Regular.ttf")
rng = np.random.default_rng(23)


def fc(pattern: str) -> str:
    return subprocess.run(["fc-match", "-f", "%{file}", pattern], capture_output=True, text=True, check=True).stdout


BLACK_IT = fc("Inter Display:style=Black Italic")
XBOLD_IT = fc("Inter Display:style=ExtraBold Italic")
BOLD_IT = fc("Inter Display:style=Bold Italic")
BOLD = fc("Inter Display:style=Bold")
BLACK = fc("Inter Display:style=Black")
SEMI = fc("Inter:style=SemiBold")
for f in (BLACK_IT, XBOLD_IT, BOLD_IT, BOLD, BLACK, SEMI):
    assert "Inter" in os.path.basename(f), f

# ---- 色
LIME = (150, 255, 60)
VIOLET = (138, 60, 255)
GOLD = (255, 214, 64)
WHITE = (255, 255, 255)
INK = (20, 16, 24)
C = {  # レーンの色（今の色に合わせる）
    "LC": (255, 95, 174), "HH": (79, 180, 255), "LP": (255, 95, 174), "SD": (255, 210, 60),
    "HT": (62, 224, 106), "BD": (166, 140, 255), "LT": (255, 74, 74), "FT": (255, 162, 58),
    "CY": (79, 180, 255), "RD": (124, 200, 255), "HHO": (214, 214, 214), "LBD": (102, 230, 255),
}
JUDGE = [("PERFECT", (255, 214, 64)), ("GREAT", (90, 232, 110)), ("GOOD", (80, 182, 255)),
         ("OK", (186, 120, 255)), ("MISS", (255, 80, 80)), ("AUTO", (200, 200, 210))]
# 難易度の色（選曲の 5 段と、DTX の #DLABEL の名前）
DIFF = {
    "DTXMANIA": LIME, "DTX": LIME, "BASIC": (70, 160, 255), "NOVICE": (70, 160, 255),
    "ADVANCED": (255, 200, 40), "REGULAR": (255, 200, 40), "EXTREME": (255, 60, 80), "EXPERT": (255, 60, 80),
    "MASTER": (220, 70, 255), "RAW": (220, 70, 255), "RWS": (255, 160, 40), "REAL": (255, 100, 40),
}


def save(img, name):
    p = os.path.join(OUT, name)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    if name.endswith(".jpg"):
        img.convert("RGB").save(p, quality=92)
    else:
        img.save(p, optimize=True)


def lighten(c, t):
    return tuple(int(v + (255 - v) * t) for v in c)


def darken(c, t):
    return tuple(int(v * (1 - t)) for v in c)


def noise(w, h, scale=1.0):
    return rng.random((h, w)) * scale


def smooth_noise(w, h, cell):
    """なめらかなむら（0〜1）"""
    small = rng.random((max(2, h // cell + 2), max(2, w // cell + 2)))
    im = Image.fromarray((small * 255).astype(np.uint8), "L").resize((w, h), Image.BICUBIC)
    return np.asarray(im, np.float32) / 255


def speckle(w, h, color, density, alpha, blur=0.6, size=(1, 3)):
    """スプレーの細かい粒"""
    layer = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    n = int(w * h * density)
    xs = rng.random(n) * w
    ys = rng.random(n) * h
    rs = rng.uniform(size[0], size[1], n) / 2
    for x, y, r in zip(xs, ys, rs):
        d.ellipse((x - r, y - r, x + r, y + r), fill=color + (alpha,))
    return layer.filter(ImageFilter.GaussianBlur(blur))


def mist(w, h, cx, cy, rx, ry, color, alpha):
    """ぼかした色の霧"""
    layer = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    d.ellipse((cx - rx, cy - ry, cx + rx, cy + ry), fill=color + (alpha,))
    return layer.filter(ImageFilter.GaussianBlur(min(rx, ry) * 0.6))


def masked(layer, mask):
    """layer の透明度に mask（L）を掛ける"""
    layer.putalpha(ImageChops.multiply(layer.getchannel("A"), mask))
    return layer


def rough_mask(w, h, box, radius, rough=1.6):
    """角の丸い四角の、ふちを少しがたつかせた形（ちぎったテープや塗った板のふち）"""
    m = Image.new("L", (w, h), 0)
    ImageDraw.Draw(m).rounded_rectangle(box, radius=radius, fill=255)
    if rough <= 0:
        return m
    a = np.asarray(m.filter(ImageFilter.GaussianBlur(rough)), np.float32) / 255
    n = smooth_noise(w, h, 3) - 0.5
    a = np.clip((a - 0.5 + n * 0.35) * 6 + 0.5, 0, 1)
    return Image.fromarray((a * 255).astype(np.uint8), "L")


def panel(w, h, box, radius=6, fill=(16, 16, 20), alpha=235, rough=1.6, texture=9.0, grain=False):
    """塗った黒い板。コンクリートのむらと、細かいざらつき"""
    m = rough_mask(w, h, box, radius, rough)
    base = np.zeros((h, w, 3), np.float32) + fill
    blot = smooth_noise(w, h, 40) - 0.5
    base += blot[..., None] * texture * 1.4
    base += (noise(w, h) - 0.5)[..., None] * texture
    if grain:  # 木目（横に伸ばしたむら）
        g = np.asarray(Image.fromarray((rng.random((h, max(2, w // 60))) * 255).astype(np.uint8), "L").resize((w, h), Image.BICUBIC), np.float32) / 255
        base += (g - 0.5)[..., None] * texture * 1.2
    im = Image.fromarray(base.clip(0, 255).astype(np.uint8)).convert("RGBA")
    im.putalpha(m.point(lambda v: v * alpha // 255))
    return im, m


def edge_of(mask, width):
    """形のふちの帯"""
    outer = mask.filter(ImageFilter.MaxFilter(width * 2 + 1)) if width > 0 else mask
    inner = mask.filter(ImageFilter.MinFilter(width * 2 + 1))
    return ImageChops.subtract(outer, inner)


def spray_line(im, mask_edge, color, alpha=230, over=0.6):
    """ふちの帯にスプレーの線（芯と、まわりの吹きこぼれ）"""
    w, h = im.size
    core = Image.new("RGBA", (w, h), color + (0,))
    core.putalpha(mask_edge.point(lambda v: v * alpha // 255))
    halo = Image.new("RGBA", (w, h), color + (0,))
    halo.putalpha(mask_edge.filter(ImageFilter.GaussianBlur(3)).point(lambda v: int(v * over * 0.6)))
    sp = speckle(w, h, color, 0.01, 200, blur=0.3, size=(0.6, 1.6))
    sp = masked(sp, mask_edge.filter(ImageFilter.GaussianBlur(4)).point(lambda v: min(255, v * 3)))
    im.alpha_composite(halo)
    im.alpha_composite(sp)
    im.alpha_composite(core)


def outline_box(w, h, box, radius, width, rough=1.2):
    return edge_of(rough_mask(w, h, box, radius, rough), width)


def fit(im, w, h):
    s = min(w / im.width, h / im.height)
    return im.resize((max(1, round(im.width * s)), max(1, round(im.height * s))), Image.LANCZOS)


def crop_alpha(im, thr=8):
    return im.crop(im.getchannel("A").point(lambda a: 255 if a > thr else 0).getbbox())


def paste_center(dst, im, cx, cy):
    dst.alpha_composite(im, (round(cx - im.width / 2), round(cy - im.height / 2)))


# ---- 文字
def text_on(im, xy, text, path, size, fill, stroke=0, stroke_fill=INK, anchor="la", shadow=None, grad=None):
    """im に文字を描く。shadow は (dx, dy, ぼかし, 色, 濃さ)、grad は (上の色, 下の色)"""
    fnt = ImageFont.truetype(path, size)
    w, h = im.size
    if shadow:
        dx, dy, bl, col, al = shadow
        sh = Image.new("RGBA", (w, h), (0, 0, 0, 0))
        ImageDraw.Draw(sh).text((xy[0] + dx, xy[1] + dy), text, font=fnt, fill=col + (al,), anchor=anchor,
                                stroke_width=stroke, stroke_fill=col + (al,))
        im.alpha_composite(sh.filter(ImageFilter.GaussianBlur(bl)) if bl else sh)
    d = ImageDraw.Draw(im)
    if stroke:
        d.text(xy, text, font=fnt, fill=stroke_fill + (255,), anchor=anchor, stroke_width=stroke, stroke_fill=stroke_fill + (255,))
    if grad:
        mask = Image.new("L", (w, h), 0)
        ImageDraw.Draw(mask).text(xy, text, font=fnt, fill=255, anchor=anchor)
        bb = mask.getbbox()
        if bb:
            top, bot = grad
            t = np.clip((np.arange(h) - bb[1]) / max(1, bb[3] - bb[1]), 0, 1)[:, None]
            g = np.zeros((h, w, 4), np.uint8)
            for i in range(3):
                g[..., i] = (top[i] * (1 - t) + bot[i] * t).astype(np.uint8)
            g[..., 3] = np.asarray(mask)
            im.alpha_composite(Image.fromarray(g, "RGBA"))
    else:
        d.text(xy, text, font=fnt, fill=fill + (255,), anchor=anchor)


def glyphs(sheet, cells, path, size, ref="0", xscale=1.0, ss=4, **style):
    """数字や文字を升に 1 つずつ描く。cells は (文字, x, y, 幅, 高さ)。
    横は升の真ん中、縦は ref の文字の高さで揃える（並べたときに沈み込まない）。ss 倍で描いて縮める"""
    fnt = ImageFont.truetype(path, size * ss)
    rb = fnt.getbbox(ref)
    for ch, x, y, w, h in cells:
        W, H = round(w * ss / xscale), h * ss
        cell = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        cb = fnt.getbbox(ch)
        tx = (W - (cb[2] - cb[0])) / 2 - cb[0]
        ty = (H - (rb[3] - rb[1])) / 2 - rb[1]
        st = dict(style)
        if "stroke" in st:
            st["stroke"] *= ss
        if st.get("shadow"):
            dx, dy, bl, col, al = st["shadow"]
            st["shadow"] = (dx * ss, dy * ss, bl * ss, col, al)
        text_on(cell, (tx, ty), ch, path, size * ss, **st)
        sheet.alpha_composite(cell.resize((w, h), Image.LANCZOS), (x, y))


def row_cells(chars, w, h, x0=0, y0=0, extra=()):
    """0〜9 が w ずつ横に並ぶ升。extra は (文字, x, 幅)"""
    out = [(ch, x0 + i * w, y0, w, h) for i, ch in enumerate(chars)]
    out += [(ch, x0 + x, y0, ew, h) for ch, x, ew in extra]
    return out


DIGITS = "0123456789"


# =====================================================================
# 演奏の画面: レーン・チップ・パッド・判定の文字・コンボ・叩いたときの光
# =====================================================================

# ---- レーンの枠 7_Paret.png（558×720、不透明。play_view.rs の PANEL）
PANEL = [("LC", 0, 72), ("HH", 72, 49), ("LP", 121, 51), ("SD", 172, 57), ("HT", 229, 49),
         ("BD", 278, 69), ("LT", 347, 49), ("FT", 396, 54), ("CY", 450, 70), ("RD", 520, 38)]


def paret(dark=False):
    W, H = 558, 720
    base = np.full((H, W, 3), (13, 13, 17), np.float32)
    base += (noise(W, H, 10) - 5)[..., None]  # 壁のざらつき
    img = Image.fromarray(base.clip(0, 255).astype(np.uint8)).convert("RGBA")
    for lane, x, w in PANEL:
        col = C[lane]
        a = np.linspace(0, 1, H) ** 1.6
        arr = np.zeros((H, w, 4), np.uint8)
        arr[..., :3] = col
        arr[..., 3] = (a * (34 if not dark else 16))[:, None]
        img.alpha_composite(Image.fromarray(arr, "RGBA"), (x, 0))
        # スプレーの粒（下ほど多い）
        sp = speckle(w, H, col, 0.0035, 70 if not dark else 35)
        m = Image.fromarray((np.linspace(0.15, 1, H) ** 2 * 255).astype(np.uint8)[:, None].repeat(w, 1), "L")
        img.alpha_composite(masked(sp, m), (x, 0))
    d = ImageDraw.Draw(img)
    for lane, x, w in PANEL[1:]:
        d.line((x, 0, x, H), fill=(52, 52, 60, 255), width=2)
    d.line((W - 1, 0, W - 1, H), fill=(52, 52, 60, 255))
    return img.convert("RGB")


save(paret(), "7_Paret.png")
save(paret(True), "7_Paret_Dark.png")

# ---- チップ 7_chips_drums.png（718×776。play_view.rs の chip()）
CHIPS = [("BD", 0, 70), ("HH", 70, 56), ("SD", 126, 64), ("HT", 190, 56), ("LT", 246, 56), ("FT", 302, 56),
         ("CY", 358, 74), ("RD", 432, 48), ("LBD", 480, 58), ("LC", 538, 74), ("HHO", 612, 48), ("LP", 660, 58)]


def chip_bar(w, col, glow=False):
    """幅 w・高さ 64 の 1 コマ。帯は y 26〜37"""
    im = Image.new("RGBA", (w, 64), (0, 0, 0, 0))
    x0, x1, y0, y1 = 5, w - 6, 26, 37
    im.alpha_composite(mist(w, 64, w / 2, 31.5, (x1 - x0) / 2 + (6 if glow else 2), 9 if glow else 6, col, 150 if glow else 90))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((x0, y0, x1, y1), radius=5, fill=darken(col, 0.35) + (255,))
    d.rounded_rectangle((x0 + 1, y0 + 1, x1 - 1, y1 - 2), radius=4, fill=col + (255,))
    d.rounded_rectangle((x0 + 4, y0 + 3, x1 - 4, y0 + 5), radius=1, fill=lighten(col, 0.75) + (255,))
    sp = speckle(w, 64, lighten(col, 0.3), 0.02, 160, blur=0.3, size=(0.8, 1.8))
    mask = Image.new("L", (w, 64), 0)
    ImageDraw.Draw(mask).rounded_rectangle((x0 - 3, y0 - 3, x1 + 3, y1 + 3), radius=7, fill=255)
    im.alpha_composite(masked(sp, mask))
    return im


def pattern(w, col, f, lane):
    """チップのまわりの模様（8 コマ）"""
    im = Image.new("RGBA", (w, 64), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    t = f / 8
    if lane == "HHO":  # オープンは輪で見分ける
        r = 9 + 1.5 * math.sin(t * 2 * math.pi)
        d.ellipse((w / 2 - r, 31.5 - r, w / 2 + r, 31.5 + r), outline=(235, 235, 235, 230), width=2)
        return im
    a = int(70 + 50 * math.sin(t * 2 * math.pi))
    im.alpha_composite(mist(w, 64, w / 2, 31.5, w / 2 - 2, 13, col, a))
    if lane in ("BD", "LP", "LBD"):  # 足のチップは下に矢じり
        d = ImageDraw.Draw(im)
        cx = w / 2
        yy = 44 + 2 * math.sin(t * 2 * math.pi)
        d.polygon([(cx - 9, yy), (cx + 9, yy), (cx, yy + 7)], fill=col + (200,))
    return im


def chips():
    sheet = Image.new("RGBA", (718, 776), (0, 0, 0, 0))
    for lane, sx, w in CHIPS:
        col = C[lane]
        bar = chip_bar(w, col)
        sheet.alpha_composite(bar, (sx, 0))
        sheet.alpha_composite(chip_bar(w, col, True), (sx, 64))
        for f in range(8):
            sheet.alpha_composite(pattern(w, col, f, lane), (sx, 128 + f * 64))
        sheet.alpha_composite(bar, (sx, 640))
    d = ImageDraw.Draw(sheet)
    d.rectangle((0, 769, 558, 770), fill=(255, 255, 255, 170))  # 小節線
    d.rectangle((0, 772, 558, 773), fill=(255, 255, 255, 50))  # 拍の線
    return sheet


save(chips(), "7_chips_drums.png")

# ---- パッド 7_pads.png と光った絵（384×288、96 角。play_view.rs の PADS）
PADS = {"LC": (0, 0), "HH": (96, 0), "CY": (192, 0), "RD": (288, 0), "SD": (0, 96), "HT": (96, 96),
        "LT": (192, 96), "FT": (288, 96), "BD": (0, 192), "LP": (96, 192)}
# 升の中のレーンの真ん中と幅（パッドは y 570 に描かれ、y 625 より下はゲージに隠れる）
PAD_CX = {"LC": 66, "HH": 56, "SD": 50, "BD": 46, "HT": 40, "LT": 46, "FT": 44, "CY": 44, "LP": 46, "RD": 42}
PAD_W = {"LC": 66, "HH": 50, "SD": 58, "BD": 60, "HT": 50, "LT": 50, "FT": 52, "CY": 66, "LP": 50, "RD": 44}


def pad(lane, lit):
    S = 4
    im = Image.new("RGBA", (96 * S, 96 * S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    col = C[lane]
    cym = lane in ("LC", "HH", "CY", "RD")
    foot = lane in ("BD", "LP")

    def k(*v):
        return tuple(int(x * S) for x in v)

    cx, w = PAD_CX[lane], PAD_W[lane]
    if foot:
        box = k(cx - w / 2 + 6, 6, cx + w / 2 - 6, 56)
    elif cym:
        box = k(cx - w / 2, 14, cx + w / 2, 40)
    else:
        box = k(cx - w / 2, 10, cx + w / 2, 50)
    head = (236, 232, 222) if not lit else lighten(col, 0.55)
    if foot:
        d.rounded_rectangle(box, radius=10 * S, fill=(40, 40, 46, 255), outline=col + (255,), width=5 * S)
        b = [v / S for v in box]
        d.rounded_rectangle(k(b[0] + 7, b[1] + 7, b[2] - 7, b[3] - 16), radius=5 * S, fill=head + (255,))
    else:
        d.ellipse(box, fill=head + (255,))
        d.ellipse(box, outline=col + (255,), width=(5 if not cym else 4) * S)
        if cym:
            d.ellipse(k(cx - 6, 23, cx + 6, 31), fill=darken(head, 0.25) + (255,))
    im = im.resize((96, 96), Image.LANCZOS)
    sp = speckle(96, 96, col, 0.05, 120 if not lit else 200, blur=0.4, size=(0.6, 1.6))
    im.alpha_composite(masked(sp, im.getchannel("A")))
    if lit:
        glow = mist(96, 96, cx, 30, w / 2 + 6, 26, col, 170)
        glow.alpha_composite(im)
        im = glow
    return im


for name, lit in (("7_pads.png", False), ("ScreenPlayDrums pads flush.png", True)):
    sheet = Image.new("RGBA", (384, 288), (0, 0, 0, 0))
    for lane, (x, y) in PADS.items():
        sheet.alpha_composite(pad(lane, lit), (x, y))
    save(sheet, name)


# ---- 判定の文字 7_judge strings.png（6 列 × 24 コマ、250×170。14 ミリ秒ごとに 1 コマ）
def word(text, col, size):
    fnt = ImageFont.truetype(SPRAY, size)
    padding = 24
    bb = fnt.getbbox(text)
    w, h = bb[2] - bb[0] + padding * 2, bb[3] - bb[1] + padding * 2
    im = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    pos = (padding - bb[0], padding - bb[1])
    sh = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    ImageDraw.Draw(sh).text((pos[0] + 3, pos[1] + 4), text, font=fnt, fill=(0, 0, 0, 200), stroke_width=5, stroke_fill=(0, 0, 0, 200))
    im.alpha_composite(sh.filter(ImageFilter.GaussianBlur(3)))
    d = ImageDraw.Draw(im)
    d.text(pos, text, font=fnt, fill=INK + (255,), stroke_width=4, stroke_fill=INK + (255,))
    d.text(pos, text, font=fnt, fill=col + (255,))
    return im


def judges():
    sheet = Image.new("RGBA", (1500, 4080), (0, 0, 0, 0))
    for ci, (text, col) in enumerate(JUDGE):
        base = word(text, col, 32)
        for f in range(24):
            if f < 3:  # 少し大きく出て、すぐ落ち着く
                s, a, sx = 1.24 - 0.08 * f, 0.55 + 0.15 * f, 1.0
            elif f < 18:
                s, a, sx = 1.0, 1.0, 1.0
            else:  # 横に広がりながら消える
                k = (f - 17) / 6
                s, a, sx = 1.0, 1 - k, 1 + 0.15 * k
            w = int(base.width * s * sx)
            h = int(base.height * s * (1 - 0.4 * max(0, (f - 17) / 6)))
            im = base.resize((max(1, w), max(1, h)), Image.LANCZOS)
            if a < 1:
                im.putalpha(im.getchannel("A").point(lambda v: int(v * a)))
            lift = -min(f, 17) * 0.25
            tmp = Image.new("RGBA", (max(250, im.width), max(170, im.height)), (0, 0, 0, 0))
            tmp.alpha_composite(im, ((tmp.width - im.width) // 2, int((tmp.height - im.height) // 2 + lift)))
            l, t = (tmp.width - 250) // 2, (tmp.height - 170) // 2
            sheet.alpha_composite(tmp.crop((l, t, l + 250, t + 170)), (ci * 250, f * 170))
    return sheet


save(judges(), "7_judge strings.png")


# ---- コンボの数字（ScreenPlayDrums combo.png 600×380、combo_2.png 480×316。数字は 5×2 の升、COMBO の字は (0, 320, 250, 60)）
def digit_sheet(cw, ch, size, total_h, label_h=60):
    sheet = Image.new("RGBA", (cw * 5, total_h), (0, 0, 0, 0))
    font = ImageFont.truetype(SPRAY, size)
    for dgt in range(10):
        cell = Image.new("RGBA", (cw, ch), (0, 0, 0, 0))
        bb = font.getbbox(str(dgt))
        pos = (cw / 2 - (bb[0] + bb[2]) / 2, ch / 2 - (bb[1] + bb[3]) / 2)
        cell.alpha_composite(mist(cw, ch, cw / 2, ch / 2, cw / 2 - 14, ch / 2 - 22, VIOLET, 120))
        text_on(cell, pos, str(dgt), SPRAY, size, LIME, stroke=6, grad=((230, 255, 120), (150, 255, 60)))
        sheet.alpha_composite(cell, ((dgt % 5) * cw, (dgt // 5) * ch))
    lab = Image.new("RGBA", (250, label_h), (0, 0, 0, 0))
    text_on(lab, (125, label_h / 2), "COMBO", SPRAY, int(label_h * 0.78), LIME, stroke=3, anchor="mm")
    sheet.alpha_composite(lab, (0, ch * 2))
    return sheet


save(digit_sheet(120, 160, 150, 380), "ScreenPlayDrums combo.png")
save(digit_sheet(96, 128, 120, 316), "ScreenPlayDrums combo_2.png")


# ---- 火花（128×128、黒地に加算）と星（32×32）とレーンの光（42×128）
def fire(col):
    W = 128
    im = Image.new("RGBA", (W, W), (0, 0, 0, 255))
    layer = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    for _ in range(90):
        ang = rng.random() * 2 * math.pi
        r = rng.uniform(8, 56)
        s = rng.uniform(1, 4.5) * (1 - r / 70)
        x, y = 64 + r * math.cos(ang), 64 + r * math.sin(ang)
        d.ellipse((x - s, y - s, x + s, y + s), fill=lighten(col, 0.3) + (255,))
    layer = layer.filter(ImageFilter.GaussianBlur(0.8))
    im.alpha_composite(mist(W, W, 64, 64, 34, 34, col, 200))
    im.alpha_composite(layer)
    im.alpha_composite(mist(W, W, 64, 64, 14, 14, lighten(col, 0.7), 255))
    return im.convert("RGB").convert("RGBA")


FIRE_LANES = ["BD", "CY", "FT", "HH", "HT", "LC", "LP", "LT", "RD", "SD"]
for lane in FIRE_LANES:
    save(fire(C[lane]), f"ScreenPlayDrums chip fire_{lane}.png")
save(fire((255, 240, 160)), "ScreenPlayDrums chip fire_Bonus.png")

for lane in FIRE_LANES:
    st = Image.new("RGBA", (32, 32), (0, 0, 0, 0))
    ImageDraw.Draw(st).ellipse((11, 11, 21, 21), fill=lighten(C[lane], 0.3) + (255,))
    save(st.filter(ImageFilter.GaussianBlur(1.2)), f"ScreenPlayDrums chip star_{lane}.png")

FLUSH = {"leftcymbal": "LC", "hihat": "HH", "snare": "SD", "bass": "BD", "hitom": "HT", "lowtom": "LT",
         "floortom": "FT", "cymbal": "CY", "leftpedal": "LP", "left pedal": "LP"}
for name, lane in FLUSH.items():
    col = C[lane]
    arr = np.zeros((128, 42, 4), np.uint8)
    arr[..., :3] = col
    t = np.linspace(1, 0, 128) ** 1.2  # 上が明るく、下へ消える（下から上へ動く）
    edge = np.minimum(np.linspace(0, 1, 42), np.linspace(1, 0, 42)) * 2
    arr[..., 3] = (t[:, None] * np.clip(edge * 1.6, 0, 1)[None, :] * 70).astype(np.uint8)
    im = Image.fromarray(arr, "RGBA")
    sp = speckle(42, 128, lighten(col, 0.3), 0.03, 90, blur=0.4, size=(0.7, 1.6))
    im.alpha_composite(masked(sp, im.getchannel("A").point(lambda v: min(255, v * 4))))
    save(im, f"ScreenPlayDrums lane flush {name}.png")
    save(im.transpose(Image.FLIP_TOP_BOTTOM), f"ScreenPlayDrums lane flush {name} reverse.png")


# =====================================================================
# 演奏の画面: 左の板・ゲージ・数字・進み具合・判定の線・練習の合格の札
# =====================================================================

# ---- 判定の線 ScreenPlayDrums hit-bar.png（8×8 の上 6 段を横に引き伸ばす）
hb = Image.new("RGBA", (8, 8), (0, 0, 0, 0))
d = ImageDraw.Draw(hb)
d.line((0, 1, 7, 1), fill=(230, 255, 190, 255))
d.rectangle((0, 2, 7, 4), fill=LIME + (60,))
d.line((0, 5, 7, 5), fill=(190, 255, 130, 220))
save(hb, "ScreenPlayDrums hit-bar.png")

# ---- 左の板 7_SkillPanel.png（257×439。play_view.rs の draw_skill_panel）
# 名前は (x-2, 26)、判定の行は y 72 + 30k、数は x 80〜160、割合は x 167〜247、
# 難易度の札は (14, 266) の 60 角、達成率は (58, 277)、スキルは (88, 363)
W, H = 257, 439
sp_img, sp_mask = panel(W, H, (2, 2, 254, 428), radius=8, fill=(15, 15, 19), alpha=228)
band, _ = panel(W, H, (6, 8, 250, 58), radius=4, fill=(7, 7, 9), alpha=235, rough=1.0)
sp_img.alpha_composite(band)
spray_line(sp_img, outline_box(W, H, (2, 2, 254, 428), 8, 1), LIME, alpha=110, over=0.3)
d = ImageDraw.Draw(sp_img)
labels = [("Perfect", JUDGE[0][1]), ("Great", JUDGE[1][1]), ("Good", JUDGE[2][1]), ("Ok", JUDGE[3][1]),
          ("Miss", JUDGE[4][1]), ("Max Combo", LIME)]
for k, (text, col) in enumerate(labels):
    y = 72 + k * 30
    text_on(sp_img, (10, y + 13), text, BOLD_IT, 15 if text != "Max Combo" else 13, col, stroke=2, anchor="lm")
    for x in range(12, 246, 4):  # 行の下の点線
        d.point((x, y + 27), fill=(255, 255, 255, 40))


def rule(img, y, x0=8, x1=248, col=LIME, alpha=170):
    m = Image.new("L", img.size, 0)
    ImageDraw.Draw(m).line((x0, y, x1, y), fill=255, width=2)
    a = np.asarray(m, np.float32) * (0.75 + 0.25 * smooth_noise(img.width, img.height, 6))
    spray_line(img, Image.fromarray(a.astype(np.uint8), "L"), col, alpha=alpha, over=0.5)


rule(sp_img, 258)
rule(sp_img, 350)
sk = Image.new("RGBA", (W, H), (0, 0, 0, 0))
text_on(sk, (12, 386), "SKILL", SPRAY, 24, LIME, stroke=3, anchor="lm", shadow=(2, 3, 2, VIOLET, 160))
sp_img.alpha_composite(sk)
save(sp_img, "7_SkillPanel.png")

# ---- 難易度の札 7_Difficulty.png（60×720 に 60 角が 12。今は 1 つめの DTX だけを使う）
DIFF7 = ["DTX", "DTX", "NOVICE", "REGULAR", "EXPERT", "MASTER", "BASIC", "ADVANCED", "EXTREME", "RAW", "RWS", "REAL"]
sheet = Image.new("RGBA", (60, 720), (0, 0, 0, 0))
for i, name in enumerate(DIFF7):
    col = DIFF[name]
    cell, m = panel(60, 60, (2, 3, 57, 57), radius=4, fill=(10, 10, 13), alpha=240, rough=0.8)
    spray_line(cell, outline_box(60, 60, (2, 3, 57, 57), 4, 1, rough=0.6), col, alpha=230, over=0.4)
    tag = rough_mask(60, 60, (4, 4, 55, 16), 2, rough=0.6)
    t = Image.new("RGBA", (60, 60), col + (0,))
    t.putalpha(tag)
    cell.alpha_composite(t)
    text_on(cell, (30, 10.5), name, BLACK, 9 if len(name) < 7 else 7, INK, anchor="mm")
    sheet.alpha_composite(cell, (0, i * 60))
save(sheet, "7_Difficulty.png")

# ---- 数字の絵
WHITE_NUM = dict(fill=WHITE, stroke=2, shadow=(1, 2, 1, (0, 0, 0), 160))
# 7_LevelNumber.png（16×32、点は x 160 の幅 5）
s = Image.new("RGBA", (165, 32), (0, 0, 0, 0))
glyphs(s, row_cells(DIGITS, 16, 32, extra=[(".", 160, 5)]), XBOLD_IT, 25, xscale=0.82, **WHITE_NUM)
save(s, "7_LevelNumber.png")
# 7_Ratenumber_l.png（28×42、点は x 280 の幅 10）
s = Image.new("RGBA", (290, 42), (0, 0, 0, 0))
glyphs(s, row_cells(DIGITS, 28, 42, extra=[(".", 280, 10)]), XBOLD_IT, 44, xscale=0.78, **WHITE_NUM)
save(s, "7_Ratenumber_l.png")
# 7_Ratenumber_s.png（20×26、% は x 200、点は x 210）
s = Image.new("RGBA", (220, 26), (0, 0, 0, 0))
glyphs(s, row_cells(DIGITS, 20, 26, extra=[("%", 200, 20)]), XBOLD_IT, 26, xscale=0.8, **WHITE_NUM)
glyphs(s, [(".", 210, 0, 10, 26)], XBOLD_IT, 26, **WHITE_NUM)
save(s, "7_Ratenumber_s.png")
# 7_RatePercent_l.png（28×32）
s = Image.new("RGBA", (28, 32), (0, 0, 0, 0))
glyphs(s, [("%", 0, 0, 28, 32)], XBOLD_IT, 26, **WHITE_NUM)
save(s, "7_RatePercent_l.png")
# 7_skill max.png（100×42）
s = Image.new("RGBA", (100, 42), (0, 0, 0, 0))
text_on(s, (50, 21), "MAX", BLACK_IT, 36, GOLD, stroke=3, anchor="mm", shadow=(2, 3, 2, VIOLET, 170),
        grad=((255, 244, 160), (255, 170, 30)))
save(s, "7_skill max.png")
# 7_score numbersGD.png（36×50 の数字、「SCORE」は (0, 50, 86, 28)）
s = Image.new("RGBA", (360, 78), (0, 0, 0, 0))
glyphs(s, row_cells(DIGITS, 36, 50), BLACK_IT, 46, xscale=0.86, fill=WHITE, stroke=3,
       shadow=(2, 3, 2, VIOLET, 190), grad=(WHITE, (205, 215, 230)))
text_on(s, (4, 64), "SCORE", SPRAY, 22, LIME, stroke=3, anchor="lm", shadow=(1, 2, 1, VIOLET, 160))
save(s, "7_score numbersGD.png")
# 7_lag numbers.png（15×19 の升が 4 列。0〜9・?・-。(0,0) と (64,64) を使う。早いが青、遅いが赤）
s = Image.new("RGBA", (128, 128), (0, 0, 0, 0))
for (bx, by), col in [((0, 0), (110, 200, 255)), ((64, 0), (255, 220, 60)), ((0, 64), (255, 150, 50)), ((64, 64), (255, 90, 90))]:
    cells = [(ch, bx + (i % 4) * 15, by + (i // 4) * 19, 15, 19) for i, ch in enumerate(DIGITS + "?-")]
    glyphs(s, cells, BLACK, 15, xscale=0.9, fill=col, stroke=2)
save(s, "7_lag numbers.png")

# ---- 落ちる速さのアイコン 7_panel_icons.jpg（42×48 の段。0.5〜10.5 の 21 段。32×32 に縮めて描かれる。v2 は 8.0 までの 16 段だった）
s = Image.new("RGBA", (42, 48 * 21), (0, 0, 0, 255))
for i in range(21):
    cell = Image.new("RGBA", (42 * 4, 48 * 4), (8, 8, 10, 255))
    text_on(cell, (84, 34), "SPEED", BLACK_IT, 40, LIME, anchor="mm")
    value = f"{(i + 1) * 0.5:.1f}"
    text_on(cell, (84, 120), value, BLACK_IT, 92 if len(value) == 3 else 70, WHITE, anchor="mm")
    s.alpha_composite(cell.resize((42, 48), Image.LANCZOS), (0, i * 48))
save(s, "7_panel_icons.jpg")

# ---- ゲージ 7_Gauge.png（543×94。上の半分が奥、下の半分が手前。棒は (20, 9) から 480×31、速さのアイコンは右の (506, 8)）
W = 543
g_img = Image.new("RGBA", (W, 94), (0, 0, 0, 0))
back, bm = panel(W, 47, (1, 1, 541, 46), radius=6, fill=(14, 14, 17), alpha=240)
well, _ = panel(W, 47, (17, 6, 503, 42), radius=4, fill=(4, 4, 6), alpha=255, rough=0.6)
back.alpha_composite(well)
box, _ = panel(W, 47, (505, 5, 540, 42), radius=4, fill=(6, 6, 8), alpha=255, rough=0.6)
back.alpha_composite(box)
g_img.alpha_composite(back, (0, 0))
front = Image.new("RGBA", (W, 47), (0, 0, 0, 0))
spray_line(front, outline_box(W, 47, (17, 6, 503, 42), 4, 1, rough=0.8), LIME, alpha=220, over=0.5)
spray_line(front, outline_box(W, 47, (1, 1, 541, 46), 6, 1, rough=1.0), (60, 60, 70), alpha=200, over=0.2)
g_img.alpha_composite(front, (0, 47))
save(g_img, "7_Gauge.png")


# ---- ゲージの棒（504×31。左の 480 を使い、ゲージの量で横に縮める。jpg は満タンのとき）
def gauge_bar(c0, c1, sparkle, opaque=False):
    w, h = 504, 31
    x = np.linspace(0, 1, w)[None, :, None]
    y = np.linspace(0, 1, h)[:, None, None]
    col = np.array(c0, np.float32) * (1 - x) + np.array(c1, np.float32) * x
    col = col * (1.12 - 0.35 * y)  # 上が明るい
    col = col + (noise(w, h, 18) - 9)[..., None]
    arr = np.concatenate([col.clip(0, 255), np.full((h, w, 1), 255.0)], axis=2).astype(np.uint8)
    im = Image.fromarray(arr, "RGBA")
    im.alpha_composite(speckle(w, h, sparkle, 0.02, 170, blur=0.3, size=(0.6, 1.5)))
    d = ImageDraw.Draw(im)
    d.line((0, 2, w, 2), fill=lighten(c1, 0.6) + (150,))
    if opaque:
        return im.convert("RGB")
    return im


save(gauge_bar((40, 150, 40), LIME, (230, 255, 200)), "7_gauge_bar.png")
save(gauge_bar((255, 150, 20), (255, 236, 110), (255, 255, 230), opaque=True), "7_gauge_bar.jpg")

# ---- 進み具合の枠 7_Drum_Progress_bg.png（32×720。棒は (2, 15) から 20×540）
pb, pm = panel(32, 720, (0, 1, 31, 573), radius=8, fill=(12, 12, 15), alpha=235, rough=1.0)
spray_line(pb, outline_box(32, 720, (0, 1, 31, 573), 8, 1, rough=0.8), LIME, alpha=150, over=0.4)
save(pb, "7_Drum_Progress_bg.png")

# ---- ジャケットの板 7_JacketPanel.png（v2 でも何も描いていない。349×347 の透明）
save(Image.new("RGBA", (349, 347), (0, 0, 0, 0)), "7_JacketPanel.png")
# ---- 100 コンボごとの爆発 7_combobomb.png（v2 では黒＝加算で何も出ない。同じ大きさの黒）
save(Image.new("RGB", (360, 4760), (0, 0, 0)), "7_combobomb.png")


# =====================================================================
# 画像生成の絵（badges・result_words）を切り出す
# =====================================================================
# badges.png は背景が透けずに壁の絵で来たので、明るさから透明度を作る（壁は暗く、塗料は明るい）。
# 中心は目で測った値（3×3 の格子。8 番目のメダルだけ大きいので四角で切る）
BADGE_AT = {"allmusic": (393, 157), "artist": (767, 157), "difficulty": (1149, 157), "level": (393, 436),
            "player": (767, 436), "title": (1149, 436), "skill": (393, 757), "chevron": (1149, 757)}
MEDAL_BOX = (575, 575, 960, 1000)
badges = np.asarray(Image.open(os.path.join(RAW3, "badges.png")).convert("RGB"), np.float32) / 255


def key_out(box, circle=None):
    x0, y0, x1, y1 = box
    sub = badges[y0:y1, x0:x1]
    v = sub.max(axis=2)
    wall = np.median(sub[v < np.percentile(v, 30)], axis=0)
    a = np.clip((v - 0.20) / 0.32, 0, 1)
    if circle:
        cx, cy, r = circle
        yy, xx = np.mgrid[y0:y1, x0:x1]
        a *= np.clip((r - np.hypot(xx - cx, yy - cy)) / 12, 0, 1)
    col = (sub - wall * (1 - a[..., None])) / np.maximum(a[..., None], 1e-3)
    out = np.dstack([np.clip(col, 0, 1), a])
    return crop_alpha(Image.fromarray((out * 255).astype(np.uint8), "RGBA"), 20)


BADGE = {k: key_out((cx - 140, cy - 140, cx + 140, cy + 140), (cx, cy, 140)) for k, (cx, cy) in BADGE_AT.items()}
MEDAL = key_out(MEDAL_BOX)

# result_words.png は透明の地に 3 つの文字が縦に並ぶ（上から EXCELLENT、FULL COMBO、STAGE CLEARED）
words = Image.open(os.path.join(RAW3, "result_words.png")).convert("RGBA")
rows = np.asarray(words.getchannel("A")).max(axis=1) > 24
spans, start = [], None
for y, on in enumerate(list(rows) + [False]):
    if on and start is None:
        start = y
    elif not on and start is not None:
        if y - start > 40:
            spans.append((start, y))
        start = None
assert len(spans) == 3, spans
WORD = {k: crop_alpha(words.crop((0, a, words.width, b)), 24) for k, (a, b) in zip(["excellent", "fullcombo", "cleared"], spans)}

# =====================================================================
# 選曲の画面
# =====================================================================

# ---- 曲の帯（638×82）・BOX の帯（638×78、閉じた BOX と、上の階層へ戻る帯）
# 帯の左の端の真ん中にランプ、x 47〜113 に小さいジャケット、x 118 から曲名（黒い字に白い縁）
def bar_board(w, h, fill, outline, alpha=238):
    im, m = panel(w, h, (2, 2, w - 3, h - 3), radius=6, fill=fill, alpha=alpha, rough=1.4, texture=12, grain=True)
    spray_line(im, outline_box(w, h, (2, 2, w - 3, h - 3), 6, 1), outline, alpha=150, over=0.35)
    return im


save(bar_board(638, 82, (18, 18, 22), (90, 90, 100)), "5_bar.png")


def chevrons(im, x, cy, col, n=3):
    d = ImageDraw.Draw(im)
    for i in range(n):
        x0 = x + i * 11
        d.polygon([(x0, cy - 7), (x0 + 8, cy), (x0, cy + 7), (x0 + 3, cy)], fill=col + (255,))


box_closed = bar_board(638, 78, (64, 30, 128), LIME)
box_closed.alpha_composite(masked(speckle(638, 78, (170, 120, 255), 0.004, 120), rough_mask(638, 78, (2, 2, 635, 75), 6)))
chevrons(box_closed, 26, 39, LIME)
save(box_closed, "5_box_closed.png")
box_open = bar_board(638, 78, (150, 205, 60), (230, 255, 190))
box_open.alpha_composite(masked(speckle(638, 78, (210, 255, 140), 0.004, 140), rough_mask(638, 78, (2, 2, 635, 75), 6)))
chevrons(box_open, 26, 39, (40, 60, 20))
save(box_open, "5_box_open.png")

# ---- スキルの帯（380×18 の枠と、282×10 の中身。中身は (79, 4) から 286×8 に引き伸ばされる。数は x 23 から）
sb = Image.new("RGBA", (380, 18), (0, 0, 0, 0))
paste_center(sb, fit(BADGE["chevron"], 17, 17), 9, 9)
d = ImageDraw.Draw(sb)
d.rectangle((77, 3, 367, 14), fill=(6, 6, 8, 200), outline=(235, 235, 235, 210))
save(sb, "5_skillbar.png")
fill_ = Image.new("RGBA", (282, 10), (0, 0, 0, 0))
x = np.linspace(0, 1, 282)[None, :, None]
arr = np.array((60, 190, 50), np.float32) * (1 - x) + np.array((210, 255, 80), np.float32) * x
arr = np.repeat(arr, 10, axis=0) * np.linspace(1.15, 0.85, 10)[:, None, None]
fill_ = Image.fromarray(np.concatenate([arr.clip(0, 255), np.full((10, 282, 1), 255.0)], axis=2).astype(np.uint8), "RGBA")
save(fill_, "5_skillbar_fill.png")

# ---- ランプ（28×86。0 はまだクリアしていない、1〜5 はクリアした一番上の難易度 BASIC〜DTXMANIA）
for i, col in enumerate([(52, 52, 60), DIFF["BASIC"], DIFF["ADVANCED"], DIFF["EXTREME"], DIFF["MASTER"], DIFF["DTXMANIA"]]):
    lp = Image.new("RGBA", (28, 86), (0, 0, 0, 0))
    if i:
        lp.alpha_composite(mist(28, 86, 14, 43, 11, 38, col, 170))
    d = ImageDraw.Draw(lp)
    d.rounded_rectangle((7, 5, 20, 80), radius=6, fill=INK + (255,))
    d.rounded_rectangle((8, 6, 19, 79), radius=5, fill=(col if i else (34, 34, 40)) + (255,))
    if i:
        d.rounded_rectangle((10, 9, 12, 60), radius=1, fill=lighten(col, 0.7) + (230,))
        lp.alpha_composite(masked(speckle(28, 86, lighten(col, 0.4), 0.05, 180, blur=0.3, size=(0.6, 1.4)),
                                  rough_mask(28, 86, (7, 5, 20, 80), 6, 0)))
    save(lp, f"Lamp/{i:02}.png")

# ---- ランク（64×64。0 が SS、6 が E。第 1 段階の結果のランクの絵を縮める）と、スキルの対象の印
for i, r in enumerate(["SS", "S", "A", "B", "C", "D", "E"]):
    src_ = crop_alpha(Image.open(os.path.join(OUT, f"8_rank{r}.png")).convert("RGBA"))
    rk = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    paste_center(rk, fit(src_, 64, 64), 32, 32)
    save(rk, f"Rank/rank_{i}.png")
rk = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
paste_center(rk, fit(BADGE["chevron"], 62, 62), 32, 32)
save(rk, "Rank/skill.png")

# ---- 並べ替えのアイコン（90×70。上に 50 角の絵、下に名前。おすすめは第 1 段階で作ったもの）
SORT_LABEL = {"allmusic": "All Songs", "artist": "Artist", "difficulty": "Difficulty", "level": "Level",
              "player": "Player", "title": "Title", "skill": "Skill"}
for key, label in SORT_LABEL.items():
    ic = Image.new("RGBA", (90, 70), (0, 0, 0, 0))
    b = fit(BADGE[key], 50, 50)
    ic.alpha_composite(b, (45 - b.width // 2, 2))
    text_on(ic, (45, 60), label, SEMI, 12, (255, 226, 40), stroke=2, anchor="mm")
    save(ic, f"Sorting/{key}.png")

# ---- 並べ替えメニューの地（662×92）と、上の帯（320×36 を横 4 倍に引き伸ばす）
sm, smm = panel(662, 92, (0, 0, 662, 92), radius=0, fill=(10, 10, 13), alpha=215, rough=0)
a = np.asarray(sm.getchannel("A"), np.float32) * np.linspace(0.7, 1.0, 92)[:, None]
sm.putalpha(Image.fromarray(a.astype(np.uint8), "L"))
sm.alpha_composite(mist(662, 92, 331, 96, 360, 18, LIME, 120))
rule(sm, 89, 0, 662, LIME, 230)
sm.alpha_composite(masked(speckle(662, 92, VIOLET, 0.003, 150), Image.fromarray((np.linspace(0, 255, 92)[:, None].repeat(662, 1)).astype(np.uint8), "L")))
save(sm, "5_sortmenu_bg.png")
tb = Image.new("RGBA", (320, 36), (0, 0, 0, 0))
arr = np.zeros((36, 320, 4), np.uint8)
arr[..., 0:3] = 6
arr[..., 3] = np.linspace(235, 200, 36)[:, None].astype(np.uint8)
tb = Image.fromarray(arr, "RGBA")
d = ImageDraw.Draw(tb)
d.line((0, 33, 320, 33), fill=LIME + (255,), width=2)
d.line((0, 35, 320, 35), fill=VIOLET + (200,), width=1)
save(tb, "SongSelect/top_bar.png")

# ---- 密度のグラフ（板 160×342、中の筋 112×286。筋は x 5 + 12×レーン の幅 6、y 0〜270。数は右寄せで (149, 316)）
GRAPH_LANES = [(219, 112, 147), (0, 191, 255), (255, 105, 180), (255, 255, 0), (0, 128, 0),
               (147, 112, 219), (255, 0, 0), (255, 165, 0), (0, 191, 255)]  # select.rs の LANE_COLORS
gp, _ = panel(160, 342, (2, 2, 157, 339), radius=6, fill=(12, 12, 15), alpha=232)
gp.alpha_composite(mist(160, 342, 90, 300, 70, 40, LIME, 50))
spray_line(gp, outline_box(160, 342, (2, 2, 157, 339), 6, 1), LIME, alpha=140, over=0.35)
for text, y in (("END", 20), ("START", 282)):
    lab = Image.new("RGBA", (80, 20), (0, 0, 0, 0))
    text_on(lab, (40, 10), text, BLACK, 11, (200, 200, 210), anchor="mm")
    lab = crop_alpha(lab.rotate(90, expand=True, resample=Image.BICUBIC), 20)
    gp.alpha_composite(lab, (12, y if text == "END" else y - lab.height))
text_on(gp, (149, 304), "TOTAL NOTES", BLACK, 10, LIME, anchor="rm")
save(gp, "SongSelect/graph_panel_drums.png")
gf = Image.new("RGBA", (112, 286), (0, 0, 0, 0))
d = ImageDraw.Draw(gf)
for i, col in enumerate(GRAPH_LANES):
    x0 = 5 + 12 * i
    d.rectangle((x0, 0, x0 + 5, 269), fill=(4, 4, 6, 230))
    d.ellipse((x0 - 1, 275, x0 + 6, 282), fill=col + (255,))
save(gf, "SongSelect/graph_fg_drums.png")

# ---- 音符の数の数字 5_bpm font.png（12×20）
s = Image.new("RGBA", (132, 20), (0, 0, 0, 0))
glyphs(s, row_cells(DIGITS, 12, 20), BLACK, 16, xscale=0.9, fill=WHITE, stroke=2)
save(s, "5_bpm font.png")

# ---- 難易度の板 5_difficulty_panel.png（215×387。下から BASIC・ADVANCED・EXTREME・MASTER・DTXMANIA の 5 段、74 ずつ）
# 段 i の下の端を B = 387 - 74i として、ランクとスキルの印は (60, B-76) と (14, B-76) の 27 角、
# レベルの数は (125, B-41) から、達成率は (23, B-30) から
dp, _ = panel(215, 387, (1, 4, 213, 386), radius=8, fill=(8, 8, 10), alpha=215)
for i, name in enumerate(["BASIC", "ADVANCED", "EXTREME", "MASTER", "DTXMANIA"]):
    B = 387 - 74 * i
    col = DIFF[name]
    card, cm = panel(215, 387, (6, B - 80, 209, B - 9), radius=5, fill=(20, 20, 25), alpha=245, rough=1.0)
    stripe = Image.new("RGBA", (215, 387), col + (0,))
    stripe.putalpha(ImageChops.multiply(cm, rough_mask(215, 387, (6, B - 80, 11, B - 9), 2, 0.6)))
    card.alpha_composite(stripe)
    spray_line(card, outline_box(215, 387, (6, B - 80, 209, B - 9), 5, 1, rough=0.8), col, alpha=200, over=0.4)
    well, _ = panel(215, 387, (110, B - 56, 205, B - 12), radius=4, fill=(6, 6, 8), alpha=255, rough=0.6)
    card.alpha_composite(well)
    d = ImageDraw.Draw(card)
    for x0 in (14, 60):  # ランクとスキルの印を置く場所
        d.rounded_rectangle((x0, B - 76, x0 + 27, B - 49), radius=3, outline=(255, 255, 255, 50))
    text_on(card, (203, B - 68), name, BLACK_IT, 15, col, stroke=2, anchor="rm")
    text_on(card, (16, B - 40), "RATE", BLACK, 9, (170, 170, 180), anchor="lm")
    dp.alpha_composite(card)
save(dp, "5_difficulty_panel.png")

# ---- 選んでいる難易度の枠 5_difficultyframe.png（226×98。段の 7 左、下の端から 5 下に置かれる）
fr = Image.new("RGBA", (226, 98), (0, 0, 0, 0))
fr.alpha_composite(mist(226, 98, 113, 49, 112, 48, LIME, 60))
fr_edge = outline_box(226, 98, (5, 6, 220, 92), 7, 2, rough=1.2)
spray_line(fr, fr_edge, LIME, alpha=255, over=0.9)
hole = rough_mask(226, 98, (9, 10, 216, 88), 5, 0)
fr.putalpha(ImageChops.subtract(fr.getchannel("A"), hole.point(lambda v: v * 200 // 255)))
spray_line(fr, fr_edge, LIME, alpha=255, over=0.0)
save(fr, "5_difficultyframe.png")

# ---- レベルの数字 5_level number.png（2048×128。select.rs の LEVEL_GLYPHS を 128/28 倍した升）
k = 128 / 28
s = Image.new("RGBA", (2048, 128), (0, 0, 0, 0))
cells = [(ch, round(x * k), 0, round(w * k), 128) for ch, x, w in
         [(c_, 20 * i, 20) for i, c_ in enumerate(DIGITS)] + [(".", 200, 10), ("-", 210, 20), ("?", 230, 20), ("%", 250, 20)]]
glyphs(s, cells, XBOLD_IT, 104, xscale=0.85, ss=1, fill=WHITE, stroke=7, shadow=(3, 6, 4, (0, 0, 0), 170))
save(s, "5_level number.png")


# =====================================================================
# 結果の画面
# =====================================================================
# ---- クリアの文字（ランクの絵の下に重なる。v2 の文字の場所に合わせる）
for name, key, size, center, box in [
    ("ScreenResult Excellent.png", "excellent", (520, 190), (299, 125), (300, 118)),
    ("ScreenResult fullcombo.png", "fullcombo", (520, 190), (302, 120), (300, 110)),
    ("ScreenResult StageCleared.png", "cleared", (600, 190), (302, 132), (240, 112)),
]:
    im = Image.new("RGBA", size, (0, 0, 0, 0))
    paste_center(im, fit(WORD[key], *box), *center)
    save(im, name)

# ---- NEW RECORD（90×12）
s = Image.new("RGBA", (90 * 4, 12 * 4), (0, 0, 0, 0))
text_on(s, (180, 24), "NEW RECORD", BLACK_IT, 40, GOLD, stroke=5, anchor="mm", grad=((255, 244, 160), (255, 170, 30)))
save(s.resize((90, 12), Image.LANCZOS), "8_New Record.png")

# ---- 進み具合の枠 8_progress_bar_panel.png（37×427。棒は x 1〜13 と 15〜19、高さ 425。右に END と START）
pp = Image.new("RGBA", (37, 427), (0, 0, 0, 0))
d = ImageDraw.Draw(pp)
d.rectangle((0, 0, 36, 426), fill=(12, 12, 15, 225))
for x in (0, 14, 20, 36):
    d.line((x, 0, x, 426), fill=(255, 255, 255, 90))
d.line((0, 0, 36, 0), fill=(255, 255, 255, 90))
d.line((0, 426, 36, 426), fill=(255, 255, 255, 90))
for text, top in (("END", True), ("START", False)):
    lab = Image.new("RGBA", (120, 20), (0, 0, 0, 0))
    text_on(lab, (60, 10), text + " »", BLACK, 11, LIME, anchor="mm")
    lab = crop_alpha(lab.rotate(-90, expand=True, resample=Image.BICUBIC), 20)
    pp.alpha_composite(lab, (28 - lab.width // 2, 6 if top else 420 - lab.height))
save(pp, "8_progress_bar_panel.png")

# =====================================================================
# 読み込みの画面
# =====================================================================
# ---- 難易度の帯 6_Difficulty.png（262×600 に 262×50 が 12 段。今は 1 段目の DTXMANIA だけを使う）
DIFF6 = ["DTXMANIA", "DTXMANIA", "NOVICE", "REGULAR", "EXPERT", "MASTER", "BASIC", "ADVANCED", "EXTREME", "RAW", "RWS", "REAL"]
sheet = Image.new("RGBA", (262, 600), (0, 0, 0, 0))
for i, name in enumerate(DIFF6):
    col = DIFF[name]
    row = Image.new("RGBA", (262, 50), (0, 0, 0, 0))
    m = Image.new("L", (262, 50), 0)
    ImageDraw.Draw(m).polygon([(0, 4), (196, 4), (184, 46), (0, 46)], fill=255)
    a = np.asarray(m.filter(ImageFilter.GaussianBlur(1.2)), np.float32) / 255
    a = np.clip((a - 0.5 + (smooth_noise(262, 50, 3) - 0.5) * 0.35) * 6 + 0.5, 0, 1)
    m = Image.fromarray((a * 255).astype(np.uint8), "L")
    body = np.zeros((50, 262, 3), np.float32) + col
    body *= (0.92 + 0.16 * smooth_noise(262, 50, 12))[..., None]
    tape = Image.fromarray(body.clip(0, 255).astype(np.uint8)).convert("RGBA")
    tape.putalpha(m)
    row.alpha_composite(mist(262, 50, 100, 26, 100, 20, col, 90))
    row.alpha_composite(tape)
    row.alpha_composite(masked(speckle(262, 50, lighten(col, 0.5), 0.01, 160, blur=0.3, size=(0.6, 1.6)), m))
    text_on(row, (12, 26), name, BLACK_IT, 30, INK, anchor="lm")
    sheet.alpha_composite(row, (0, i * 50))
save(sheet, "6_Difficulty.png")

# ---- 大きいレベルの数字 6_LevelNumber.png（100×130 が横に 10、点は (1000, 92, 30, 38)）
s = Image.new("RGBA", (1030, 130), (0, 0, 0, 0))
glyphs(s, row_cells(DIGITS, 100, 130), BLACK_IT, 124, xscale=0.82, ss=2, fill=WHITE, stroke=6,
       shadow=(4, 6, 3, VIOLET, 200), grad=(WHITE, (215, 225, 240)))
dot = Image.new("RGBA", (30, 38), (0, 0, 0, 0))
d = ImageDraw.Draw(dot)
d.ellipse((5, 12, 25, 32), fill=INK + (255,))
d.ellipse((8, 15, 22, 29), fill=WHITE + (255,))
s.alpha_composite(dot, (1000, 92))
save(s, "6_LevelNumber.png")

# ---- パートの板 6_Part.png（v2 でも何も描いていない。262×150 の透明）
save(Image.new("RGBA", (262, 150), (0, 0, 0, 0)), "6_Part.png")

# =====================================================================
# タイトル・設定・共通
# =====================================================================
# ---- タイトルのメニュー 2_menu.png（227×39 の段。0 Start、1 Options（描かない）、2 Config、3 Quit、4 カーソル、5 カーソルの光）
mn = Image.new("RGBA", (256, 256), (0, 0, 0, 0))
for i, text in enumerate(["Start", "Options", "Config", "Quit"]):
    text_on(mn, (113, i * 39 + 20), text, SPRAY, 28, WHITE, stroke=3, anchor="mm", shadow=(2, 3, 2, VIOLET, 170))
cur = Image.new("RGBA", (227, 39), (0, 0, 0, 0))
cm = rough_mask(227, 39, (10, 5, 216, 33), 8, 1.6)
band_ = Image.new("RGBA", (227, 39), LIME + (0,))
band_.putalpha(cm.filter(ImageFilter.GaussianBlur(2)).point(lambda v: v * 110 // 255))
cur.alpha_composite(band_)
spray_line(cur, edge_of(cm, 1), LIME, alpha=230, over=0.5)
mn.alpha_composite(cur, (0, 39 * 4))
fl = Image.new("RGBA", (227, 39), (0, 0, 0, 0))
fl.alpha_composite(mist(227, 39, 113, 19, 110, 16, (220, 255, 180), 230))
mn.alpha_composite(fl, (0, 39 * 5))
save(mn, "2_menu.png")

# ---- 設定の背景 4_background.png（第 1 段階の壁の絵を少しぼかして暗く。上に暗い幕が重なる）
bg = Image.open(os.path.join(OUT, "1_background.jpg")).convert("RGB").resize((1280, 720), Image.LANCZOS)
bg = ImageEnhance.Brightness(bg.filter(ImageFilter.GaussianBlur(3))).enhance(0.8)
save(bg, "4_background.png")

# ---- 設定の上の帯 4_header panel.png（1280×105。右上に小さい札）と下の帯 4_footer panel.png（v2 でも透明）
hp = Image.new("RGBA", (1280, 105), (0, 0, 0, 0))
tag, tm = panel(1280, 105, (1146, 16, 1270, 50), radius=4, fill=(14, 14, 18), alpha=235, rough=1.2)
spray_line(tag, edge_of(tm, 1), LIME, alpha=200, over=0.5)
text_on(tag, (1208, 33), "FREE PLAY", SPRAY, 18, LIME, stroke=2, anchor="mm")
hp.alpha_composite(tag)
save(hp, "4_header panel.png")
save(Image.new("RGBA", (1280, 30), (0, 0, 0, 0)), "4_footer panel.png")

# ---- 英数字の文字 Screen font dfp.png と強調の dfp em.png（256×256。gfx.rs の DFP の表の升に 1 文字ずつ）
DFP_SRC = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "crates", "app", "src", "gfx.rs"), encoding="utf-8").read()


def dfp_table():
    """gfx.rs の DFP の表（('A', src(177.0, 59.0, 23.0, 27.0), 18.0) の形）をそのまま読む"""
    import re
    body = DFP_SRC.split("pub const DFP", 1)[1].split("};", 1)[0]
    out = []
    for m in re.finditer(r"\('(\\'|\\\\|[^'])', src\(([\d.]+), ([\d.]+), ([\d.]+), ([\d.]+)\)", body):
        ch = m.group(1).replace("\\'", "'").replace("\\\\", "\\")
        out.append((ch, *(int(float(v)) for v in m.groups()[1:])))
    return out


DFP = dfp_table()
assert len(DFP) > 80 and DFP[0] == (" ", 10, 3, 13, 27), DFP[:3]
for name, col in (("Screen font dfp.png", WHITE), ("Screen font dfp em.png", LIME)):
    s = Image.new("RGBA", (256, 256), (0, 0, 0, 0))
    cells = [(ch, x_, y_, w_, h_) for ch, x_, y_, w_, h_ in DFP if ch != " "]
    glyphs(s, cells, BLACK_IT, 21, ref="H", fill=col, stroke=2)
    save(s, name)

# ---- シャッターに出る小さいロゴ logo_small.png（1588×275 を 412×71 に縮めて描かれる）
logo = crop_alpha(Image.open(os.path.join(RAW1, "logo_a.png")).convert("RGBA"))
ls = Image.new("RGBA", (1588, 275), (0, 0, 0, 0))
paste_center(ls, fit(logo, 1560, 268), 794, 137)
save(ls, "logo_small.png")

# =====================================================================
# 練習の合格の札 7_Excellent.png（1280×720 の (402, 274) から 480×177 を切り出して使う）
# =====================================================================
ex = Image.new("RGBA", (1280, 720), (0, 0, 0, 0))
md = fit(MEDAL, 150, 172)
paste_center(ex, md, 402 + 80, 274 + 88)
paste_center(ex, fit(WORD["excellent"], 318, 120), 402 + 160 + 158, 274 + 92)
save(ex, "7_Excellent.png")

print("done")
