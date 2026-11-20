//! NX と同じ 1280×720 の画面に、スキンの画像を NX と同じ座標で描く。
//!
//! - 窓の大きさに合わせて拡大し、縦横の比は保つ（余りは黒）
//! - 画像の一部を切り出し、拡大・透明度・加算合成（光る表現）を付けて描ける（NX の `CTexture.tDraw2D` と同じ使い方）
//! - スキンの画像はユーザーの PC にある NX のフォルダから読む（NXT には入れない）
//!
//! 描画そのものは egui の Mesh で行う。egui は乗算済みアルファで合成するので、
//! 頂点の色のアルファを 0 にすると加算合成になる。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Color32, Mesh, Pos2, Rect, Vec2, pos2, vec2};

/// NX の画面の幅と高さ
pub const W: f32 = 1280.0;
pub const H: f32 = 720.0;

/// GPU に載せた画像
#[derive(Clone)]
pub struct Tex {
    handle: egui::TextureHandle,
    /// 画像のピクセル数
    pub w: f32,
    pub h: f32,
}

impl Tex {
    pub fn new(ctx: &egui::Context, name: &str, image: egui::ColorImage) -> Self {
        let [w, h] = image.size;
        let handle = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
        Self {
            handle,
            w: w as f32,
            h: h as f32,
        }
    }

    pub fn id(&self) -> egui::TextureId {
        self.handle.id()
    }
}

/// 画像のどこを切り出すか（NX の `Rectangle`）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Src {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub const fn src(x: f32, y: f32, w: f32, h: f32) -> Src {
    Src { x, y, w, h }
}

/// 描き方
#[derive(Debug, Clone, Copy)]
pub struct Opt {
    /// 切り出す範囲（None なら全体）
    pub src: Option<Src>,
    /// 拡大率（NX の `vcScaleRatio`。左上を基準に広げる）
    pub sx: f32,
    pub sy: f32,
    /// 不透明度（0〜1。NX の `nTransparency / 255`）
    pub alpha: f32,
    /// 加算合成（NX の `b加算合成`）
    pub add: bool,
}

impl Default for Opt {
    fn default() -> Self {
        Self {
            src: None,
            sx: 1.0,
            sy: 1.0,
            alpha: 1.0,
            add: false,
        }
    }
}

/// NX の座標で描く画面
pub struct Canvas {
    painter: egui::Painter,
    origin: Pos2,
    scale: f32,
}

impl Canvas {
    /// `area`（egui の座標）の中に 1280×720 を収める。外側は黒で塗る
    pub fn new(painter: &egui::Painter, area: Rect) -> Self {
        let scale = (area.width() / W).min(area.height() / H).max(0.01);
        let size = vec2(W * scale, H * scale);
        let origin = area.center() - size / 2.0;
        painter.rect_filled(area, 0.0, Color32::BLACK);
        let painter = painter.with_clip_rect(Rect::from_min_size(origin, size));
        Self {
            painter,
            origin,
            scale,
        }
    }

