//! 選曲画面。NX v2.0（BocuD 版 20260121）の `CStageSongSelectionNew` とその部品
//! （`SongSelectionContainer`・`SongSelectionElement`・`StatusPane`・`SortMenuContainer`・
//! `DensityGraph`）と同じ画像を同じ座標と順で描く。座標と動きの速さは v2 のコードから写している。
//!
//! v2 との違い：背景の動画（5_background.mp4）は同梱せず、止まった絵（5_background.jpg）を出す。
//! v2 の 20260121 では中身の無い密度のグラフに、レーンごとの音符の混み具合を描く。
//!
//! 帯の並びの動きは v2 と同じく、送ると帯が動き、次の帯が半分を過ぎたところで選んでいる曲が変わる。
//! v2 は 1 フレームに 10 ピクセルまでしか動かない（1 秒に 7 本ほど）ので、それより速く送ると送りを捨てる。
//! ここでは送った分だけ速く動くばねにして、速く送っても位置が飛ばず、送りも捨てないようにしている。
//! 曲名の文字とジャケットは別のスレッドで作り、見えている帯と向かう先の帯から先に用意する。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Color32, Pos2};
use library::{LEVELS, Node, Song, SongChart, SortKind};

use crate::gfx::{Canvas, Gfx, Glyphs, Opt, Tex, src};
use crate::jobs::Jobs;
use crate::songs::{Entry, SongList};
use crate::text::{Style, Text};

/// 曲の帯の並びの原点（選んでいる帯の左上）と間隔（v2 `SongSelectionContainer`）
const LIST_X: f32 = 765.0;
const LIST_Y: f32 = 320.0;
const SPACING: f32 = 85.0;
/// 選んでいる帯に近いほど左へ出す（離れが `OFFSET_RANGE` より近ければ、最大 `OFFSET_DISTANCE`）
const OFFSET_RANGE: f32 = 90.0;
const OFFSET_DISTANCE: f32 = 25.0;
/// 選んでいる帯より上と下に描く帯の数（帯のずれは半本分までなので、これで画面の上から下まで埋まる）
const ROWS_AROUND: i64 = 5;
/// 描く帯の外で、文字と画像を前もって用意しておく帯の数（向かう先はさらに送った分だけ）
const ROWS_AHEAD: i64 = 2;
/// 曲名とアーティストの幅の上限（長ければ切り、選んでいる帯では流す）
const TEXT_MAX_W: f32 = 460.0;
/// 長い文字を流す速さ（ピクセル毎秒）と、流れる文字のつなぎ目の間
const SCROLL_SPEED: f32 = 50.0;
const SCROLL_GAP: f32 = 60.0;
/// 大きなジャケット
const ART_X: f32 = 320.0;
const ART_Y: f32 = 35.0;
const ART_SIZE: f32 = 300.0;
/// 並べ替えメニュー（v2 `SortMenuContainer`：右上を (1281, 35) に合わせた 662×92。
/// アイコンは 90 ごとに並び、3 つ目が選んでいるもので 18 下がる）
const SORT_X: f32 = 1281.0 - 662.0;
const SORT_Y: f32 = 35.0;
const SORT_SPACING: f32 = 90.0;
const SORT_SELECTED: usize = 2;
const SORT_DIP: f32 = 18.0;
/// 難易度のパネル（v2 `StatusPane`：左下を (430, 720) に合わせる。難易度ごとに 74 上へ）
const PANE_X: f32 = 430.0;
const PANE_Y: f32 = 720.0;
const PANE_SPACING: f32 = 74.0;
/// 密度のグラフ（v2 `DensityGraph`）
const GRAPH_X: f32 = 212.0;
const GRAPH_Y: f32 = 355.0;
/// グラフのレーン（graph_fg_drums.png の黒い縦の帯：(30, 15) に置いた絵の左から 5、幅 6、間隔 12、高さ 270）
const GRAPH_LANE_X: f32 = GRAPH_X + 30.0 + 5.0;
const GRAPH_LANE_W: f32 = 6.0;
const GRAPH_LANE_STEP: f32 = 12.0;
const GRAPH_TOP: f32 = GRAPH_Y + 15.0;
const GRAPH_H: f32 = 270.0;
/// グラフを縦に分ける数（1 つ 3 ピクセル）
const BINS: usize = 90;
/// 並べ替えメニューの動きの速さ（v2：残りの 10 倍毎秒で近づき、1 フレーム（60 分の 1 秒）に 10 ピクセルまで）
const SPEED: f32 = 10.0;
const MAX_SPEED: f32 = 600.0;
/// 帯の並びのばねの強さ（毎秒）。1 本の送りは 0.07 秒ほどで半分を過ぎ（v2 とほぼ同じ）、0.25 秒ほどで止まる。
/// 続けて送ると、送る速さの 1/12 秒分ほど遅れてついていく（1 秒に 30 本なら 2.4 本後ろ）
const SPRING: f32 = 25.0;
/// 先に送っておける帯の数。これより多い送りは捨てる（1 度に 10 本送る PageDown などはその本数まで）
const MAX_AHEAD: usize = 8;
/// これだけのフレームのあいだ頼み直されなかった譜面の読み込みは捨てる（速く送って通り過ぎた曲）
const STALE_FRAMES: u64 = 20;
/// 選んでからプレビューの音を鳴らすまで（上流の既定 1000 ミリ秒）
pub const PREVIEW_WAIT: Duration = Duration::from_millis(1000);

/// 帯の絵の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bar {
    Song,
    /// 閉じた BOX
    Box,
    /// 開いた BOX（`<< 戻る`）
    Back,
}

