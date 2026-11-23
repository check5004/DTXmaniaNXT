//! 曲名などの文字を画像にして描く（NX が GDI で文字を画像にしてから描くのと同じ）。
//!
//! NX の既定のフォントは MS P ゴシックなので、Windows ではそれを使う。
//! 他の OS では近い日本語のフォントを探す。フォントのファイルはアプリに同梱しない。
//! 窓の大きさに合わせた細かさで画像にし、NX の座標での大きさで描く。
//!
//! 画像にするのは窓が大きいほど重い（2560×1440 で曲名 1 つ 10〜30 ミリ秒）。選曲画面の曲名のように
//! 次々に要るものは [`Texts::request`] で別のスレッドに頼み、できたものを [`Texts::ready`] で描く。

use std::collections::HashMap;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use egui::Color32;

use crate::gfx::{Canvas, Opt, Tex};
use crate::jobs::Jobs;

/// フォントの候補（ファイル、コレクションの中の名前）。上から順に探す
const CANDIDATES: &[(&str, Option<&str>)] = &[
    // Windows（NX の既定は MS PGothic）
    ("C:\\Windows\\Fonts\\msgothic.ttc", Some("MS PGothic")),
    ("C:\\Windows\\Fonts\\meiryo.ttc", None),
    ("C:\\Windows\\Fonts\\YuGothM.ttc", None),
    // macOS
    ("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", None),
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", None),
    // Linux（IPA P ゴシックは MS P ゴシックと字幅が近い）
    ("/usr/share/fonts/opentype/ipafont-gothic/ipagp.ttf", None),
    ("/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf", None),
    (
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        None,
    ),
    ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", None),
    (
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        None,
    ),
    ("/usr/share/fonts/truetype/fonts-japanese-gothic.ttf", None),
];

/// コレクション（.ttc）の中で `name` の書体が何番目か
fn face_index(data: &[u8], name: &str) -> Option<u32> {
    let n = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    (0..n).find(|&i| {
        ttf_parser::Face::parse(data, i).is_ok_and(|f| {
            f.names().into_iter().any(|nm| {
                (nm.name_id == ttf_parser::name_id::FAMILY
                    || nm.name_id == ttf_parser::name_id::FULL_NAME)
                    && nm.to_string().as_deref() == Some(name)
            })
        })
    })
}

fn load_font() -> Option<FontArc> {
    for (path, name) in CANDIDATES {
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        let index = match name {
            Some(n) => face_index(&data, n).unwrap_or(0),
            None => 0,
        };
        match ab_glyph::FontVec::try_from_vec_and_index(data, index) {
            Ok(f) => {
                log::info!("text font: {path} #{index}");
                return Some(FontArc::new(f));
            }
            Err(e) => log::warn!("{path}: {e}"),
        }
    }
    log::warn!("no font for skin text");
    None
}

/// 文字の見た目
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// NX の座標での文字の大きさ（ピクセル）
    pub px: f32,
    pub color: Color32,
    /// 太字（MS P ゴシックには太字が無いので、GDI と同じく横に太らせる）
    pub bold: bool,
    /// 影（色、右下へのずれ）
    pub shadow: Option<(Color32, f32)>,
    /// 縁取り（色、太さ）
    pub edge: Option<(Color32, f32)>,
    /// 上から下へのグラデーションの下の色（上は `color`）。NX の選んでいる項目の黄色から橙
    pub gradient: Option<Color32>,
}

impl Style {
    pub fn plain(px: f32, color: Color32) -> Self {
        Self {
            px,
            color,
            bold: false,
            shadow: None,
            edge: None,
            gradient: None,
        }
    }

    /// 縁取りした文字（NX の `DrawPrivateFont(文字, 色, 縁の色)`）
    pub fn edged(px: f32, color: Color32, edge: Color32, width: f32) -> Self {
        Self {
            edge: Some((edge, width)),
            ..Self::plain(px, color)
        }
    }
}

/// 文字の画像
#[derive(Clone)]
pub struct Text {
    tex: Tex,
    /// NX の座標での幅と高さ
    pub w: f32,
    pub h: f32,
}

impl Text {
    /// (x, y) を左上にして描く。`sx` は横の縮め方（長い曲名を枠に収める）
    pub fn draw(&self, c: &Canvas, x: f32, y: f32, sx: f32, alpha: f32) {
        c.draw_opt(
            &self.tex,
            x,
            y,
            Opt {
                sx: self.w / self.tex.w * sx,
                sy: self.h / self.tex.h,
                alpha,
                ..Default::default()
            },
        );
    }

