//! 選曲画面。上流 DTXManiaNX の `CStageSongSelection` とその部品（`CActSelectSongList`、
//! `CActSelectStatusPanel`、`CActSelectPreimagePanel`、`CActSelectArtistComment`、
//! `CActSelectInformation`、`CActSelectShowCurrentPosition`）と同じ画像を同じ座標と順で描く。
//! 座標と動きの速さは上流のコードから写している。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use egui::{Color32, Pos2};
use library::{LEVELS, Node, Song, SongChart};

use crate::gfx::{Canvas, Gfx, Glyphs, H, Opt, Src, Tex, src};
use crate::songs::{Entry, SongList};
use crate::text::Style;

/// 13 本の曲バーの基本の y 座標（上流 `ptバーの基本座標`）。6 本目（5）が選んでいる曲
const BAR_Y: [f32; 13] = [
    5.0, 56.0, 107.0, 158.0, 209.0, 270.0, 362.0, 413.0, 464.0, 515.0, 566.0, 617.0, 668.0,
];
/// 曲バーの x（選んでいる曲のバーは少し左）
const BAR_X: f32 = 673.0;
const SELECTED_BAR_X: f32 = 665.0;
/// 選んでいる曲のバーの文字の y（バーはこの 30 上）
const SELECTED_Y: f32 = 269.0;
/// 曲名の最大の幅（長い曲名は横に縮める）
const TITLE_MAX_W: f32 = 510.0;
/// フェードの長さ（上流 `CActFIFOBlack`：0〜100 を 5 ミリ秒ずつ）
const FADE: Duration = Duration::from_millis(500);
/// 選んでから大きなジャケットを出すまで（上流の既定 100 ミリ秒、そこから 100 ミリ秒で濃くなる）
const PREIMAGE_WAIT_MS: f64 = 100.0;
/// 選んでからプレビューの音を鳴らすまで（上流の既定 1000 ミリ秒）
pub const PREVIEW_WAIT: Duration = Duration::from_millis(1000);

/// 曲バーの種類（上流 `EBarType`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BarType {
    Score,
    Box,
    Other,
}

impl BarType {
    fn bar(self) -> &'static str {
        match self {
            Self::Score => "Graphics/5_bar score.png",
            Self::Box => "Graphics/5_bar box.png",
            Self::Other => "Graphics/5_bar other.png",
        }
    }

    fn selected_bar(self) -> &'static str {
        match self {
            Self::Score => "Graphics/5_bar score selected.png",
            Self::Box => "Graphics/5_bar box selected.png",
            Self::Other => "Graphics/5_bar other selected.png",
        }
    }
}

/// 1 行に出すもの
struct Row {
    title: String,
    kind: BarType,
    /// ジャケット（無ければスキンの既定の画像）
    preimage: Preimage,
}

#[derive(Clone)]
enum Preimage {
    File(PathBuf),
    Skin(&'static str),
}

/// 選んでいる譜面を読んで分かること（選曲のパネルに出す）
#[derive(Debug, Clone, Default)]
struct Details {
    duration_ms: f64,
    /// LC, HH, LP, SD, HT, BD, LT, FT, CY の数（上流の並び。HH は開きを、LP は左バスを、CY はライドを含む）
    lanes: [usize; 9],
    total: usize,
}

fn read_details(path: &Path) -> Option<Details> {
    use dtx::DrumLane as L;
    let bytes = std::fs::read(path).ok()?;
    let chart = dtx::load(&bytes);
    let tempo = dtx::TempoMap::build(&chart);
    let notes = tempo.drum_notes(&chart);
    let mut lanes = [0usize; 9];
    for n in &notes {
        let i = match n.lane {
            L::LC => 0,
            L::HH | L::HHO => 1,
            L::LP | L::LBD => 2,
            L::SD => 3,
            L::HT => 4,
            L::BD => 5,
            L::LT => 6,
            L::FT => 7,
            L::CY | L::RD => 8,
        };
        lanes[i] += 1;
    }
    let duration_ms = chart
        .chips
        .iter()
        .map(|c| tempo.chip_ms(c))
        .fold(0.0, f64::max);
    Some(Details {
        duration_ms,
        lanes,
        total: notes.len(),
    })
}

/// フェード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fade {
    In,
    Out,
}

/// 選曲画面の動き（スクロールや登場のアニメ）
pub struct SelectView {
    /// 画面に入った時刻（登場のアニメ）
    entered: Instant,
    /// 目標までのスクロールの残り（1 行 = 100。上流 `nTargetScrollCounter - nCurrentScrollCounter`）
    pending: i64,
    scroll_tick: Instant,
    /// 選んでいるものが変わった時刻（ジャケットの遅れた表示やコメントの流れ）
    changed: Instant,
    selected_key: Option<(usize, usize, usize)>,
    fade: Option<(Fade, Instant)>,
    /// 曲のフォルダの中のファイル名（大文字・小文字を区別しない OS のため）
    found: HashMap<(PathBuf, String), Option<PathBuf>>,
    details: HashMap<PathBuf, Option<Details>>,
    details_tx: Sender<PathBuf>,
    details_rx: Receiver<(PathBuf, Option<Details>)>,
}