/// 帯 1 本に出すもの（v2 `SongSelectionElement.UpdateSongNode`）
struct Row {
    title: String,
    /// 下の小さい文字（アーティスト、BOX の曲の数など）
    sub: String,
    bar: Bar,
    preimage: Preimage,
    /// クリアした一番上の難易度 + 1（0 は無し）。曲だけ
    lamp: Option<usize>,
    /// 一番高いスキルと、その譜面の最大のスキルとの比
    skill: Option<(f64, f64)>,
}

#[derive(Clone)]
enum Preimage {
    File(PathBuf),
    Skin(&'static str),
}

/// 選んでいる譜面を読んで分かること（密度のグラフに出す）
#[derive(Debug, Clone)]
struct Details {
    /// 音符の数
    total: usize,
    /// レーン（LC, HH, LP, SD, HT, BD, LT, FT, CY）ごと、曲の頭から終わりまでを `BINS` に分けた音符の数
    bins: Box<[[u16; BINS]; 9]>,
    /// `bins` の一番大きい数
    peak: u16,
}

/// レーンの並び（graph_fg_drums.png の下のアイコンの順。HH は開きを、LP は左バスを、CY はライドを含む）
fn lane_index(lane: dtx::DrumLane) -> usize {
    use dtx::DrumLane as L;
    match lane {
        L::LC => 0,
        L::HH | L::HHO => 1,
        L::LP | L::LBD => 2,
        L::SD => 3,
        L::HT => 4,
        L::BD => 5,
        L::LT => 6,
        L::FT => 7,
        L::CY | L::RD => 8,
    }
}

fn read_details(path: &Path) -> Option<Details> {
    let bytes = std::fs::read(path).ok()?;
    let chart = dtx::load(&bytes);
    let tempo = dtx::TempoMap::build(&chart);
    let notes = tempo.drum_notes(&chart);
    Some(details_of(&notes))
}

fn details_of(notes: &[dtx::DrumNote]) -> Details {
    let mut bins = Box::new([[0u16; BINS]; 9]);
    let first = notes.iter().map(|n| n.ms).fold(f64::INFINITY, f64::min);
    let last = notes.iter().map(|n| n.ms).fold(f64::NEG_INFINITY, f64::max);
    let span = (last - first).max(1.0);
    for n in notes {
        let b = (((n.ms - first) / span * BINS as f64) as usize).min(BINS - 1);
        let cell = &mut bins[lane_index(n.lane)][b];
        *cell = cell.saturating_add(1);
    }
    let peak = bins.iter().flatten().copied().max().unwrap_or(0);
    Details {
        total: notes.len(),
        bins,
        peak,
    }
}

/// 選んでいるものを見分ける（並べ替え、BOX の深さ、行、難易度）
type SelectionKey = (SortKind, usize, usize, usize);

/// 選曲画面の動き（スクロールや並べ替えメニューのアニメ）
pub struct SelectView {
    /// 帯の並びの、止まる位置からのずれ（ピクセル。下が正）。選んでいる帯は `LIST_Y + scroll` に描く。
    /// 半本分を超えたらカーソルを動かして 1 本分戻すので、いつも ±半本分の中
    scroll: f32,
    /// `scroll` の速さ（ピクセル毎秒）
    velocity: f32,
    /// `scroll` の向かう先。送ると 1 本分ずつ増減し、カーソルが動くと 1 本分戻る（止まると 0）
    target: f32,
    /// 並べ替えメニューの、目標の位置までの残り（ピクセル。右が正）
    sort_pending: f32,
    tick: Instant,
    /// 描いたフレームの数（別のスレッドに頼んだ仕事が要らなくなったかを見るのに使う）
    frame: u64,
    /// 選んでいるものが変わった時刻（プレビューの音と、長い曲名を流すのに使う）
    changed: Instant,
    selected_key: Option<SelectionKey>,
    /// 曲のフォルダの中のファイル名（大文字・小文字を区別しない OS のため）
    found: HashMap<(PathBuf, String), Option<PathBuf>>,
    /// 読み終えた譜面の混み具合（読めなかったものは None）
    details: HashMap<PathBuf, Option<Details>>,
    details_jobs: Jobs<PathBuf, Option<Details>>,
}

impl Default for SelectView {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectView {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            scroll: 0.0,
            velocity: 0.0,
            target: 0.0,
            sort_pending: 0.0,
            tick: now,
            frame: 0,
            changed: now,
            selected_key: None,
            found: HashMap::new(),
            details: HashMap::new(),
            details_jobs: Jobs::new("chart details", 1, |path: &PathBuf| read_details(path)),
        }
    }

    /// 画面に入り直す（v2 と同じく、出入りはシャッターなのでこの画面ではフェードしない）
    pub fn enter(&mut self) {
        self.stop();
        self.sort_pending = 0.0;
        self.tick = Instant::now();
        self.selected_key = None;
    }

    /// 帯の並びを止まった形にする
    fn stop(&mut self) {
        self.scroll = 0.0;
        self.velocity = 0.0;
        self.target = 0.0;
    }

    /// カーソルを `delta` 本動かすよう頼む。帯が動いて、次の帯が半分を過ぎたところでカーソルが動く
    /// （[`SelectView::update`]。v2 と同じ）。先に送っておけるのは [`MAX_AHEAD`] 本まで。送りを受けたら true
    pub fn push(&mut self, delta: isize) -> bool {
        let limit = MAX_AHEAD.max(delta.unsigned_abs()) as f32 * SPACING;
        let before = self.target;
        self.target = (self.target - delta as f32 * SPACING).clamp(-limit, limit);
        self.target != before
    }

    /// 今選んでいる帯から `delta` 本先の帯へ動かす（帯を押したとき。送ってあった分は取り消す）
    pub fn push_to(&mut self, delta: isize) {
        self.target = -(delta as f32) * SPACING;
    }