    /// 幅が `max_w` を超えるなら横に縮めて描く（NX の曲名と同じ）
    pub fn draw_fit(&self, c: &Canvas, x: f32, y: f32, max_w: f32) {
        let sx = if self.w > max_w { max_w / self.w } else { 1.0 };
        self.draw(c, x, y, sx, 1.0);
    }
}

#[derive(Hash, PartialEq, Eq, Clone)]
struct Key {
    text: String,
    style: [u32; 9],
    raster: u32,
}

fn key(text: &str, s: &Style, raster: f32) -> Key {
    let (sc, so) = s.shadow.map_or((0, 0), |(c, o)| {
        (u32::from_le_bytes(c.to_array()), o.to_bits())
    });
    let (ec, ew) = s.edge.map_or((0, 0), |(c, w)| {
        (u32::from_le_bytes(c.to_array()), w.to_bits())
    });
    Key {
        text: text.to_owned(),
        style: [
            s.px.to_bits(),
            u32::from_le_bytes(s.color.to_array()),
            u32::from(s.bold),
            sc,
            so,
            ec,
            ew,
            s.gradient.map_or(0, |c| u32::from_le_bytes(c.to_array())),
            u32::from(s.gradient.is_some()),
        ],
        raster: (raster * 100.0).round() as u32,
    }
}

/// 別のスレッドで文字を画像にする仕事（見分けるのは `key` だけ）
#[derive(Clone)]
struct Job {
    key: Key,
    style: Style,
    raster: f32,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for Job {}

impl std::hash::Hash for Job {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

/// 文字を画像にするスレッドの数
const THREADS: usize = 2;
/// これだけのフレームのあいだ頼み直されなかった仕事は捨てる（通り過ぎた曲名）
const STALE_FRAMES: u64 = 20;

/// 作った文字の画像を覚えておく
pub struct Texts {
    ctx: egui::Context,
    font: Option<FontArc>,
    map: HashMap<Key, (Option<Text>, u64)>,
    frame: u64,
    /// 今の窓での NX の 1 ピクセルあたりの画面のピクセル数
    raster: f32,
    /// 別のスレッドで画像にする（フォントが無ければ無い）
    jobs: Option<Jobs<Job, Option<egui::ColorImage>>>,
}

impl Texts {
    pub fn new(ctx: &egui::Context) -> Self {
        let font = load_font();
        let jobs = font.clone().map(|f| {
            Jobs::new("text", THREADS, move |j: &Job| {
                render(&f, &j.key.text, &j.style, j.raster)
            })
        });
        Self {
            ctx: ctx.clone(),
            font,
            map: HashMap::new(),
            frame: 0,
            raster: 1.0,
            jobs,
        }
    }

    fn to_text(&self, img: egui::ColorImage, raster: f32) -> Text {
        let (w, h) = (img.size[0] as f32 / raster, img.size[1] as f32 / raster);
        Text {
            tex: Tex::new(&self.ctx, "text", img),
            w,
            h,
        }
    }

    /// フレームの初めに呼ぶ。`raster` は NX の 1 ピクセルが画面の何ピクセルか
    pub fn begin(&mut self, raster: f32) {
        self.frame += 1;
        // 細かさは 1/4 刻みにして、窓を少し動かしただけで作り直さないようにする
        let raster = ((raster * 4.0).round() / 4.0).clamp(0.5, 4.0);
        if raster != self.raster {
            self.raster = raster;
            self.map.clear();
        }
        let done = match &mut self.jobs {
            Some(jobs) => jobs.poll(self.frame.saturating_sub(STALE_FRAMES)),
            None => Vec::new(),
        };
        for (job, img) in done {
            // 頼んだあとで窓の大きさが変わったものは捨てる
            if job.raster == self.raster {
                let t = img.map(|img| self.to_text(img, job.raster));
                self.map.insert(job.key, (t, self.frame));
            }
        }
        if self.map.len() > 400 {
            let frame = self.frame;
            self.map.retain(|_, (_, used)| frame - *used < 120);
        }
    }

    /// `max_w` に収まるように行を分ける（`\n` でも分ける）。
    /// 日本語はどの字の間でも分けるが、句読点や閉じ括弧は行の頭に置かず、英数字の語は途中で分けない。
    /// 英語の文の折り返したところの空白は、行の頭にも終わりにも残さない
    pub fn wrap(&self, text: &str, px: f32, max_w: f32) -> Vec<String> {
        let Some(font) = &self.font else {
            return text.lines().map(str::to_owned).collect();
        };
        let sf = font.as_scaled(PxScale::from(px));
        wrap_with(text, max_w, |ch| sf.h_advance(sf.glyph_id(ch)))
    }