impl Default for SelectView {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectView {
    pub fn new() -> Self {
        let (details_tx, job_rx) = channel::<PathBuf>();
        let (done_tx, details_rx) = channel();
        std::thread::spawn(move || {
            for path in job_rx {
                let d = read_details(&path);
                if done_tx.send((path, d)).is_err() {
                    break;
                }
            }
        });
        let now = Instant::now();
        Self {
            entered: now,
            pending: 0,
            scroll_tick: now,
            changed: now,
            selected_key: None,
            fade: Some((Fade::In, now)),
            found: HashMap::new(),
            details: HashMap::new(),
            details_tx,
            details_rx,
        }
    }

    /// 画面に入り直す（登場のアニメとフェードから）
    pub fn enter(&mut self) {
        let now = Instant::now();
        self.entered = now;
        self.pending = 0;
        self.scroll_tick = now;
        self.fade = Some((Fade::In, now));
        self.selected_key = None;
    }

    /// カーソルを `delta` 行動かした（`SongList::move_by` の後に呼ぶ）
    pub fn moved(&mut self, delta: isize) {
        self.pending += delta as i64 * 100;
    }

    /// BOX の出入りなどで一覧が変わった（スクロールせずに切り替える）
    pub fn jumped(&mut self) {
        self.pending = 0;
    }

    /// 決めたので暗くする
    pub fn fade_out(&mut self) {
        if !matches!(self.fade, Some((Fade::Out, _))) {
            self.fade = Some((Fade::Out, Instant::now()));
        }
    }

    pub fn fading_out(&self) -> bool {
        matches!(self.fade, Some((Fade::Out, _)))
    }

    /// 暗くし終えた
    pub fn faded_out(&self) -> bool {
        matches!(self.fade, Some((Fade::Out, t)) if t.elapsed() >= FADE)
    }

    /// スクロールが止まっていて、選んでから `wait` 経った
    pub fn settled_for(&self, songs: &SongList, wait: Duration) -> bool {
        self.pending == 0
            && self.selected_key == Some(selection_key(songs))
            && self.changed.elapsed() >= wait
    }

    /// 時間を進める（上流と同じく 2 ミリ秒ごとに、遠いほど速く近づく）
    pub fn update(&mut self, songs: &SongList) {
        let now = Instant::now();
        while now.duration_since(self.scroll_tick) >= Duration::from_millis(2) {
            self.scroll_tick += Duration::from_millis(2);
            let d = self.pending.abs();
            if d == 0 {
                continue;
            }
            let step = match d {
                0..=100 => 2,
                101..=300 => 3,
                301..=500 => 4,
                _ => 8,
            };
            self.pending -= self.pending.signum() * step.min(d);
        }
        if self.pending == 0 {
            self.scroll_tick = now;
        }
        for (path, d) in self.details_rx.try_iter() {
            self.details.insert(path, d);
        }
        // 選んでいるものが変わったか（スクロール中は変わったことにしない）
        let key = selection_key(songs);
        if self.pending == 0 && self.selected_key != Some(key) {
            self.selected_key = Some(key);
            self.changed = now;
        }
        if matches!(self.fade, Some((Fade::In, t)) if t.elapsed() >= FADE) {
            self.fade = None;
        }
    }

    /// 選んでいる曲のプレビューの音（`#PREVIEW`）
    pub fn preview_path(&mut self, songs: &SongList) -> Option<PathBuf> {
        let (dir, name) = match songs.selected_entry()? {
            Entry::Node(Node::Song(s)) => {
                let c = chart_for(songs, s)?;
                (c.path.parent()?.to_owned(), c.head.preview.trim())
            }
            // box.def の `#PREVIEW`
            Entry::Node(Node::Box(b)) => (b.dir.clone(), b.preview.trim()),
            _ => return None,
        };
        if name.is_empty() {
            return None;
        }
        self.find(&dir, name)
    }

    /// 曲のフォルダの中のファイル（大文字・小文字を区別しない）
    fn find(&mut self, dir: &Path, rel: &str) -> Option<PathBuf> {
        let key = (dir.to_owned(), rel.to_owned());
        if let Some(p) = self.found.get(&key) {
            return p.clone();
        }
        let p = nx::find_file(dir, rel).filter(|p| p.is_file());
        if self.found.len() > 2000 {
            self.found.clear();
        }
        self.found.insert(key, p.clone());
        p
    }