    /// BOX の出入りなどで一覧が変わった（スクロールせずに切り替える）
    pub fn jumped(&mut self) {
        self.stop();
    }

    /// 並べ替えを `delta` 個となりへ変えた（`SongList::set_sort` の後に呼ぶ）
    pub fn sorted(&mut self, delta: isize) {
        self.stop();
        self.sort_pending = (self.sort_pending + delta as f32 * SORT_SPACING)
            .clamp(-2.0 * SORT_SPACING, 2.0 * SORT_SPACING);
    }

    /// スクロールが止まっていて、選んでから `wait` 経った
    pub fn settled_for(&self, songs: &SongList, wait: Duration) -> bool {
        self.scroll == 0.0
            && self.target == 0.0
            && self.selected_key == Some(selection_key(songs))
            && self.changed.elapsed() >= wait
    }

    /// 時間を進める。帯が半分を過ぎたらカーソルを動かす。描く前に毎フレーム 1 回呼ぶ
    pub fn update(&mut self, songs: &mut SongList) {
        let now = Instant::now();
        let dt = now.duration_since(self.tick).as_secs_f32().min(0.1);
        self.tick = now;
        self.frame += 1;
        if songs.len() == 0 {
            self.stop();
        } else {
            let rows = self.advance(dt);
            if rows != 0 {
                songs.move_by(rows);
            }
        }
        approach(&mut self.sort_pending, dt);
        let stale = self.frame.saturating_sub(STALE_FRAMES);
        for (path, d) in self.details_jobs.poll(stale) {
            if self.details.len() > 500 {
                self.details.clear();
            }
            self.details.insert(path, d);
        }
        // 動きがなめらかかを確かめるための記録（RUST_LOG=scroll_trace=trace で出す。時刻、カーソル、ずれ）
        log::trace!(
            target: "scroll_trace",
            "{} {} {:.2}",
            crate::time::host_us(),
            songs.cursor,
            self.scroll
        );
        let key = selection_key(songs);
        if self.selected_key != Some(key) {
            self.selected_key = Some(key);
            self.changed = now;
        }
    }

    /// 帯の並びを `dt` 秒動かす。帯が半分を過ぎた本数（次の曲の向きが正）を返す。その分カーソルを動かす
    fn advance(&mut self, dt: f32) -> isize {
        spring(&mut self.scroll, &mut self.velocity, self.target, dt);
        let mut rows = 0;
        while self.scroll <= -SPACING / 2.0 {
            rows += 1;
            self.scroll += SPACING;
            self.target += SPACING;
        }
        while self.scroll >= SPACING / 2.0 {
            rows -= 1;
            self.scroll -= SPACING;
            self.target -= SPACING;
        }
        rows
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

    /// 譜面の混み具合。まだ読んでいなければ別のスレッドで読むよう頼む（要るあいだは毎フレーム呼ぶ）
    fn details(&mut self, path: &Path) -> Option<Details> {
        match self.details.get(path) {
            Some(d) => d.clone(),
            None => {
                self.details_jobs.request(&path.to_owned(), self.frame);
                None
            }
        }
    }

    fn row(&mut self, songs: &SongList, e: Entry<'_>) -> Row {
        match e {
            Entry::Back => Row {
                title: "<< 戻る".into(),
                sub: "BOX を出ます。".into(),
                bar: Bar::Back,
                preimage: Preimage::Skin("Graphics/5_preimage backbox.png"),
                lamp: None,
                skill: None,
            },
            Entry::Random => Row {
                title: "< ランダムに選ぶ >".into(),
                sub: "この中から 1 曲を選びます。".into(),
                bar: Bar::Song,
                preimage: Preimage::Skin("Graphics/5_preimage random.png"),
                lamp: None,
                skill: None,
            },
            Entry::Node(Node::Box(b)) => {
                let name = b.preimage.trim();
                let file = (!name.is_empty())
                    .then(|| self.find(&b.dir, name))
                    .flatten();
                Row {
                    title: b.title.clone(),
                    sub: match song_count(&b.nodes) {
                        0 => "曲がありません".to_owned(),
                        n => format!("{n} 曲"),
                    },
                    bar: Bar::Box,
                    preimage: file.map_or(
                        Preimage::Skin("Graphics/5_preimage default.png"),
                        Preimage::File,
                    ),
                    lamp: None,
                    skill: None,
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
                let artist = chart
                    .map(|c| c.head.artist.clone())
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| s.artist.clone());
                // クリアした一番上の難易度（v2 `UpdateLamp`）
                let lamp = (0..LEVELS)
                    .rev()
                    .find(|&i| {
                        s.charts[i]
                            .as_ref()
                            .and_then(|c| songs.stats(&c.path))
                            .is_some_and(|st| st.best_rank.is_some())
                    })
                    .map_or(0, |i| i + 1);
                let skill = library::sort::top_skill(s, songs.all_stats()).and_then(|(i, v)| {
                    let max = s.charts[i].as_ref()?.head.level? * 20.0;
                    Some((v, (v / max).clamp(0.0, 1.0)))
                });
                Row {
                    title: s.title.clone(),
                    sub: artist,
                    bar: Bar::Song,
                    preimage: file.map_or(
                        Preimage::Skin("Graphics/5_preimage default.png"),
                        Preimage::File,
                    ),
                    lamp: Some(lamp),
                    skill,
                }
            }
        }
    }

    /// 描く（先に [`SelectView::update`] を呼んでおく）
    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx, songs: &SongList) {
        // 背景（v2 は上に動画を重ねる）
        if let Some(t) = g.skin.tex("Graphics/5_background.jpg") {
            c.draw(&t, 0.0, 0.0);
        } else {
            c.fill(0.0, 0.0, 1280.0, 720.0, Color32::from_rgb(20, 24, 40));
        }
        self.draw_list(c, g, songs);
        // 左の飾りの紙（v2 は左上を軸に回す。右回りが正）
        if let Some(t) = g.skin.tex("Graphics/SongSelect/back1.png") {
            draw_turned(c, &t, 174.0, 393.0, 1.63);
        }
        if let Some(t) = g.skin.tex("Graphics/SongSelect/back2.png") {
            draw_turned(c, &t, 126.0, 336.0, -0.06);
        }
        self.draw_graph(c, g, songs);
        self.draw_status(c, g, songs);
        self.draw_sort_menu(c, g, songs);
        self.draw_art(c, g, songs);
        if let Some(t) = g.skin.tex("Graphics/SongSelect/top_bar.png") {
            c.draw_opt(
                &t,
                0.0,
                0.0,
                Opt {
                    sx: 1280.0 / t.w,
                    ..Default::default()
                },
            );
        }
    }