    pub fn get(&mut self, text: &str, style: &Style) -> Option<Text> {
        if text.is_empty() {
            return None;
        }
        let k = key(text, style, self.raster);
        let frame = self.frame;
        if let Some((t, used)) = self.map.get_mut(&k) {
            *used = frame;
            return t.clone();
        }
        let t = self.font.as_ref().and_then(|f| {
            let img = render(f, text, style, self.raster)?;
            Some(self.to_text(img, self.raster))
        });
        self.map.insert(k, (t.clone(), frame));
        t
    }

    /// 別のスレッドで画像にするよう頼む（できていれば何もしない）。後で頼んだものほど先にできる。
    /// 要るあいだは毎フレーム頼み直す（頼み直されなくなった仕事は捨てる）
    pub fn request(&mut self, text: &str, style: &Style) {
        if text.is_empty() {
            return;
        }
        let k = key(text, style, self.raster);
        if let Some((_, used)) = self.map.get_mut(&k) {
            *used = self.frame;
            return;
        }
        if let Some(jobs) = &mut self.jobs {
            let job = Job {
                key: k,
                style: *style,
                raster: self.raster,
            };
            jobs.request(&job, self.frame);
        }
    }

    /// できている文字の画像（[`Texts::request`] で頼んだもの。まだなら None）
    pub fn ready(&mut self, text: &str, style: &Style) -> Option<Text> {
        if text.is_empty() {
            return None;
        }
        let frame = self.frame;
        let (t, used) = self.map.get_mut(&key(text, style, self.raster))?;
        *used = frame;
        t.clone()
    }
}

/// 行の頭に置かない字
fn no_line_start(ch: char) -> bool {
    "、。，．・：；？！）」』】〕＞ー～…,.:;?!)]".contains(ch)
}

/// [`Texts::wrap`] の中身（`advance` は字の送り）
fn wrap_with(text: &str, max_w: f32, advance: impl Fn(char) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        let mut w = 0.0f32;
        // 今の英数字の語（空白の無い ASCII の字の並び。"it's" や "Alt+Enter" も 1 語）の始まり（行の中の位置、そこまでの幅）
        let mut word: Option<(usize, f32)> = None;
        // 折り返したあとの行か（その頭の空白は捨てる）
        let mut wrapped = false;
        let end = |l: String| l.trim_end_matches(' ').to_owned();
        for ch in para.chars() {
            if ch == ' ' && wrapped && line.is_empty() {
                continue;
            }
            let adv = advance(ch);
            let in_word = ch.is_ascii_graphic();
            if w + adv > max_w && !line.is_empty() && !no_line_start(ch) {
                wrapped = true;
                match word {
                    Some((i, wi)) if in_word && i > 0 => {
                        // 語ごと次の行へ
                        let rest = line.split_off(i);
                        lines.push(end(std::mem::replace(&mut line, rest)));
                        w -= wi;
                        word = Some((0, 0.0));
                    }
                    _ => {
                        lines.push(end(std::mem::take(&mut line)));
                        w = 0.0;
                        word = None;
                        if ch == ' ' {
                            continue;
                        }
                    }
                }
            }
            if !in_word {
                word = None;
            } else if word.is_none() {
                word = Some((line.len(), w));
            }
            line.push(ch);
            w += adv;
        }
        lines.push(line);
    }
    lines
}

/// 文字の形（0〜1 の濃さ）を並べる。`ascent` の位置に基準線
fn coverage(font: &FontArc, text: &str, px: f32) -> (Vec<f32>, usize, usize) {
    let sf = font.as_scaled(PxScale::from(px));
    let ascent = sf.ascent();
    let h = (ascent - sf.descent()).ceil().max(1.0) as usize;
    let mut glyphs = Vec::new();
    let mut x = 0.0f32;
    let mut prev = None;
    for ch in text.chars() {
        if ch.is_control() {
            continue;
        }
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        glyphs.push(id.with_scale_and_position(sf.scale(), point(x, ascent)));
        x += sf.h_advance(id);
        prev = Some(id);
    }
    let w = x.ceil().max(1.0) as usize + 1;
    let mut cov = vec![0.0f32; w * h];
    for g in glyphs {
        let Some(o) = font.outline_glyph(g) else {
            continue;
        };
        let b = o.px_bounds();
        o.draw(|gx, gy, c| {
            let px = b.min.x as i64 + i64::from(gx);
            let py = b.min.y as i64 + i64::from(gy);
            if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                let i = py as usize * w + px as usize;
                cov[i] = (cov[i] + c).min(1.0);
            }
        });
    }
    (cov, w, h)
}