    fn details(&mut self, path: &Path) -> Option<Details> {
        match self.details.get(path) {
            Some(d) => d.clone(),
            None => {
                self.details.insert(path.to_owned(), None);
                let _ = self.details_tx.send(path.to_owned());
                None
            }
        }
    }

    fn row(&mut self, songs: &SongList, e: Entry<'_>) -> Row {
        match e {
            Entry::Back => Row {
                title: "<< BACK".into(),
                kind: BarType::Box,
                preimage: Preimage::Skin("Graphics/5_preimage backbox.png"),
            },
            Entry::Random => Row {
                title: "< RANDOM SELECT >".into(),
                kind: BarType::Other,
                preimage: Preimage::Skin("Graphics/5_preimage random.png"),
            },
            Entry::Node(Node::Box(b)) => {
                let name = b.preimage.trim();
                let file = (!name.is_empty())
                    .then(|| self.find(&b.dir, name))
                    .flatten();
                Row {
                    title: b.title.clone(),
                    kind: BarType::Box,
                    preimage: file.map_or(
                        Preimage::Skin("Graphics/5_preimage default.png"),
                        Preimage::File,
                    ),
                }
            }
            Entry::Node(Node::Song(s)) => {
                let chart = chart_for(songs, s);
                let file = chart.and_then(|c| {
                    let name = c.head.preimage.trim();
                    if name.is_empty() {
                        return None;
                    }
                    self.find(c.path.parent()?, name)
                });
                Row {
                    title: s.title.clone(),
                    kind: BarType::Score,
                    preimage: file.map_or(
                        Preimage::Skin("Graphics/5_preimage default.png"),
                        Preimage::File,
                    ),
                }
            }
        }
    }

    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx, songs: &SongList) {
        self.update(songs);
        let ms = self.entered.elapsed().as_secs_f64() * 1000.0;
        let skin = g.skin.is_loaded();
        // 背景
        if let Some(t) = g.skin.tex("Graphics/5_background.jpg") {
            c.draw(&t, 0.0, 0.0);
        } else {
            c.fill(0.0, 0.0, 1280.0, 720.0, Color32::from_rgb(20, 24, 40));
        }
        if let Some(t) = g.skin.tex("Graphics/5_BPM.png") {
            c.draw(&t, 32.0, 258.0);
        }
        self.draw_preimage(c, g, songs);
        self.draw_artist_comment(c, g, songs);
        self.draw_list(c, g, songs, ms);
        self.draw_status(c, g, songs);
        // 演奏の記録の欄（上流 `CActSelectPerfHistoryPanel`：0〜100 を 5 ミリ秒ずつ、下から上がる）。中身はまだ
        if let Some(t) = g.skin.tex_keyed("Graphics/5_play history panel.png") {
            let k = (ms / 5.0).min(100.0) / 100.0;
            let q = ((1.5 + 0.5 * k) * std::f64::consts::PI).cos();
            c.draw(&t, 700.0, 570.0 + (t.h as f64 * (1.0 - q * q)) as f32);
        }
        // 上の帯（上流 ct登場時アニメ用共通：0〜100 を 3 ミリ秒ずつ、sin で下りてくる）
        let common = (ms / 3.0).min(100.0) / 100.0;
        if let Some(t) = g.skin.tex("Graphics/5_header panel.png") {
            let y = (t.h as f64 * (std::f64::consts::FRAC_PI_2 * common).sin()) as f32 - t.h;
            c.draw(&t, 0.0, y);
        }
        draw_information(c, g, ms);
        if let Some(t) = g.skin.tex("Graphics/5_footer panel.png") {
            c.draw(&t, 0.0, 720.0 - t.h);
        }
        self.draw_scrollbar(c, g, songs, common);
        if !skin {
            draw_no_skin(c, g);
        }
        if let Some((kind, t)) = self.fade {
            let p = (t.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0);
            let a = match kind {
                Fade::In => 1.0 - p,
                Fade::Out => p,
            };
            c.cover(Color32::from_black_alpha((a * 255.0) as u8));
        }
    }