    /// 曲の帯の並び（v2 `SongSelectionContainer.Draw`）
    fn draw_list(&mut self, c: &Canvas, g: &mut Gfx, songs: &SongList) {
        let n = songs.len();
        if n == 0 {
            let msg = if songs.scanning().is_some() {
                "曲を探しています…"
            } else {
                "曲が見つかりません"
            };
            if let Some(t) = g.texts.get(
                msg,
                &Style::edged(24.0, Color32::BLACK, Color32::WHITE, 2.0),
            ) {
                t.draw(c, LIST_X + 78.0, LIST_Y + 38.0 - t.h / 2.0, 1.0, 1.0);
            }
            return;
        }
        let row_of = |view: &mut Self, k: i64| {
            let i = (songs.cursor as i64 + k).rem_euclid(n as i64) as usize;
            songs.entry(i).map(|e| view.row(songs, e))
        };
        // 文字と画像を別のスレッドに頼む。後で頼んだものほど先にできるので、遠い帯から頼み、
        // 選んでいる帯を最後にする。描く帯の外も、上下に少しと、送ってある向きにはその分まで
        let ahead = (-self.target / SPACING).round() as i64;
        let lo = -ROWS_AROUND - ROWS_AHEAD + ahead.min(0);
        let hi = ROWS_AROUND + ROWS_AHEAD + ahead.max(0);
        let mut order: Vec<i64> = (lo..=hi).collect();
        order.sort_by_key(|k| std::cmp::Reverse(k.abs()));
        for k in order {
            if let Some(row) = row_of(self, k) {
                request_row(g, &row);
            }
        }
        // 下の帯から描く（上の帯が手前）
        for k in (-ROWS_AROUND..=ROWS_AROUND).rev() {
            let rel = k as f32 * SPACING + self.scroll;
            let y = LIST_Y + rel;
            if y > 720.0 || y + SPACING < 0.0 {
                continue;
            }
            let t = (OFFSET_RANGE - rel.abs()).clamp(0.0, OFFSET_RANGE) / OFFSET_RANGE;
            let x = LIST_X - t * OFFSET_DISTANCE;
            let Some(row) = row_of(self, k) else {
                continue;
            };
            self.draw_row(c, g, &row, x, y, k == 0);
        }
    }

    /// 帯 1 本（v2 `SongSelectionElement`。(x, y) は要素の左上）
    fn draw_row(&mut self, c: &Canvas, g: &mut Gfx, row: &Row, x: f32, y: f32, selected: bool) {
        let bar = match row.bar {
            Bar::Song => g.skin.tex("Graphics/5_bar.png"),
            Bar::Box => g.skin.tex("Graphics/5_box_closed.png"),
            Bar::Back => g.skin.tex("Graphics/5_box_open.png"),
        };
        match bar {
            // BOX の絵は左の 1 列を切って 1 右に置く（v2 の絵の端のずれの直し）
            Some(t) if row.bar != Bar::Song => {
                c.part(
                    &t,
                    x - 39.0,
                    y + 42.0 - t.h / 2.0,
                    src(1.0, 0.0, t.w - 1.0, t.h),
                );
            }
            Some(t) => c.draw(&t, x - 40.0, y + 42.0 - t.h / 2.0),
            None => c.fill(
                x - 40.0,
                y + 1.0,
                638.0,
                82.0,
                Color32::from_rgba_unmultiplied(40, 40, 40, 220),
            ),
        }
        // スキルの帯（中身が下、枠が上）
        if let Some((skill, ratio)) = row.skill {
            if let Some(t) = g.skin.tex("Graphics/5_skillbar_fill.png") {
                c.draw_opt(
                    &t,
                    x + 161.0,
                    y + 12.0,
                    Opt {
                        sx: 286.0 * ratio as f32 / t.w,
                        sy: 8.0 / t.h,
                        ..Default::default()
                    },
                );
            }
            if let Some(t) = g.skin.tex("Graphics/5_skillbar.png") {
                c.draw(&t, x + 82.0, y + 15.0 - t.h / 2.0);
            }
            if let Some(t) = g.texts.ready(&skill_text(skill), &SKILL_STYLE) {
                t.draw(c, x + 105.0, y + 16.0 - t.h / 2.0, 1.0, 1.0);
            }
        }
        if let Some(t) = thumbnail(g, &row.preimage, false) {
            c.draw_opt(
                &t,
                x + 40.0 - 32.5,
                y + 40.0 - 32.5,
                Opt {
                    sx: 65.0 / t.w,
                    sy: 65.0 / t.h,
                    ..Default::default()
                },
            );
        }
        let scroll = selected.then(|| self.changed.elapsed().as_secs_f32() * SCROLL_SPEED);
        if let Some(t) = g.texts.ready(&row.title, &TITLE_STYLE) {
            draw_clipped(c, &t, x + 78.0, y + 38.0 - t.h / 2.0, scroll);
        }
        if !row.sub.trim().is_empty()
            && let Some(t) = g.texts.ready(&row.sub, &SUB_STYLE)
        {
            draw_clipped(c, &t, x + 80.0, y + 60.0 - t.h / 2.0, scroll);
        }
        if let Some(lamp) = row.lamp
            && let Some(t) = g.skin.tex(&format!("Graphics/Lamp/{lamp:02}.png"))
        {
            c.draw(&t, x - 40.0 - t.w / 2.0, y + 40.0 - t.h / 2.0);
        }
    }