/// 楕円（横の半径 `rx`、縦の半径 `ry`）の、縦に `dy` 離れた行での横の半径
fn half_width(rx: usize, ry: usize, dy: usize) -> usize {
    let (rx2, ry2) = ((rx * rx).max(1) as f32, (ry * ry).max(1) as f32);
    (0..=rx)
        .rev()
        .find(|&dx| (dx * dx) as f32 / rx2 + (dy * dy) as f32 / ry2 <= 1.0 + 1e-3)
        .unwrap_or(0)
}

/// 濃さを周りの楕円（横 `rx`、縦 `ry` ピクセル）に広げる（縁取りと太字に使う）。
/// 横の幅ごとの最大を先に作り、行ごとに楕円の幅のものを縦に重ねる（1 点ごとに楕円の中を全部見るより速い）
fn dilate(cov: &[f32], w: usize, h: usize, rx: usize, ry: usize) -> Vec<f32> {
    if rx == 0 && ry == 0 {
        return cov.to_vec();
    }
    // rows[r]：横に ±r の最大（r - 1 のものの左・中・右の最大）
    let mut rows: Vec<Vec<f32>> = Vec::with_capacity(rx + 1);
    rows.push(cov.to_vec());
    for r in 1..=rx {
        let prev = &rows[r - 1];
        let mut next = prev.clone();
        for y in 0..h {
            let line = &prev[y * w..(y + 1) * w];
            let out = &mut next[y * w..(y + 1) * w];
            for x in 0..w {
                let mut m = line[x];
                if x > 0 {
                    m = m.max(line[x - 1]);
                }
                if x + 1 < w {
                    m = m.max(line[x + 1]);
                }
                out[x] = m;
            }
        }
        rows.push(next);
    }
    let mut out = vec![0.0f32; w * h];
    for dy in -(ry as i64)..=(ry as i64) {
        let src = &rows[half_width(rx, ry, dy.unsigned_abs() as usize)];
        for y in 0..h {
            let sy = y as i64 + dy;
            if sy < 0 || sy as usize >= h {
                continue;
            }
            let line = &src[sy as usize * w..(sy as usize + 1) * w];
            for (o, &v) in out[y * w..(y + 1) * w].iter_mut().zip(line) {
                *o = o.max(v);
            }
        }
    }
    out
}

/// 下の色の上に `a` の濃さで色 `c` を重ねる（乗算済み）
fn over(dst: [f32; 4], c: Color32, a: f32) -> [f32; 4] {
    let a = a * f32::from(c.a()) / 255.0;
    let k = 1.0 - a;
    [
        f32::from(c.r()) / 255.0 * a + dst[0] * k,
        f32::from(c.g()) / 255.0 * a + dst[1] * k,
        f32::from(c.b()) / 255.0 * a + dst[2] * k,
        a + dst[3] * k,
    ]
}