    /// 曲バーの一覧（上流 `CActSelectSongList.OnUpdateAndDraw`）
    fn draw_list(&mut self, c: &Canvas, g: &mut Gfx, songs: &SongList, ms: f64) {
        let n = songs.len();
        if n == 0 {
            let msg = if songs.scanning().is_some() {
                "曲を探しています…"
            } else {
                "曲が見つかりません"
            };
            if let Some(t) = g.texts.get(msg, &Style::plain(24.0, Color32::WHITE)) {
                t.draw(c, 800.0, 280.0, 1.0, 1.0);
            }
            self.draw_list_panels(c, g, 1.0);
            return;
        }
        // 表示の位置（1 行 = 100）。スクロール中は目標の手前
        let pos = songs.cursor as i64 * 100 - self.pending;
        let sel = pos.div_euclid(100);
        let frac = pos.rem_euclid(100) as f32 / 100.0;
        let item = |i: usize| (sel - 5 + i as i64).rem_euclid(n as i64) as usize;
        // 登場のアニメ（上流 ct登場アニメ用[i]：-i*10 から 100 まで、3 ミリ秒ずつ）
        let appear = |i: usize| (ms / 3.0 - i as f64 * 10.0).min(100.0);
        let appearing = appear(12) < 100.0;
        for (i, &base_y) in BAR_Y.iter().enumerate() {
            let a = appear(i);
            if appearing && a < 0.0 {
                continue;
            }
            if !appearing && i == 0 && frac > 0.0 {
                // 一番上の行は、上へ動いているあいだは出さない
                continue;
            }
            let Some(entry) = songs.entry(item(i)) else {
                continue;
            };
            let row = self.row(songs, entry);
            if i == 5 && (appearing || frac == 0.0) {
                self.draw_selected_bar(c, g, &row);
                continue;
            }
            let (x, y) = if appearing {
                (BAR_X + 500.0 - (a / 100.0 * 500.0) as f32, base_y)
            } else {
                let next = if i == 0 { 0 } else { i - 1 };
                (BAR_X, base_y + (BAR_Y[next] - base_y) * frac)
            };
            self.draw_bar(c, g, &row, x, y);
        }
        let panels = if appearing {
            (appear(0) / 100.0).max(0.0) as f32
        } else {
            1.0
        };
        self.draw_list_panels(c, g, panels);
        // 何番目か（上流 tアイテム数の描画。右詰め）
        if let Some(t) = g.skin.tex("Graphics/5_skill number on gauge etc.png") {
            let s = format!("{}/{}", songs.cursor + 1, n);
            let mut x = 1260.0;
            for ch in s.chars().rev() {
                let (dx, dy) = match ch {
                    '/' => (96.0, 0.0),
                    d => {
                        let v = d.to_digit(10).unwrap_or(0) as f32;
                        ((v % 6.0) * 16.0, (v / 6.0).floor() * 16.0)
                    }
                };
                c.part(&t, x, 620.0, src(dx, dy, 16.0, 16.0));
                x -= 16.0;
            }
        }
    }

    fn draw_list_panels(&self, c: &Canvas, g: &Gfx, p: f32) {
        if let Some(t) = g.skin.tex("Graphics/5_header song list.png") {
            c.draw(&t, 0.0, t.h * p - t.h);
        }
        if let Some(t) = g.skin.tex("Graphics/5_footer song list.png") {
            c.draw(&t, 0.0, 720.0 - t.h * p);
        }
    }

    fn thumbnail(&mut self, g: &mut Gfx, p: &Preimage) -> Option<Tex> {
        match p {
            Preimage::File(path) => match g.images.get(path) {
                Some(t) => Some(t),
                None if g.images.failed(path) => g.skin.tex("Graphics/5_preimage default.png"),
                None => None,
            },
            Preimage::Skin(rel) => g.skin.tex(rel),
        }
    }

    /// ジャケットを `size` の正方形に収めて描く（縦横の比は保ち、真ん中に寄せる）
    fn draw_fit(c: &Canvas, t: &Tex, x: f32, y: f32, size: f32, scale: f32, alpha: f32) {
        let f = (size / t.w).min(size / t.h) * scale;
        let (w, h) = (t.w * f, t.h * f);
        c.draw_opt(
            t,
            x + ((size - w) / 2.0).floor(),
            y + ((size - h) / 2.0).floor(),
            Opt {
                sx: f,
                sy: f,
                alpha,
                ..Default::default()
            },
        );
    }

    fn title_style() -> Style {
        // 上流：曲名は太字の 20 ピクセル（2 倍で描いて半分に）、影は (10,10,10) の 1/4 の濃さを 1 ピクセル右下
        Style {
            px: 20.0,
            color: Color32::WHITE,
            bold: true,
            shadow: Some((Color32::from_rgba_unmultiplied(10, 10, 10, 0x40), 1.0)),
            edge: None,
        }
    }

    fn draw_bar(&mut self, c: &Canvas, g: &mut Gfx, row: &Row, x: f32, y: f32) {
        match g.skin.tex(row.kind.bar()) {
            Some(t) => c.draw(&t, x, y),
            None => c.fill(
                x,
                y,
                620.0,
                48.0,
                Color32::from_rgba_unmultiplied(60, 50, 40, 200),
            ),
        }
        if let Some(t) = g.texts.get(&row.title, &Self::title_style()) {
            // 文字は高さ 37 の帯の真ん中
            t.draw_fit(c, x + 78.0, y + 5.0 + (37.0 - t.h) / 2.0, TITLE_MAX_W);
        }
        if let Some(t) = self.thumbnail(g, &row.preimage) {
            Self::draw_fit(c, &t, x + 31.0, y + 2.0, 44.0, 1.0, 1.0);
        }
    }