    /// 画面の点（NX の座標）にある曲の帯が、選んでいる帯から何本先か（上は負）
    pub fn row_at(&self, p: Pos2) -> Option<isize> {
        if p.x < LIST_X - 40.0 || !(36.0..720.0).contains(&p.y) || Self::sort_at(p).is_some() {
            return None;
        }
        Some(((p.y - LIST_Y - self.scroll) / SPACING).floor() as isize)
    }

    /// 画面の点（NX の座標）にある並べ替えのアイコンが、選んでいるものから何個先か（左は負）
    pub fn sort_at(p: Pos2) -> Option<isize> {
        if !(SORT_Y..SORT_Y + 92.0).contains(&p.y) || p.x < ART_X + ART_SIZE {
            return None;
        }
        let j = ((p.x - SORT_X) / SORT_SPACING).round() as isize;
        Some(j - SORT_SELECTED as isize)
    }

    /// 大きなジャケット（v2 `bigAlbumArt`：300×300 に引き伸ばす）
    fn draw_art(&mut self, c: &Canvas, g: &mut Gfx, songs: &SongList) {
        let Some(entry) = songs.selected_entry() else {
            return;
        };
        let row = self.row(songs, entry);
        if let Some(t) = thumbnail(g, &row.preimage, true) {
            c.draw_opt(
                &t,
                ART_X,
                ART_Y,
                Opt {
                    sx: ART_SIZE / t.w,
                    sy: ART_SIZE / t.h,
                    ..Default::default()
                },
            );
        }
    }

    /// 並べ替えメニュー（v2 `SortMenuContainer`）
    fn draw_sort_menu(&self, c: &Canvas, g: &mut Gfx, songs: &SongList) {
        if let Some(t) = g.skin.tex("Graphics/5_sortmenu_bg.png") {
            c.draw(&t, SORT_X, SORT_Y);
        }
        let center = SORT_X + SORT_SELECTED as f32 * SORT_SPACING;
        let n = SortKind::ALL.len() as isize;
        for j in -1..=n {
            let kind = songs.sort().step(j - SORT_SELECTED as isize);
            let x = SORT_X + j as f32 * SORT_SPACING + self.sort_pending;
            let t = (OFFSET_RANGE - (center - x).abs()).clamp(0.0, OFFSET_RANGE) / OFFSET_RANGE;
            let y = SORT_Y + 40.0 + t * SORT_DIP;
            if let Some(tex) = g.skin.tex(&format!("Graphics/Sorting/{}.png", kind.icon())) {
                c.draw(&tex, x - tex.w / 2.0, y - tex.h / 2.0);
            }
        }
    }

    /// 密度のグラフ（v2 `DensityGraph`。下が曲の頭、上が終わり）
    fn draw_graph(&mut self, c: &Canvas, g: &mut Gfx, songs: &SongList) {
        if let Some(t) = g.skin.tex("Graphics/SongSelect/graph_panel_drums.png") {
            c.draw(&t, GRAPH_X, GRAPH_Y);
        }
        if let Some(t) = g.skin.tex("Graphics/SongSelect/graph_fg_drums.png") {
            c.draw(&t, GRAPH_X + 30.0, GRAPH_Y + 15.0);
        }
        let chart = match songs.selected_entry() {
            Some(Entry::Node(Node::Song(s))) => chart_for(songs, s),
            _ => None,
        };
        let Some(d) = chart.and_then(|ch| self.details(&ch.path)) else {
            return;
        };
        if d.peak > 0 {
            let cell = GRAPH_H / BINS as f32;
            for (lane, bins) in d.bins.iter().enumerate() {
                let x = GRAPH_LANE_X + lane as f32 * GRAPH_LANE_STEP;
                for (b, &count) in bins.iter().enumerate() {
                    if count == 0 {
                        continue;
                    }
                    let a = 0.35 + 0.65 * f32::from(count) / f32::from(d.peak);
                    let y = GRAPH_TOP + GRAPH_H - (b + 1) as f32 * cell;
                    c.fill(
                        x,
                        y,
                        GRAPH_LANE_W,
                        cell,
                        LANE_COLORS[lane].gamma_multiply(a),
                    );
                }
            }
        }
        // 音符の数（パネルの「Total Notes」の下に右寄せ）
        let s = d.total.to_string();
        let font = g.skin.tex("Graphics/5_bpm font.png");
        let right = GRAPH_X + 149.0;
        BPM_GLYPHS.draw(
            c,
            font.as_ref(),
            right - BPM_GLYPHS.width(&s),
            GRAPH_Y + 316.0,
            &s,
        );
    }