    /// NX の 1 ピクセルが egui の何ポイントか
    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn pos(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.scale
    }

    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.pos(x, y), vec2(w, h) * self.scale)
    }

    /// egui の座標を NX の座標に
    pub fn to_nx(&self, p: Pos2) -> Pos2 {
        let v: Vec2 = (p - self.origin) / self.scale;
        pos2(v.x, v.y)
    }

    pub fn painter(&self) -> &egui::Painter {
        &self.painter
    }

    /// 同じ座標のまま、描ける範囲だけ狭めた画面
    pub fn with_clip(&self, clip: Rect) -> Canvas {
        Canvas {
            painter: self.painter.with_clip_rect(clip),
            origin: self.origin,
            scale: self.scale,
        }
    }

    /// 画像全体を (x, y) に等倍で描く
    pub fn draw(&self, t: &Tex, x: f32, y: f32) {
        self.draw_opt(t, x, y, Opt::default());
    }

    /// 画像の一部を (x, y) に描く
    pub fn part(&self, t: &Tex, x: f32, y: f32, s: Src) {
        self.draw_opt(
            t,
            x,
            y,
            Opt {
                src: Some(s),
                ..Default::default()
            },
        );
    }

    pub fn draw_opt(&self, t: &Tex, x: f32, y: f32, o: Opt) {
        let full = src(0.0, 0.0, t.w, t.h);
        let mut s = o.src.unwrap_or(full);
        let (mut x, mut y) = (x, y);
        // 画像の外にはみ出す切り出しは、はみ出した分を描かない（D3D の描き方に合わせる）
        if s.x < 0.0 {
            x -= s.x * o.sx;
            s.w += s.x;
            s.x = 0.0;
        }
        if s.y < 0.0 {
            y -= s.y * o.sy;
            s.h += s.y;
            s.y = 0.0;
        }
        s.w = s.w.min(t.w - s.x);
        s.h = s.h.min(t.h - s.y);
        if s.w <= 0.0 || s.h <= 0.0 || o.alpha <= 0.0 {
            return;
        }
        let dest = self.rect(x, y, s.w * o.sx, s.h * o.sy);
        let uv = Rect::from_min_max(
            pos2(s.x / t.w, s.y / t.h),
            pos2((s.x + s.w) / t.w, (s.y + s.h) / t.h),
        );
        let a = (o.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        let color = if o.add {
            Color32::from_rgba_premultiplied(a, a, a, 0)
        } else {
            Color32::from_rgba_premultiplied(a, a, a, a)
        };
        let mut mesh = Mesh::with_texture(t.id());
        mesh.add_rect_with_uv(dest, uv, color);
        self.painter.add(mesh);
    }

    /// 四角を塗る（スキンが無いときの代わりなど）
    pub fn fill(&self, x: f32, y: f32, w: f32, h: f32, color: Color32) {
        self.painter.rect_filled(self.rect(x, y, w, h), 0.0, color);
    }

    /// 枠を描く
    pub fn frame(&self, x: f32, y: f32, w: f32, h: f32, color: Color32) {
        self.painter.rect_stroke(
            self.rect(x, y, w, h),
            0.0,
            egui::Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
    }

    /// 画面全体を色で覆う（フェード）
    pub fn cover(&self, color: Color32) {
        self.fill(0.0, 0.0, W, H, color);
    }
}

/// 画像のファイルを読む。`black_key` なら真っ黒のピクセルを透明にする（NX の `b黒を透過する`）
fn load_image(path: &Path, black_key: bool, max_side: Option<u32>) -> Option<egui::ColorImage> {
    let img = match image::open(path) {
        Ok(i) => i,
        Err(e) => {
            log::warn!("{}: {e}", path.display());
            return None;
        }
    };
    let img = match max_side {
        Some(m) if img.width().max(img.height()) > m => {
            img.resize(m, m, image::imageops::FilterType::Triangle)
        }
        _ => img,
    };
    let mut rgba = img.to_rgba8();
    if black_key {
        for p in rgba.pixels_mut() {
            // 上流（D3DX のカラーキー）と同じく、不透明な黒だけを抜く
            if p.0 == [0, 0, 0, 255] {
                p[3] = 0;
            }
        }
    }
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

/// NX のスキン（画像と音）。無ければ何も描かない
pub struct Skin {
    ctx: egui::Context,
    dir: Option<PathBuf>,
    cache: RefCell<HashMap<(String, bool), Option<Tex>>>,
}

impl Skin {
    pub fn new(ctx: &egui::Context, dir: Option<PathBuf>) -> Self {
        Self {
            ctx: ctx.clone(),
            dir,
            cache: RefCell::default(),
        }
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn is_loaded(&self) -> bool {
        self.dir.is_some()
    }

    /// スキンの中のファイル（`Graphics\5_bar box.png` など NX の書き方でよい）
    pub fn path(&self, rel: &str) -> Option<PathBuf> {
        nx::find_file(self.dir.as_deref()?, rel)
    }

    /// 画像（一度読んだら覚えておく）
    pub fn tex(&self, rel: &str) -> Option<Tex> {
        self.tex_with(rel, false)
    }

    /// 真っ黒を透明にして読む画像
    pub fn tex_keyed(&self, rel: &str) -> Option<Tex> {
        self.tex_with(rel, true)
    }

    fn tex_with(&self, rel: &str, black_key: bool) -> Option<Tex> {
        let key = (rel.to_owned(), black_key);
        if let Some(t) = self.cache.borrow().get(&key) {
            return t.clone();
        }
        let tex = self
            .path(rel)
            .and_then(|p| load_image(&p, black_key, None))
            .map(|img| Tex::new(&self.ctx, rel, img));
        if tex.is_none() && self.dir.is_some() {
            log::warn!("skin: no {rel}");
        }
        self.cache.borrow_mut().insert(key, tex.clone());
        tex
    }
}

enum Slot {
    Loading,
    Ready(Tex, u64),
    Failed,
}

/// 曲の画像（ジャケット）を別のスレッドで読む
pub struct Images {
    ctx: egui::Context,
    tx: Sender<PathBuf>,
    rx: Receiver<(PathBuf, Option<egui::ColorImage>)>,
    slots: HashMap<PathBuf, Slot>,
    frame: u64,
}

/// これより多く覚えたら、しばらく使っていないものを捨てる
const MAX_IMAGES: usize = 200;
/// 読むときに縮める大きさ（NX のジャケットの表示は最大 368 ピクセル）
const MAX_SIDE: u32 = 400;

impl Images {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, job_rx) = channel::<PathBuf>();
        let (done_tx, rx) = channel();
        std::thread::spawn(move || {
            for path in job_rx {
                let img = load_image(&path, false, Some(MAX_SIDE));
                if done_tx.send((path, img)).is_err() {
                    break;
                }
            }
        });
        Self {
            ctx: ctx.clone(),
            tx,
            rx,
            slots: HashMap::new(),
            frame: 0,
        }
    }

    /// 読み終わったものを GPU に載せる。毎フレーム 1 回呼ぶ
    pub fn poll(&mut self) {
        self.frame += 1;
        for (path, img) in self.rx.try_iter() {
            let slot = match img {
                Some(img) => Slot::Ready(
                    Tex::new(&self.ctx, &path.to_string_lossy(), img),
                    self.frame,
                ),
                None => Slot::Failed,
            };
            self.slots.insert(path, slot);
        }
        if self.slots.len() > MAX_IMAGES {
            let frame = self.frame;
            self.slots
                .retain(|_, s| !matches!(s, Slot::Ready(_, used) if frame - *used > 60));
        }
    }

    /// 読めていれば画像。まだなら読み始める
    pub fn get(&mut self, path: &Path) -> Option<Tex> {
        match self.slots.get_mut(path) {
            Some(Slot::Ready(t, used)) => {
                *used = self.frame;
                Some(t.clone())
            }
            Some(_) => None,
            None => {
                self.slots.insert(path.to_owned(), Slot::Loading);
                let _ = self.tx.send(path.to_owned());
                None
            }
        }
    }

    /// 読めなかったか
    pub fn failed(&self, path: &Path) -> bool {
        matches!(self.slots.get(path), Some(Slot::Failed))
    }
}

/// 描くための道具一式（スキンの画像、文字、曲の画像）
pub struct Gfx {
    pub skin: Skin,
    pub texts: crate::text::Texts,
    pub images: Images,
}

impl Gfx {
    pub fn new(ctx: &egui::Context, skin_dir: Option<PathBuf>) -> Self {
        Self {
            skin: Skin::new(ctx, skin_dir),
            texts: crate::text::Texts::new(ctx),
            images: Images::new(ctx),
        }
    }

    /// フレームの初めに呼ぶ
    pub fn begin(&mut self, c: &Canvas, pixels_per_point: f32) {
        self.images.poll();
        self.texts.begin(c.scale() * pixels_per_point);
    }
}

/// 数字などを並べた画像から 1 文字ずつ切り出して描く（NX の `tDrawBPM` などと同じ）
pub struct Glyphs<'a> {
    /// 文字、切り出す範囲、次の文字までの幅
    pub table: &'a [(char, Src, f32)],
    /// 表に無い文字の幅
    pub missing: f32,
}

impl Glyphs<'_> {
    pub fn draw(&self, c: &Canvas, t: Option<&Tex>, x: f32, y: f32, text: &str) {
        let mut x = x;
        for ch in text.chars() {
            match self.table.iter().find(|(k, _, _)| *k == ch) {
                Some((_, s, adv)) => {
                    if let Some(t) = t {
                        c.part(t, x, y, *s);
                    }
                    x += adv;
                }
                None => x += self.missing,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_keeps_the_aspect_ratio() {
        let ctx = egui::Context::default();
        let painter = egui::Painter::new(
            ctx,
            egui::LayerId::background(),
            Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1200.0)),
        );
        let c = Canvas::new(
            &painter,
            Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1200.0)),
        );
        assert_eq!(c.scale(), 1.5);
        // 縦に余った 120 ポイントは上下に半分ずつ
        assert_eq!(c.pos(0.0, 0.0), pos2(0.0, 60.0));
        assert_eq!(c.pos(1280.0, 720.0), pos2(1920.0, 1140.0));
        assert_eq!(c.to_nx(pos2(960.0, 600.0)), pos2(640.0, 360.0));
    }
}