    fn draw_selected_bar(&mut self, c: &Canvas, g: &mut Gfx, row: &Row) {
        let (x, y) = (SELECTED_BAR_X, SELECTED_Y);
        match g.skin.tex(row.kind.selected_bar()) {
            Some(t) => c.draw(&t, x, y - 30.0),
            None => c.fill(
                x,
                y - 30.0,
                640.0,
                96.0,
                Color32::from_rgba_unmultiplied(150, 60, 60, 220),
            ),
        }
        if let Some(t) = self.thumbnail(g, &row.preimage) {
            Self::draw_fit(c, &t, x + 7.0, y - 3.0, 44.0, 1.0, 1.0);
        }
        if let Some(t) = g.texts.get(&row.title, &Self::title_style()) {
            t.draw_fit(c, x + 55.0, y + (37.0 - t.h) / 2.0, TITLE_MAX_W);
        }
    }

    /// 画面の点（NX の座標）にある曲バーが、選んでいる曲から何本先か（上は負）
    pub fn row_at(p: Pos2) -> Option<isize> {
        if p.x < SELECTED_BAR_X || !(0.0..H).contains(&p.y) {
            return None;
        }
        let top = |i: usize| if i == 5 { SELECTED_Y - 30.0 } else { BAR_Y[i] };
        (0..13)
            .rev()
            .find(|&i| p.y >= top(i))
            .map(|i| i as isize - 5)
    }

    /// 大きなジャケット（上流 `CActSelectPreimagePanel`）
    fn draw_preimage(&mut self, c: &Canvas, g: &mut Gfx, songs: &SongList) {
        let (bx, by) = (250.0, 34.0);
        match g.skin.tex("Graphics/5_preimage panel.png") {
            Some(t) => c.draw(&t, bx, by),
            None => c.frame(bx, by, 313.0, 313.0, Color32::GRAY),
        }
        if self.pending != 0 {
            return;
        }
        // 選んでから 100 ミリ秒待ち、100 ミリ秒で 90% から 100% の大きさへ濃くなる
        let t_ms = self.changed.elapsed().as_secs_f64() * 1000.0 - PREIMAGE_WAIT_MS;
        if t_ms <= 0.0 {
            return;
        }
        let p = (t_ms / 100.0).min(1.0) as f32;
        let Some(entry) = songs.selected_entry() else {
            return;
        };
        let row = self.row(songs, entry);
        if let Some(t) = self.thumbnail(g, &row.preimage) {
            Self::draw_fit(c, &t, bx + 8.0, by + 8.0, 292.0, 0.9 + 0.1 * p, p);
        }
    }

