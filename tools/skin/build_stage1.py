"""生成した絵から、第 1 段階のスキンのファイル（assets/skin/Graphics の 23 個）を組む。
生成の費用はかからない（切り抜き・縮小・文字入れだけ）。
使い方: python3 tools/skin/build_stage1.py <生成した絵の dir> <生成した絵の dir> assets/skin/Graphics
生成した絵の名前は prompts.json の name に .png を付けたもの（bg_a.png、ranks.png など）。
文字は IPA ゴシック（fonts-ipafont-gothic）と Inter で描く。
"""
import sys, os
import numpy as np
from PIL import Image, ImageFilter, ImageEnhance, ImageDraw, ImageFont

GEN, GEN2, OUT = sys.argv[1:4]
os.makedirs(os.path.join(OUT, 'SongSelect'), exist_ok=True)
os.makedirs(os.path.join(OUT, 'Sorting'), exist_ok=True)
JP = '/usr/share/fonts/truetype/fonts-japanese-gothic.ttf'

def font(path, size):
    return ImageFont.truetype(path, size)

def en_font(size):
    import subprocess
    p = subprocess.run(['fc-match', '-f', '%{file}', 'Inter:semibold'], capture_output=True, text=True).stdout
    return ImageFont.truetype(p, size)

def save(im, rel, **kw):
    p = os.path.join(OUT, rel)
    if rel.endswith('.jpg'):
        im.convert('RGB').save(p, quality=90, optimize=True)
    else:
        im.save(p, optimize=True)