    /// 難易度のパネル（v2 `StatusPane`）
    fn draw_status(&mut self, c: &Canvas, g: &mut Gfx, songs: &SongList) {
        if let Some(t) = g.skin.tex("Graphics/5_difficulty_panel.png") {
            c.draw(&t, PANE_X, PANE_Y - t.h);
        }
        let song = match songs.selected_entry() {
            Some(Entry::Node(Node::Song(s))) => Some(s.as_ref()),
            _ => None,
        };
        let level = song.and_then(|s| songs.level_for(s)).unwrap_or(songs.level);
        if let Some(t) = g.skin.tex("Graphics/5_difficultyframe.png") {
            c.draw(
                &t,
                PANE_X - 7.0,
                PANE_Y + 5.0 - PANE_SPACING * level as f32 - t.h,
            );
        }
        let numbers = g.skin.tex("Graphics/5_level number.png");
        let skill_charts = songs.skill_charts();
        for i in 0..LEVELS {
            let (tx, ty) = (PANE_X + 125.0, PANE_Y - 41.0 - PANE_SPACING * i as f32);
            let chart = song.and_then(|s| s.charts[i].as_ref());
            let Some(chart) = chart else {
                draw_level_number(c, numbers.as_ref(), tx, ty, 1.0, "-.--");
                continue;
            };
            let text = chart
                .head
                .level
                .map_or_else(|| "-.--".to_owned(), |l| format!("{l:.2}"));
            draw_level_number(c, numbers.as_ref(), tx, ty, 1.0, &text);
            let Some(st) = songs.stats(&chart.path) else {
                continue;
            };
            let Some(rank) = st.best_rank else {
                continue;
            };
            // ランクとスキルのアイコン（27×27）、達成率
            let icon_y = PANE_Y - 49.0 - PANE_SPACING * i as f32 - 27.0;
            if skill_charts.contains(&chart.path)
                && let Some(t) = g.skin.tex("Graphics/Rank/skill.png")
            {
                draw_square(c, &t, PANE_X + 14.0, icon_y, 27.0);
            }
            if let Some(t) = g.skin.tex(&format!("Graphics/Rank/rank_{rank}.png")) {
                draw_square(c, &t, PANE_X + 60.0, icon_y, 27.0);
            }
            if let Some(rate) = st.best_rate {
                draw_level_number(
                    c,
                    numbers.as_ref(),
                    tx - 102.0,
                    ty + 11.0,
                    0.6,
                    &format!("{rate:.2}%"),
                );
            }
        }
    }
}

/// 曲名、下の小さい文字、スキルの数の見た目
const TITLE_STYLE: Style = Style {
    px: 24.0,
    color: Color32::BLACK,
    bold: false,
    shadow: None,
    edge: Some((Color32::WHITE, 2.0)),
    gradient: None,
};
const SUB_STYLE: Style = Style {
    px: 16.0,
    color: Color32::BLACK,
    bold: false,
    shadow: None,
    edge: Some((Color32::WHITE, 1.5)),
    gradient: None,
};
const SKILL_STYLE: Style = Style {
    px: 16.0,
    color: Color32::WHITE,
    bold: false,
    shadow: None,
    edge: Some((Color32::from_black_alpha(160), 1.0)),
    gradient: None,
};

fn skill_text(skill: f64) -> String {
    format!("{skill:.2}")
}

/// 帯 1 本の文字と画像を、別のスレッドで用意するよう頼む
fn request_row(g: &mut Gfx, row: &Row) {
    if let Preimage::File(path) = &row.preimage {
        g.images.get(path);
    }
    if let Some((skill, _)) = row.skill {
        g.texts.request(&skill_text(skill), &SKILL_STYLE);
    }
    if !row.sub.trim().is_empty() {
        g.texts.request(&row.sub, &SUB_STYLE);
    }
    g.texts.request(&row.title, &TITLE_STYLE);
}

/// 帯の小さいジャケットか大きなジャケットの画像。`request` なら、まだ読んでいなければ読むよう頼む
/// （帯の分は [`request_row`] で頼んである）
fn thumbnail(g: &mut Gfx, p: &Preimage, request: bool) -> Option<Tex> {
    match p {
        Preimage::File(path) => {
            let t = if request {
                g.images.get(path)
            } else {
                g.images.ready(path)
            };
            match t {
                Some(t) => Some(t),
                None if g.images.failed(path) => g.skin.tex("Graphics/5_preimage default.png"),
                None => None,
            }
        }
        Preimage::Skin(rel) => g.skin.tex(rel),
    }
}

/// 臨界減衰のばねで `x`（速さ `v`）を `target` へ寄せる。行き過ぎず、速さが途切れないので、
/// 続けて送っても動きがなめらか。式で解くので、フレームの長さによらず同じ動きになる
fn spring(x: &mut f32, v: &mut f32, target: f32, dt: f32) {
    let e = *x - target;
    if e.abs() < 0.5 && v.abs() < 20.0 {
        *x = target;
        *v = 0.0;
        return;
    }
    let c = *v + SPRING * e;
    let k = (-SPRING * dt).exp();
    *x = target + (e + c * dt) * k;
    *v = (*v - SPRING * c * dt) * k;
}

/// 残りを 0 へ近づける（v2：残りの `SPEED` 倍毎秒、1 秒に `MAX_SPEED` ピクセルまで）
fn approach(v: &mut f32, dt: f32) {
    if v.abs() < 0.5 {
        *v = 0.0;
        return;
    }
    let step = (*v * (1.0 - (-SPEED * dt).exp())).clamp(-MAX_SPEED * dt, MAX_SPEED * dt);
    *v -= step;
}

/// 左上を (x, y) に置き、そこを軸に右回りに `angle` ラジアン回して描く（v2 の UIImage の rotation）
fn draw_turned(c: &Canvas, t: &Tex, x: f32, y: f32, angle: f32) {
    let (sin, cos) = angle.sin_cos();
    let (hw, hh) = (t.w / 2.0, t.h / 2.0);
    let cx = x + cos * hw - sin * hh;
    let cy = y + sin * hw + cos * hh;
    c.draw_rot(t, cx, cy, -angle, Opt::default());
}