    /// アーティスト名とコメント（上流 `CActSelectArtistComment`）
    fn draw_artist_comment(&mut self, c: &Canvas, g: &mut Gfx, songs: &SongList) {
        if let Some(t) = g.skin.tex_keyed("Graphics/5_comment bar.png") {
            c.draw(&t, 560.0, 257.0);
        }
        let (artist, comment) = match songs.selected_entry() {
            Some(Entry::Node(Node::Song(s))) => {
                let chart = chart_for(songs, s);
                let artist = chart
                    .map(|c| c.head.artist.clone())
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| s.artist.clone());
                (
                    artist,
                    chart.map(|c| c.head.comment.clone()).unwrap_or_default(),
                )
            }
            // 上流と同じく、box.def に無ければ決まった文
            Some(Entry::Node(Node::Box(b))) => (
                b.artist.clone(),
                if b.comment.is_empty() {
                    "BOX に移動します。".to_owned()
                } else {
                    b.comment.clone()
                },
            ),
            Some(Entry::Back) => (String::new(), "BOX を出ます。".to_owned()),
            Some(Entry::Random) => (
                String::new(),
                format!(
                    "難易度レベル {} 付近の曲をランダムに選択します。難易度レベルを持たない曲も選択候補となります。",
                    songs.level + 1
                ),
            ),
            _ => (String::new(), String::new()),
        };
        let white = Style::plain(20.0, Color32::WHITE);
        if let Some(t) = g.texts.get(&artist, &white) {
            // 右寄せ（右端 1235）。510 より長ければ縮める
            let w = t.w.min(510.0);
            t.draw_fit(c, 1260.0 - 25.0 - w, 320.0, 510.0);
        }
        if let Some(t) = g.texts.get(&comment, &white) {
            let (x, y) = (683.0, 339.0);
            let room = 1250.0 - x;
            if t.w < room {
                t.draw(c, x, y, 1.0, 1.0);
            } else {
                // 長いコメントは流す（上流：-740 から 10 ミリ秒に 1 ピクセル）
                let span = 740.0 + t.w;
                let off = ((self.changed.elapsed().as_secs_f32() * 100.0) % span) - 740.0;
                let clip = c
                    .painter()
                    .clip_rect()
                    .intersect(c.rect(x, y - 2.0, 1250.0 - x, 30.0));
                let sub = c.with_clip(clip);
                t.draw(&sub, x - off, y, 1.0, 1.0);
            }
        }
    }

    /// 難易度や BPM のパネル（上流 `CActSelectStatusPanel`）
    fn draw_status(&mut self, c: &Canvas, g: &mut Gfx, songs: &SongList) {
        let entry = songs.selected_entry();
        let song = match entry {
            Some(Entry::Node(Node::Song(s))) => Some(s.as_ref()),
            _ => None,
        };
        let chart = song.and_then(|s| chart_for(songs, s));
        let details = chart.and_then(|ch| self.details(&ch.path));
        // BPM と長さ
        let bpm_font = g.skin.tex("Graphics/5_bpm font.png");
        if let Some(ch) = chart {
            let (bx, by) = (90.0, 275.0);
            let bpm = ch
                .head
                .bpm
                .map(|b| format!("{:>3}", b.round() as i64))
                .unwrap_or_default();
            BPM_GLYPHS.draw(c, bpm_font.as_ref(), bx + 45.0, by + 23.0, &bpm);
            if let Some(d) = &details {
                let secs = (d.duration_ms / 1000.0) as u64;
                let len = format!("{}:{:02}", secs / 60, secs % 60);
                BPM_GLYPHS.draw(c, bpm_font.as_ref(), bx + 42.0, by - 7.0, &len);
            }
        }
        if let Some(t) = g.skin.tex("Graphics/5_skill point panel.png") {
            c.draw(&t, 32.0, 180.0);
        }
        // レーンごとのチップの数のグラフ
        let (gx, gy) = (15.0, 368.0);
        if let Some(t) = g.skin.tex("Graphics/5_graph panel drums.png") {
            c.draw(&t, gx, gy);
        }
        if let Some(d) = &details {
            if d.total > 0 {
                let s = d.total.to_string();
                let x = gx + 66.0 - (s.len() as f32 - 1.0) * 6.0;
                BPM_GLYPHS.draw(c, bpm_font.as_ref(), x, gy + 298.0, &s);
            }
            for (i, &n) in d.lanes.iter().enumerate() {
                let h = (n as f32 * 252.0 / 300.0).floor().min(252.0);
                if h > 0.0 {
                    c.fill(
                        gx + 31.0 + i as f32 * 8.0,
                        gy + 21.0 + 252.0 - h,
                        4.0,
                        h,
                        LANE_COLORS[i],
                    );
                }
            }
        }
        // 難易度の表
        let (px, py) = (130.0, 350.0);
        let body = g.skin.tex("Graphics/5_status panel.png");
        let (bw, bh) = body.as_ref().map_or((561.0, 342.0), |t| (t.w, t.h));
        match &body {
            Some(t) => c.draw(t, px, py),
            None => c.frame(px, py, bw, bh, Color32::GRAY),
        }
        let panel = g.skin.tex("Graphics/5_difficulty panel.png");
        let (cw, ch_) = panel.as_ref().map_or((187.0, 58.0), |t| {
            ((t.w / 3.0).floor(), (t.h * 2.0 / 11.0).floor())
        });
        let frame = g.skin.tex("Graphics/5_difficulty frame.png");
        let numbers = g.skin.tex("Graphics/5_level number.png");
        let labels: [Option<String>; LEVELS] = match entry {
            Some(Entry::Node(Node::Song(s))) => level_labels(s),
            Some(Entry::Random) => std::array::from_fn(|i| Some(format!("L{}", i + 1))),
            _ => Default::default(),
        };
        let single = song.is_some_and(|s| {
            labels.iter().all(Option::is_none) && s.charts.iter().any(Option::is_some)
        });
        let selected_level = song.and_then(|s| songs.level_for(s)).unwrap_or(songs.level);
        for col in 0..3 {
            let x = px + bw + cw * (col as f32 - 3.0);
            let y = py + bh - (ch_ * 11.0 / 2.0).floor() - 5.0;
            if let Some(t) = &panel {
                c.part(t, x, y, src(cw * col as f32, 0.0, cw, t.h));
            }
            for (i, label) in labels.iter().enumerate() {
                if label.is_none() {
                    continue;
                }
                let by = 389.0 + (4 - i) as f32 * 60.0;
                if col == 0
                    && selected_level == i
                    && let Some(t) = &frame
                {
                    c.draw(t, x, by);
                }
                let level = if col == 0 {
                    song.and_then(|s| s.charts[i].as_ref())
                        .and_then(|c| c.head.level)
                } else {
                    None
                };
                let s = level.map_or_else(|| "-.--".to_owned(), |l| format!("{l:.2}"));
                LEVEL_GLYPHS.draw(c, numbers.as_ref(), x + cw - 77.0, by + ch_ - 35.0, &s);
            }
            if single {
                // set.def の無い曲は一番上の段に 1 つだけ
                let by = 389.0;
                if col == 0
                    && let Some(t) = &frame
                {
                    c.draw(t, x, by);
                }
                let level = if col == 0 {
                    chart.and_then(|c| c.head.level)
                } else {
                    None
                };
                let s = level.map_or_else(|| "-.--".to_owned(), |l| format!("{l:4.2}"));
                LEVEL_GLYPHS.draw(c, numbers.as_ref(), x + cw - 77.0, by + ch_ - 35.0, &s);
            }
        }
        // 難易度の名前（上流は 8×16 の文字の画像。選んでいる難易度は赤）
        let console = g.skin.tex("Graphics/Console font 8x16.png");
        for (i, label) in labels.iter().enumerate() {
            if let Some(l) = label {
                let font = if selected_level == i { 1 } else { 0 };
                console_print(
                    c,
                    console.as_ref(),
                    140.0 + i as f32 * 110.0,
                    352.0,
                    font,
                    l,
                );
            }
        }
    }

    /// スクロールバー（上流 `CActSelectShowCurrentPosition`）
    fn draw_scrollbar(&self, c: &Canvas, g: &Gfx, songs: &SongList, common: f64) {
        let Some(t) = g.skin.tex("Graphics/5_scrollbar.png") else {
            return;
        };
        let x = 1280.0 - 24.0 + 50.0 - (common * 100.0 / 2.0) as f32;
        let y = 120.0;
        c.part(&t, x, y, src(0.0, 0.0, 12.0, 492.0));
        let n = songs.len();
        if n > 1 {
            let d = (492.0 - 12.0) / (n - 1) as f32;
            let py = d * songs.cursor as f32 - d * self.pending as f32 / 100.0;
            if (0.0..=480.0).contains(&py) {
                c.part(&t, x, y + py, src(0.0, 492.0, 12.0, 12.0));
            }
        } else {
            c.part(&t, x, y, src(0.0, 492.0, 12.0, 12.0));
        }
    }
}