def xy_cut(alpha, thr=48, min_area=400):
    """透明の地に置かれた部品を、つながった塊ごとに切り出す。上の行から、行の中は左から"""
    from collections import deque
    k = 4
    a = np.asarray(alpha.resize((alpha.width // k, alpha.height // k), Image.BOX)) > thr
    h, w = a.shape
    seen = np.zeros_like(a)
    boxes = []
    for y in range(h):
        for x in range(w):
            if a[y, x] and not seen[y, x]:
                q = deque([(y, x)]); seen[y, x] = True
                y0 = y1 = y; x0 = x1 = x; n = 0
                while q:
                    cy, cx = q.popleft(); n += 1
                    y0, y1, x0, x1 = min(y0, cy), max(y1, cy), min(x0, cx), max(x1, cx)
                    for dy in (-1, 0, 1):
                        for dx in (-1, 0, 1):
                            ny, nx = cy + dy, cx + dx
                            if 0 <= ny < h and 0 <= nx < w and a[ny, nx] and not seen[ny, nx]:
                                seen[ny, nx] = True; q.append((ny, nx))
                if n * k * k >= min_area:
                    boxes.append([x0 * k, y0 * k, (x1 + 1) * k, (y1 + 1) * k])
    # 大きな塊に含まれる小さな塊（しぶき）は親に吸収する
    boxes.sort(key=lambda b: -(b[2] - b[0]) * (b[3] - b[1]))
    big = []
    for b in boxes:
        cx, cy = (b[0] + b[2]) / 2, (b[1] + b[3]) / 2
        host = next((g for g in big if g[0] - 40 <= cx <= g[2] + 40 and g[1] - 40 <= cy <= g[3] + 40), None)
        if host:
            host[0], host[1] = min(host[0], b[0]), min(host[1], b[1]); host[2], host[3] = max(host[2], b[2]), max(host[3], b[3])
        elif (b[2] - b[0]) * (b[3] - b[1]) >= 60000:
            big.append(b)
    # 行に分けて並べる
    big.sort(key=lambda b: b[1])
    rows = []
    for b in big:
        if rows and b[1] < max(r[3] for r in rows[-1]) - 40:
            rows[-1].append(b)
        else:
            rows.append([b])
    return [tuple(b) for r in rows for b in sorted(r, key=lambda b: b[0])]

def fit(im, w, h):
    r = min(w / im.width, h / im.height)
    return im.resize((max(1, round(im.width * r)), max(1, round(im.height * r))), Image.LANCZOS)

def outlined(draw, xy, text, f, fill, stroke=(0, 0, 0), sw=3, anchor='la'):
    draw.text(xy, text, font=f, fill=fill, stroke_width=sw, stroke_fill=stroke, anchor=anchor)

# 背景
bg = Image.open(os.path.join(GEN, 'bg_a.png')).convert('RGB').crop((0, 80, 1536, 944)).resize((1280, 720), Image.LANCZOS)
soft = ImageEnhance.Brightness(bg.filter(ImageFilter.GaussianBlur(2.5))).enhance(0.85)
dim = ImageEnhance.Brightness(bg.filter(ImageFilter.GaussianBlur(7))).enhance(0.72)
save(bg, '1_background.jpg'); save(bg, '2_background.jpg')
for n in ['5_background.jpg', '7_background.jpg', '8_background.jpg']:
    save(soft, n)

# 部品の切り出し
parts_im = Image.open(os.path.join(GEN2, 'parts.png')).convert('RGBA')
pb = xy_cut(parts_im.getchannel('A'))
assert len(pb) == 4, pb
strip, poster, plate, badge = [parts_im.crop(b) for b in pb]
words_im = Image.open(os.path.join(GEN2, 'words.png')).convert('RGBA')
wb = xy_cut(words_im.getchannel('A'))
assert len(wb) == 3, wb
thanks, back_w, random_w = [words_im.crop(b) for b in wb]

# タイトルのロゴとメニューの板（2_background.png は透明の上に重ねる 1 枚）
logo = Image.open(os.path.join(GEN, 'logo_a.png')).convert('RGBA')
logo = logo.crop(logo.getchannel('A').point(lambda a: 255 if a > 8 else 0).getbbox())
t = Image.new('RGBA', (1280, 720), (0, 0, 0, 0))
L = fit(logo, 1040, 400)
t.alpha_composite(L, (640 - L.width // 2, 250 - L.height // 2))
# 板は横長すぎるので、高さを合わせてから真ん中を抜いて左右をつなぐ（木目は横向きなので継ぎ目が目立たない）
PW, PH = 324, 154
p = plate.resize((round(plate.width * PH / plate.height), PH), Image.LANCZOS)
half, blend = PW // 2, 24
left = p.crop((0, 0, half + blend, PH)); right = p.crop((p.width - half - blend, 0, p.width, PH))
plate_out = Image.new('RGBA', (PW, PH), (0, 0, 0, 0))
plate_out.alpha_composite(right, (PW - right.width, 0))
mask = Image.new('L', (left.width, PH), 255)
md = ImageDraw.Draw(mask)
for i in range(blend * 2):
    md.line([(left.width - 1 - i, 0), (left.width - 1 - i, PH)], fill=int(255 * (i / (blend * 2))))
plate_out.paste(left, (0, 0), Image.composite(left, Image.new('RGBA', left.size, (0, 0, 0, 0)), mask).getchannel('A'))
t.alpha_composite(plate_out, (458, 494))
save(t, '2_background.png')

# ランク（7 枚で 1 組、各 274 角）
ranks_im = Image.open(os.path.join(GEN2, 'ranks.png')).convert('RGBA')
rb = xy_cut(ranks_im.getchannel('A'))
assert len(rb) == 7, rb
for name, b in zip(['SS', 'S', 'A', 'B', 'C', 'D', 'E'], rb):
    e = fit(ranks_im.crop(b), 262, 262)
    c = Image.new('RGBA', (274, 274), (0, 0, 0, 0))
    c.alpha_composite(e, ((274 - e.width) // 2, (274 - e.height) // 2))
    save(c, f'8_rank{name}.png')

# シャッター（558×720、上下の端まで絵）
sh = Image.open(os.path.join(GEN2, 'shutter.png')).convert('RGB')
h = round(sh.width * 720 / 558)
sh = sh.crop((0, (sh.height - h) // 2, sh.width, (sh.height - h) // 2 + h)).resize((558, 720), Image.LANCZOS)
save(sh, '7_shutter.png')

# STAGE FAILED（左右の扉の継ぎ目が x=640）
fl = Image.open(os.path.join(GEN2, 'failed.png')).convert('RGB').crop((0, 80, 1536, 944)).resize((1280, 720), Image.LANCZOS)
save(fl, '7_stage_failed.jpg')

# 選曲の左の飾りの紙
save(strip.resize((362, 134), Image.LANCZOS), 'SongSelect/back1.png')
save(poster.resize((495, 529), Image.LANCZOS), 'SongSelect/back2.png')

# 並べ替えのアイコン「おすすめ」（90×70。ほかの 9 個は第 3 段階）
ic = Image.new('RGBA', (90, 70), (0, 0, 0, 0))
bd = fit(badge, 50, 50)
ic.alpha_composite(bd, (45 - bd.width // 2, 2))
d = ImageDraw.Draw(ic)
outlined(d, (45, 60), 'Recommended', en_font(12), (255, 226, 40), sw=2, anchor='mm')
save(ic, 'Sorting/recommended.png')

# ジャケットの代わり（400 角）
def square(box, dark=0.75, blur=1.5):
    s = bg.crop(box).resize((400, 400), Image.LANCZOS).filter(ImageFilter.GaussianBlur(blur))
    return ImageEnhance.Brightness(s).enhance(dark).convert('RGBA')
sq = square((850, 290, 1280, 720), 0.8)
L = fit(logo, 370, 300); sq.alpha_composite(L, (200 - L.width // 2, 200 - L.height // 2))
save(sq.convert('RGB'), '5_preimage default.png')
sq = square((0, 290, 430, 720), 0.7)
W = fit(back_w, 360, 200); sq.alpha_composite(W, (200 - W.width // 2, 200 - W.height // 2))
save(sq.convert('RGB'), '5_preimage backbox.png')
sq = square((850, 0, 1280, 430), 0.7)
W = fit(random_w, 360, 170); sq.alpha_composite(W, (200 - W.width // 2, 150 - W.height // 2))
d = ImageDraw.Draw(sq)
outlined(d, (200, 290), 'カテゴリー内', font(JP, 46), (255, 255, 255), sw=4, anchor='mm')
save(sq.convert('RGB'), '5_preimage random.png')

# 読み込み（注意書きは自前の文）
ld = dim.convert('RGBA')
ov = Image.new('RGBA', (1280, 720), (0, 0, 0, 0)); od = ImageDraw.Draw(ov)
od.rectangle([191, 107, 483, 282], fill=(0, 0, 0, 150))
od.rectangle([0, 508, 1280, 720], fill=(0, 0, 0, 150))
ld.alpha_composite(ov)
d = ImageDraw.Draw(ld)
d.text((640, 585), '演奏の前に、スティックの届くところに人や物がないか確かめてください。', font=font(JP, 30), fill=(255, 255, 255), anchor='mm')
d.text((640, 640), 'Before you play, make sure no one and nothing is within reach of your sticks.', font=en_font(24), fill=(220, 224, 235), anchor='mm')
save(ld.convert('RGB'), '6_background.jpg')

# 終了
en = dim.convert('RGBA')
T = fit(thanks, 780, 190); en.alpha_composite(T, (1250 - T.width, 18))
d = ImageDraw.Draw(en)
outlined(d, (1240, 18 + T.height + 26), 'ゲームが終了しました', font(JP, 30), (255, 226, 40), sw=3, anchor='rm')
save(en.convert('RGB'), '9_background.jpg')
print('ok', len(os.listdir(OUT)))