/// 画像を `size` の正方形に引き伸ばして描く
fn draw_square(c: &Canvas, t: &Tex, x: f32, y: f32, size: f32) {
    c.draw_opt(
        t,
        x,
        y,
        Opt {
            sx: size / t.w,
            sy: size / t.h,
            ..Default::default()
        },
    );
}

/// 幅が `TEXT_MAX_W` を超える文字は切る。`scroll`（流した距離）があれば右から左へ流す
/// （v2 `HorizontallyScrollingText`）
fn draw_clipped(c: &Canvas, t: &Text, x: f32, y: f32, scroll: Option<f32>) {
    if t.w <= TEXT_MAX_W {
        t.draw(c, x, y, 1.0, 1.0);
        return;
    }
    let clip = c
        .painter()
        .clip_rect()
        .intersect(c.rect(x, y - 4.0, TEXT_MAX_W, t.h + 8.0));
    let sub = c.with_clip(clip);
    match scroll {
        Some(d) => {
            let span = t.w + SCROLL_GAP;
            let off = d % span;
            t.draw(&sub, x - off, y, 1.0, 1.0);
            t.draw(&sub, x - off + span, y, 1.0, 1.0);
        }
        None => t.draw(&sub, x, y, 1.0, 1.0),
    }
}

/// `5_level number.png` で数を書く（v2 `StatusPane.tDrawDifficulty`）。
/// 小数点より前の文字は 1.35 倍にして、下をそろえるように上げる。絵は 1 文字 20×28 の何倍かの大きさ
fn draw_level_number(c: &Canvas, t: Option<&Tex>, x: f32, y: f32, scale: f32, text: &str) {
    let Some(t) = t else {
        return;
    };
    let mult = t.h / 28.0;
    let mut x = x;
    let mut decimal = false;
    for ch in text.chars() {
        let Some(&(_, s, _)) = LEVEL_GLYPHS.table.iter().find(|(k, _, _)| *k == ch) else {
            continue;
        };
        if ch == '.' {
            decimal = true;
        }
        let k = if decimal { scale } else { scale * 1.35 };
        let raise = if decimal {
            0.0
        } else {
            (s.h * k - s.h * scale) * 111.0 / 128.0
        };
        c.draw_opt(
            t,
            x,
            y - raise,
            Opt {
                src: Some(src(s.x * mult, s.y * mult, s.w * mult, s.h * mult)),
                sx: k / mult,
                sy: k / mult,
                ..Default::default()
            },
        );
        x += s.w * k;
    }
}

/// 選んでいるものを見分ける
fn selection_key(songs: &SongList) -> SelectionKey {
    (
        songs.sort(),
        songs.breadcrumbs().len(),
        songs.cursor,
        songs.level,
    )
}

/// BOX の中の曲の数（中の BOX の曲も数える）
fn song_count(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| match n {
            Node::Box(b) => song_count(&b.nodes),
            Node::Song(_) => 1,
        })
        .sum()
}

fn chart_for<'a>(songs: &SongList, s: &'a Song) -> Option<&'a SongChart> {
    songs.level_for(s).and_then(|l| s.charts[l].as_ref())
}

/// レーンのグラフの色（v2 `DensityGraph.clDrumChipsBarColors`）
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
    ],
    missing: 12.0,
};