/// 選んでいるものを見分ける（BOX の深さ、行、難易度）
fn selection_key(songs: &SongList) -> (usize, usize, usize) {
    (songs.breadcrumbs().len(), songs.cursor, songs.level)
}

/// 曲の難易度ごとの名前（set.def の名前。set.def の無い曲は無し）
fn level_labels(s: &Song) -> [Option<String>; LEVELS] {
    std::array::from_fn(|i| {
        s.charts[i]
            .as_ref()
            .map(|c| c.label.clone())
            .filter(|l| !l.is_empty())
    })
}

fn chart_for<'a>(songs: &SongList, s: &'a Song) -> Option<&'a SongChart> {
    songs.level_for(s).and_then(|l| s.charts[l].as_ref())
}

/// 上の帯の流れる案内（上流 `CActSelectInformation`：6 秒ごとに 42 ピクセルの段を入れ替える）
fn draw_information(c: &Canvas, g: &Gfx, ms: f64) {
    let Some(t) = g.skin.tex("Graphics/5_information.png") else {
        return;
    };
    let rows = 8i64;
    let cycle = (ms / 6000.0).floor() as i64;
    let lower = cycle.rem_euclid(rows) as f32;
    let upper = (cycle - 1).rem_euclid(rows) as f32;
    let v = ms % 6000.0;
    if v <= 250.0 && cycle > 0 {
        let r = (v / 250.0) as f32;
        let top_h = (42.0 * (1.0 - r)).round();
        c.part(
            &t,
            4.0,
            0.0,
            src(0.0, upper * 42.0 + (42.0 * r).floor(), 240.0, top_h),
        );
        c.part(
            &t,
            4.0,
            (42.0 * (1.0 - r)).floor(),
            src(0.0, lower * 42.0, 240.0, (42.0 * r).floor()),
        );
    } else {
        c.part(&t, 4.0, 0.0, src(0.0, lower * 42.0, 240.0, 42.0));
    }
}

/// スキンが無いときの案内
fn draw_no_skin(c: &Canvas, g: &mut Gfx) {
    c.fill(240.0, 640.0, 800.0, 60.0, Color32::from_black_alpha(200));
    let s = Style::plain(22.0, Color32::WHITE);
    if let Some(t) = g.texts.get(
        "DTXManiaNX のフォルダを選ぶと、NX と同じ画面になります（F1 で設定）",
        &s,
    ) {
        t.draw(c, 640.0 - t.w / 2.0, 655.0, 1.0, 1.0);
    }
}