/// 文字を画像にする。大きさは `raster` 倍（画面の細かさ）
fn render(font: &FontArc, text: &str, s: &Style, raster: f32) -> Option<egui::ColorImage> {
    let px = s.px * raster;
    let (mut cov, w0, h0) = coverage(font, text, px);
    // 縁取りと影の分だけ周りを空ける
    let edge = s.edge.map_or(0.0, |(_, e)| e * raster).ceil() as usize;
    let shadow = s.shadow.map_or(0.0, |(_, o)| o * raster).round() as usize;
    let bold = if s.bold {
        ((px / 20.0).round() as usize).max(1)
    } else {
        0
    };
    let pad = edge;
    let (w, h) = (w0 + pad * 2 + shadow + bold, h0 + pad * 2 + shadow);
    if w * h > 16_000_000 {
        return None;
    }
    // 余白を付けた大きさに置き直す
    let mut base = vec![0.0f32; w * h];
    for y in 0..h0 {
        for x in 0..w0 {
            base[(y + pad) * w + x + pad] = cov[y * w0 + x];
        }
    }
    if bold > 0 {
        // 右へ太らせる
        let src = base.clone();
        for y in 0..h {
            for x in 0..w {
                let mut m = src[y * w + x];
                for d in 1..=bold {
                    if x >= d {
                        m = m.max(src[y * w + x - d]);
                    }
                }
                base[y * w + x] = m;
            }
        }
    }
    cov = base;
    let edge_cov = s.edge.map(|_| dilate(&cov, w, h, edge, edge));
    let mut px_out = vec![[0.0f32; 4]; w * h];
    if let Some((c, _)) = s.shadow {
        let src = edge_cov.as_ref().unwrap_or(&cov);
        for y in shadow..h {
            for x in shadow..w {
                let a = src[(y - shadow) * w + x - shadow];
                if a > 0.0 {
                    px_out[y * w + x] = over(px_out[y * w + x], c, a);
                }
            }
        }
    }
    if let (Some((c, _)), Some(e)) = (s.edge, &edge_cov) {
        for (p, &a) in px_out.iter_mut().zip(e) {
            if a > 0.0 {
                *p = over(*p, c, a);
            }
        }
    }
    for (i, (p, &a)) in px_out.iter_mut().zip(&cov).enumerate() {
        if a > 0.0 {
            let color = match s.gradient {
                Some(bottom) => {
                    let t = (i / w) as f32 / (h.max(2) - 1) as f32;
                    s.color.lerp_to_gamma(bottom, t)
                }
                None => s.color,
            };
            *p = over(*p, color, a);
        }
    }
    let pixels = px_out
        .iter()
        .map(|p| {
            let f = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            Color32::from_rgba_premultiplied(f(p[0]), f(p[1]), f(p[2]), f(p[3]))
        })
        .collect();
    Some(egui::ColorImage::new([w, h], pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_is_premultiplied() {
        let p = over([0.0; 4], Color32::from_rgb(255, 0, 0), 0.5);
        assert_eq!(p, [0.5, 0.0, 0.0, 0.5]);
        let p = over(p, Color32::WHITE, 1.0);
        assert_eq!(p, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn wrap_keeps_punctuation_and_words() {
        let w = |t: &str, max: f32| wrap_with(t, max, |_| 1.0);
        assert_eq!(w("あいうえお", 2.0), ["あい", "うえ", "お"]);
        // 句読点は行の頭に置かない
        assert_eq!(w("あい。う", 2.0), ["あい。", "う"]);
        // 英数字の語は分けない
        assert_eq!(w("あ Enter", 5.0), ["あ", "Enter"]);
        // 英語の文は語の間で分け、分けたところの空白は残さない
        assert_eq!(w("ab cd ef", 5.0), ["ab cd", "ef"]);
        assert_eq!(w("ab cd", 2.0), ["ab", "cd"]);
        assert_eq!(w("it's ok", 5.0), ["it's", "ok"]);
        assert_eq!(w("Alt+Enter or F11", 10.0), ["Alt+Enter", "or F11"]);
        // 段落の頭の空白はそのまま
        assert_eq!(w(" ab", 9.0), [" ab"]);
        // 行より長い語は分ける
        assert_eq!(w("Enter", 3.0), ["Ent", "er"]);
        assert_eq!(w("a\nb", 9.0), ["a", "b"]);
    }

    /// 1 点ごとに楕円の中を全部見る（[`dilate`] の答え合わせ）
    fn dilate_naive(cov: &[f32], w: usize, h: usize, rx: usize, ry: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; w * h];
        let (rx2, ry2) = ((rx * rx).max(1) as f32, (ry * ry).max(1) as f32);
        for y in 0..h {
            for x in 0..w {
                let mut m = 0.0f32;
                for dy in -(ry as i64)..=(ry as i64) {
                    for dx in -(rx as i64)..=(rx as i64) {
                        if (dx * dx) as f32 / rx2 + (dy * dy) as f32 / ry2 > 1.0 + 1e-3 {
                            continue;
                        }
                        let (sx, sy) = (x as i64 + dx, y as i64 + dy);
                        if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                            m = m.max(cov[sy as usize * w + sx as usize]);
                        }
                    }
                }
                out[y * w + x] = m;
            }
        }
        out
    }

    #[test]
    fn dilate_matches_the_ellipse() {
        let (w, h) = (23, 17);
        // 決まった乱れ（xorshift）
        let mut seed = 0x2545_f491_u32;
        let cov: Vec<f32> = (0..w * h)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                if seed.is_multiple_of(7) {
                    (seed % 1000) as f32 / 1000.0
                } else {
                    0.0
                }
            })
            .collect();
        for (rx, ry) in [
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (6, 6),
            (3, 1),
            (0, 2),
            (2, 0),
        ] {
            assert_eq!(
                dilate(&cov, w, h, rx, ry),
                dilate_naive(&cov, w, h, rx, ry),
                "rx {rx} ry {ry}"
            );
        }
    }

    #[test]
    fn dilate_spreads_by_the_radius() {
        let mut cov = vec![0.0; 25];
        cov[12] = 1.0;
        let d = dilate(&cov, 5, 5, 1, 1);
        assert_eq!(d[7], 1.0);
        assert_eq!(d[11], 1.0);
        // 斜めは楕円の外
        assert_eq!(d[6], 0.0);
        assert_eq!(d[0], 0.0);
    }
}