/// `5_level number.png` の 1 文字（v2 `stDifficultyNumber`。28 の高さの絵での座標）
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
        ('%', src(250.0, 0.0, 20.0, 28.0), 20.0),
    ],
    missing: 20.0,
};

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn row_at_finds_the_bar_under_the_pointer() {
        let mut v = SelectView::new();
        // 選んでいる帯は 320 から 85
        assert_eq!(v.row_at(pos2(900.0, 330.0)), Some(0));
        assert_eq!(v.row_at(pos2(900.0, 404.0)), Some(0));
        assert_eq!(v.row_at(pos2(900.0, 300.0)), Some(-1));
        assert_eq!(v.row_at(pos2(900.0, 410.0)), Some(1));
        assert_eq!(v.row_at(pos2(900.0, 700.0)), Some(4));
        assert_eq!(v.row_at(pos2(900.0, 200.0)), Some(-2));
        // 帯の外、上の帯、並べ替えメニュー
        assert_eq!(v.row_at(pos2(400.0, 330.0)), None);
        assert_eq!(v.row_at(pos2(900.0, 20.0)), None);
        assert_eq!(v.row_at(pos2(900.0, 80.0)), None);
        // 動いている途中は、ずれた位置の帯
        v.scroll = 30.0;
        assert_eq!(v.row_at(pos2(900.0, 330.0)), Some(-1));
        assert_eq!(v.row_at(pos2(900.0, 360.0)), Some(0));
    }

    /// `fps` で `secs` 秒動かし、フレームごとの (時刻, 並びの位置（ピクセル。次の曲の向きが正）) を返す。
    /// `presses` は送る時刻（秒）と本数
    fn simulate(fps: f32, secs: f32, presses: &[(f32, isize)]) -> (SelectView, Vec<(f32, f32)>) {
        let mut v = SelectView::new();
        let mut cursor = 0isize;
        let mut out = Vec::new();
        let dt = 1.0 / fps;
        let mut next = 0;
        for i in 0..(secs * fps).round() as usize {
            let t = i as f32 * dt;
            while next < presses.len() && presses[next].0 <= t {
                v.push(presses[next].1);
                next += 1;
            }
            cursor += v.advance(dt);
            out.push((t + dt, cursor as f32 * SPACING - v.scroll));
        }
        (v, out)
    }

    #[test]
    fn one_press_moves_one_bar_without_overshoot() {
        let (v, pos) = simulate(60.0, 1.0, &[(0.0, 1)]);
        // 0.3 秒ほどで止まり、1 本分ちょうど
        assert_eq!(v.scroll, 0.0);
        assert_eq!(v.target, 0.0);
        assert_eq!(pos.last().unwrap().1, SPACING);
        let settled = pos.iter().position(|&(_, p)| p == SPACING).unwrap();
        assert!(pos[settled].0 < 0.4, "{}", pos[settled].0);
        // 行き過ぎず、戻らない
        assert!(pos.windows(2).all(|w| w[1].1 >= w[0].1));
        assert!(pos.iter().all(|&(_, p)| p <= SPACING));
        // カーソルは帯が半分を過ぎたところで動く（v2 と同じく 0.1 秒以内）
        let half = pos.iter().position(|&(_, p)| p >= SPACING / 2.0).unwrap();
        assert!(pos[half].0 < 0.1, "{}", pos[half].0);
    }

    #[test]
    fn fast_presses_move_smoothly_without_jumps() {
        // 1 秒に 30 本（キーの押しっぱなしやホイール）を 1 秒
        let presses: Vec<(f32, isize)> = (0..30).map(|i| (i as f32 / 30.0, 1)).collect();
        for fps in [60.0, 144.0] {
            let (v, pos) = simulate(fps, 2.0, &presses);
            assert_eq!(v.target, 0.0);
            assert_eq!(pos.last().unwrap().1, 30.0 * SPACING);
            let steps: Vec<f32> = pos.windows(2).map(|w| w[1].1 - w[0].1).collect();
            // 戻らない
            assert!(steps.iter().all(|&d| d >= 0.0), "{fps}");
            // 速さが急に変わらない（となりのフレームとの差が、1 秒に 30 本の 1 フレーム分の半分まで）
            let per_frame = 30.0 * SPACING / fps;
            for w in steps.windows(2) {
                assert!((w[1] - w[0]).abs() < per_frame * 0.5, "{fps}: {w:?}");
            }
            // 遅れは 4 本まで
            let lag = pos
                .iter()
                .map(|&(t, p)| (t * 30.0).ceil().min(30.0) * SPACING - p)
                .fold(0.0, f32::max);
            assert!(lag < 4.0 * SPACING, "{fps}: {lag}");
        }
    }

    #[test]
    fn motion_does_not_depend_on_the_frame_rate() {
        let presses = [(0.0, 1), (0.05, 1), (0.1, 3), (0.4, -2)];
        let (_, a) = simulate(60.0, 1.0, &presses);
        let (_, b) = simulate(240.0, 1.0, &presses);
        for &(t, p) in &a {
            // 押した時刻がフレームに丸められる分だけ違ってよい
            let q = b.iter().find(|&&(u, _)| (u - t).abs() < 1e-3).unwrap().1;
            assert!((p - q).abs() < 15.0, "t {t}: {p} vs {q}");
        }
    }

    #[test]
    fn presses_beyond_the_limit_are_dropped() {
        let mut v = SelectView::new();
        for _ in 0..MAX_AHEAD {
            assert!(v.push(1));
        }
        assert!(!v.push(1));
        assert_eq!(v.target, -(MAX_AHEAD as f32) * SPACING);
        // 逆へはすぐ受ける
        assert!(v.push(-1));
        // 1 度に 10 本（PageDown）は 10 本まで
        let mut v = SelectView::new();
        assert!(v.push(10));
        assert_eq!(v.target, -10.0 * SPACING);
        // 押した帯へは、送ってあった分を取り消して動く
        v.push_to(-2);
        assert_eq!(v.target, 2.0 * SPACING);
    }

    #[test]
    fn sort_at_finds_the_icon_under_the_pointer() {
        // 選んでいるアイコンは x = 619 + 180
        assert_eq!(SelectView::sort_at(pos2(799.0, 90.0)), Some(0));
        assert_eq!(SelectView::sort_at(pos2(889.0, 70.0)), Some(1));
        assert_eq!(SelectView::sort_at(pos2(700.0, 70.0)), Some(-1));
        assert_eq!(SelectView::sort_at(pos2(799.0, 200.0)), None);
        assert_eq!(SelectView::sort_at(pos2(500.0, 70.0)), None);
    }

    #[test]
    fn approach_slows_down_and_stops() {
        let mut v = 40.0;
        approach(&mut v, 1.0 / 60.0);
        // 残りの 10 倍毎秒（1 フレームで約 15%）、10 ピクセルまで
        assert!(
            (v - (40.0 - 40.0 * (1.0 - (-10.0f32 / 60.0).exp()))).abs() < 1e-3,
            "{v}"
        );
        let mut far = 170.0;
        approach(&mut far, 1.0 / 60.0);
        assert!((far - 160.0).abs() < 1e-3, "{far}");
        for _ in 0..200 {
            approach(&mut v, 1.0 / 60.0);
        }
        assert_eq!(v, 0.0);
    }

    #[test]
    fn density_spreads_notes_from_start_to_end() {
        use dtx::{DrumLane, DrumNote, NotePos};
        let note = |ms: f64, lane| DrumNote {
            ms,
            lane,
            wav: 0,
            at: NotePos::new(0, 0, 1),
            beat: 0.0,
        };
        let d = details_of(&[
            note(1000.0, DrumLane::BD),
            note(1000.0, DrumLane::HHO),
            note(3000.0, DrumLane::HH),
            note(5000.0, DrumLane::RD),
        ]);
        assert_eq!(d.total, 4);
        assert_eq!(d.bins[5][0], 1);
        assert_eq!(d.bins[1][0], 1);
        assert_eq!(d.bins[1][BINS / 2], 1);
        assert_eq!(d.bins[8][BINS - 1], 1);
        assert_eq!(d.peak, 1);
    }
}