/// レーンのグラフの色（上流 `clDrumChipsBarColors`）
const LANE_COLORS: [Color32; 9] = [
    Color32::from_rgb(219, 112, 147), // PaleVioletRed
    Color32::from_rgb(0, 191, 255),   // DeepSkyBlue
    Color32::from_rgb(255, 105, 180), // HotPink
    Color32::from_rgb(255, 255, 0),   // Yellow
    Color32::from_rgb(0, 128, 0),     // Green
    Color32::from_rgb(147, 112, 219), // MediumPurple
    Color32::from_rgb(255, 0, 0),     // Red
    Color32::from_rgb(255, 165, 0),   // Orange
    Color32::from_rgb(0, 191, 255),   // DeepSkyBlue
];

/// `5_bpm font.png`（上流 `st数字`）
const BPM_GLYPHS: Glyphs<'static> = Glyphs {
    table: &[
        ('0', src(0.0, 0.0, 12.0, 20.0), 12.0),
        ('1', src(12.0, 0.0, 12.0, 20.0), 12.0),
        ('2', src(24.0, 0.0, 12.0, 20.0), 12.0),
        ('3', src(36.0, 0.0, 12.0, 20.0), 12.0),
        ('4', src(48.0, 0.0, 12.0, 20.0), 12.0),
        ('5', src(60.0, 0.0, 12.0, 20.0), 12.0),
        ('6', src(72.0, 0.0, 12.0, 20.0), 12.0),
        ('7', src(84.0, 0.0, 12.0, 20.0), 12.0),
        ('8', src(96.0, 0.0, 12.0, 20.0), 12.0),
        ('9', src(108.0, 0.0, 12.0, 20.0), 12.0),
        (':', src(123.0, 0.0, 6.0, 20.0), 6.0),
        ('p', src(132.0, 0.0, 12.0, 20.0), 12.0),
    ],
    missing: 12.0,
};

/// `5_level number.png`（上流 `stDifficultyNumber`。`tDrawDifficulty` は 1 文字 20、`.` は 10）
const LEVEL_GLYPHS: Glyphs<'static> = Glyphs {
    table: &[
        ('0', src(0.0, 0.0, 20.0, 28.0), 20.0),
        ('1', src(20.0, 0.0, 20.0, 28.0), 20.0),
        ('2', src(40.0, 0.0, 20.0, 28.0), 20.0),
        ('3', src(60.0, 0.0, 20.0, 28.0), 20.0),
        ('4', src(80.0, 0.0, 20.0, 28.0), 20.0),
        ('5', src(100.0, 0.0, 20.0, 28.0), 20.0),
        ('6', src(120.0, 0.0, 20.0, 28.0), 20.0),
        ('7', src(140.0, 0.0, 20.0, 28.0), 20.0),
        ('8', src(160.0, 0.0, 20.0, 28.0), 20.0),
        ('9', src(180.0, 0.0, 20.0, 28.0), 20.0),
        ('.', src(200.0, 0.0, 10.0, 28.0), 10.0),
        ('-', src(210.0, 0.0, 20.0, 28.0), 20.0),
        ('?', src(230.0, 0.0, 20.0, 28.0), 20.0),
    ],
    missing: 20.0,
};

/// 8×16 の文字の画像で書く（上流 `CCharacterConsole.tPrint`。font は 0 白、1 赤、2 灰）
pub fn console_print(c: &Canvas, t: Option<&Tex>, x: f32, y: f32, font: usize, s: &str) {
    const CHARS: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~ ";
    let Some(t) = t else {
        return;
    };
    let mut x = x;
    for ch in s.chars() {
        if let Some(j) = CHARS.find(ch) {
            let sx = (font / 2) as f32 * 128.0 + (j % 16) as f32 * 8.0;
            let sy = (font % 2) as f32 * 128.0 + (j / 16) as f32 * 16.0;
            c.part(
                t,
                x,
                y,
                Src {
                    x: sx,
                    y: sy,
                    w: 8.0,
                    h: 16.0,
                },
            );
        }
        x += 8.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn row_at_finds_the_bar_under_the_pointer() {
        // 選んでいる曲のバー（文字の 30 上から）
        assert_eq!(SelectView::row_at(pos2(900.0, 245.0)), Some(0));
        assert_eq!(SelectView::row_at(pos2(900.0, 300.0)), Some(0));
        // その上下
        assert_eq!(SelectView::row_at(pos2(900.0, 220.0)), Some(-1));
        assert_eq!(SelectView::row_at(pos2(900.0, 380.0)), Some(1));
        assert_eq!(SelectView::row_at(pos2(900.0, 10.0)), Some(-5));
        assert_eq!(SelectView::row_at(pos2(900.0, 700.0)), Some(7));
        // 曲バーの外
        assert_eq!(SelectView::row_at(pos2(400.0, 300.0)), None);
        assert_eq!(SelectView::row_at(pos2(900.0, 730.0)), None);
    }
}
