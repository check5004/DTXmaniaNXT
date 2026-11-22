//! 画面。egui で描く。結果の 2 ページ目（分析）と、演奏中の情報の窓、知らせ。
//! 画面の関数は状態を変えず、押されたものを返す（変えるのは main.rs）。

use std::path::PathBuf;

use dtx::DrumLane;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use engine::analysis::{NOTABLE_MEAN_MS, Subdivision, Tendency, WeakPhrase, notable_tendencies};
use engine::input::{Articulation, Pad};
use engine::judge::Judgement;
use engine::practice::Section;

use crate::config::Config;
use crate::game::{Outcome, Play, WeakMap};
use crate::gfx::{Canvas, Gfx};
use crate::history::{OvercomeCheck, TENDENCY_MIN_HITS, TENDENCY_SESSIONS};
use crate::inputs::lane_label;

fn judgement_label(j: Judgement) -> (&'static str, Color32) {
    match j {
        Judgement::Perfect => ("PERFECT", Color32::from_rgb(0xFF, 0xE0, 0x60)),
        Judgement::Great => ("GREAT", Color32::from_rgb(0x70, 0xE0, 0x90)),
        Judgement::Good => ("GOOD", Color32::from_rgb(0x70, 0xB0, 0xF0)),
        Judgement::Poor => ("POOR", Color32::from_rgb(0xB0, 0xB0, 0xB0)),
        Judgement::Miss => ("MISS", Color32::from_rgb(0xF0, 0x60, 0x60)),
    }
}

/// 選曲の画面で押されたもの
#[derive(Debug, Clone, PartialEq)]
pub enum SelectAction {
    /// 選んでいる行に入る（BOX なら中へ、曲なら始める）
    Enter,
    Back,
    Rescan,
    /// 選んでいる曲の区間を練習する
    Practice,
}

/// 結果の画面で押されたもの
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResultAction {
    Back,
    /// もう一度（練習なら同じ区間を）
    Retry,
    /// 区間を選ぶ画面へ。`Some` ならその区間を選んでおく
    Practice(Option<Section>),
}

pub fn draw_result(ui: &mut egui::Ui, g: &mut Gfx, o: &Outcome) -> Vec<ResultAction> {
    let mut actions = Vec::new();
    result_page(ui, g, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                ui.heading(RichText::new(&o.title).size(28.0));
                if !o.artist.is_empty() {
                    ui.label(&o.artist);
                }
                if let Some(lv) = o.level {
                    ui.label(format!("Lv {lv:.2}"));
                }
                if !o.cleared {
                    ui.label(RichText::new("途中でやめました").weak());
                }
                ui.add_space(24.0);
                let t = &o.tally;
                for (j, n) in [
                    (Judgement::Perfect, t.perfect),
                    (Judgement::Great, t.great),
                    (Judgement::Good, t.good),
                    (Judgement::Poor, t.poor),
                    (Judgement::Miss, t.miss),
                ] {
                    let (name, color) = judgement_label(j);
                    ui.label(
                        RichText::new(format!("{name:<8} {n:>5}"))
                            .monospace()
                            .size(20.0)
                            .color(color),
                    );
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("最大コンボ {} / {}", t.max_combo, o.notes)).size(18.0),
                );
                ui.label(format!("空打ち {}", t.stray));
                ui.add_space(16.0);
                if let Some(mean) = o.offset_mean_ms {
                    let lean = if mean > 2.0 {
                        "遅め"
                    } else if mean < -2.0 {
                        "早め"
                    } else {
                        "ちょうど"
                    };
                    let sd = o
                        .offset_sd_ms
                        .map_or(String::new(), |s| format!("、ばらつき ±{s:.1}ms"));
                    ui.label(format!("ずれの平均 {mean:+.1}ms（{lean}）{sd}"));
                    if o.lag.is_some() {
                        ui.label(
                            RichText::new(
                                "補正のおすすめは、1 ページ目の「ずれ」の板に出します（直近のプレイから）。",
                            )
                            .weak(),
                        );
                    }
                }
                if !o.overcome.is_empty() {
                    ui.add_space(24.0);
                    draw_overcome(ui, &o.overcome);
                }
                if let Some(w) = &o.weak {
                    ui.add_space(24.0);
                    if let Some(section) = draw_weak_map(ui, w) {
                        actions.push(ResultAction::Practice(Some(section)));
                    }
                }
                ui.add_space(24.0);
                draw_tendencies(ui, &o.tendencies);
                ui.add_space(24.0);
                if ui
                    .button("選曲に戻る（Enter、ドラムは右シンバルかライド）")
                    .clicked()
                {
                    actions.push(ResultAction::Back);
                }
                if ui.button("もう一度（R）").clicked() {
                    actions.push(ResultAction::Retry);
                }
                if ui.button("区間を練習（P）").clicked() {
                    actions.push(ResultAction::Practice(None));
                }
                ui.add_space(24.0);
            });
        });
    });
    actions
}

/// 結果の 2 ページ目の板の色と縁（1 ページ目の WEAK MEASURES など、NX の部品で描いた板と同じ）
const PAGE_FILL: Color32 = Color32::from_rgba_unmultiplied_const(8, 8, 16, 225);
const PAGE_EDGE: Color32 = Color32::from_gray(150);
/// 見出しの高さ（NX の座標）
const PAGE_HEADING_H: f32 = 30.0;

/// 結果の 2 ページ目の枠。NX の結果の背景の上に、ほかの NX の板と同じ暗い板を真ん中に置き、
/// 左上に DFP と同じ高さの日本語の見出しを出す
fn result_page(ui: &mut egui::Ui, g: &mut Gfx, body: impl FnOnce(&mut egui::Ui)) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            left: 16,
            right: 16,
            top: 16,
            bottom: 40,
        }))
        .show(ui, |ui| {
            let width = ui.available_width().min(1000.0);
            let height = ui.available_height();
            ui.vertical_centered(|ui| {
                egui::Frame::new()
                    .fill(PAGE_FILL)
                    .stroke(Stroke::new(1.0, PAGE_EDGE))
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(width - 32.0);
                        ui.set_max_height(height - 32.0);
                        // 見出し（スクロールしない）
                        let c = Canvas::overlay(ui.painter(), ui.ctx().content_rect());
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), PAGE_HEADING_H * c.scale()),
                            Sense::hover(),
                        );
                        let at = c.to_nx(rect.min);
                        g.heading(&c, at.x - 2.0, at.y - 8.0, "結果の分析");
                        // 背景が透けるので、文字を明るくする（弱い文字はこれを薄めた色になる）
                        ui.visuals_mut().override_text_color = Some(Color32::from_gray(225));
                        body(ui);
                    });
            });
        });
}

/// 練習でクリアした区間を、今回のプレイで克服したか
fn draw_overcome(ui: &mut egui::Ui, checks: &[OvercomeCheck]) {
    for c in checks {
        let change = match (c.before, c.now) {
            (Some(b), Some(n)) => format!("苦手度 {b:.2} → {n:.2}"),
            (None, Some(n)) => format!("苦手度 {n:.2}"),
            _ => String::new(),
        };
        if c.overcome {
            ui.label(
                RichText::new(format!(
                    "克服！ {}（{}% でクリアした区間。{change}）",
                    section_text(c.section),
                    c.speed
                ))
                .size(20.0)
                .color(PASS_COLOR),
            );
        } else {
            ui.label(format!(
                "{} は {}% でクリアした区間ですが、今回はまだ崩れました（今回の点 {:.2}、{change}）",
                section_text(c.section),
                c.speed,
                c.this_play
            ));
        }
    }
}

/// すべての曲を通した叩き方のくせ。目立つものを文で、全部を表で
fn draw_tendencies(ui: &mut egui::Ui, t: &[Tendency]) {
    ui.label(
        RichText::new(format!(
            "叩き方のくせ（すべての曲の、等速で叩いた直近 {TENDENCY_SESSIONS} 回から）"
        ))
        .size(18.0),
    );
    if t.is_empty() {
        ui.label(format!(
            "まだ記録が足りません（レーンと拍の中の位置の組ごとに {TENDENCY_MIN_HITS} 打から出します）。"
        ));
        return;
    }
    let notable = notable_tendencies(t, 3);
    if notable.is_empty() {
        ui.label(format!(
            "目立つくせはありません（どの組もずれの平均が {NOTABLE_MEAN_MS:.0}ms 未満）。"
        ));
    }
    for n in &notable {
        ui.label(
            RichText::new(format!(
                "{}の{}{}: 平均 {:.0}ms {}（ばらつき ±{:.0}ms、{} 打）",
                lane_label(n.lane),
                // 数字で始まるときは間をあける
                if sub_name(n.sub).starts_with(|c: char| c.is_ascii_digit()) {
                    " "
                } else {
                    ""
                },
                sub_name(n.sub),
                n.mean_ms.abs(),
                if n.mean_ms > 0.0 { "遅い" } else { "早い" },
                n.sd_ms,
                n.hits
            ))
            .color(tendency_color(n.mean_ms)),
        );
    }
    egui::CollapsingHeader::new("くせの表（レーン × 拍の中の位置、ずれの平均 ± ばらつき）")
        .id_salt("tendencies")
        .show(ui, |ui| {
            let lanes: Vec<DrumLane> = DrumLane::ALL
                .into_iter()
                .filter(|l| t.iter().any(|x| x.lane == *l))
                .collect();
            egui::Grid::new("tendency_grid")
                .striped(true)
                .spacing(Vec2::new(16.0, 4.0))
                .show(ui, |ui| {
                    ui.label("");
                    for sub in Subdivision::ALL {
                        ui.label(RichText::new(sub_name(sub)).strong());
                    }
                    ui.end_row();
                    for lane in lanes {
                        ui.label(RichText::new(lane_label(lane)).strong());
                        for sub in Subdivision::ALL {
                            match t.iter().find(|x| x.lane == lane && x.sub == sub) {
                                Some(x) => {
                                    ui.label(
                                        RichText::new(format!("{:+.0} ±{:.0}", x.mean_ms, x.sd_ms))
                                            .monospace()
                                            .color(tendency_color(x.mean_ms)),
                                    )
                                    .on_hover_text(format!("{} 打", x.hits));
                                }
                                None => {
                                    ui.label(RichText::new("-").weak());
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
            ui.label(RichText::new("ミリ秒。+ は遅い、- は早い").weak());
        });
}

fn sub_name(s: Subdivision) -> &'static str {
    match s {
        Subdivision::OnBeat => "表拍",
        Subdivision::Eighth => "8 分裏",
        Subdivision::Sixteenth => "16 分裏",
        Subdivision::Triplet => "3 連符",
        Subdivision::Other => "細かい位置",
    }
}

/// ずれの色。目立つほど、遅ければ赤、早ければ青
fn tendency_color(mean_ms: f64) -> Color32 {
    if mean_ms.abs() < NOTABLE_MEAN_MS {
        Color32::from_gray(180)
    } else if mean_ms > 0.0 {
        Color32::from_rgb(0xF0, 0x90, 0x70)
    } else {
        Color32::from_rgb(0x70, 0xB0, 0xF0)
    }
}

/// 合格・克服の色
const PASS_COLOR: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);

fn section_text(s: Section) -> String {
    if s.first == s.last {
        format!("小節 {}", s.first)
    } else {
        format!("小節 {}〜{}", s.first, s.last)
    }
}

fn phrase_text(p: &WeakPhrase) -> String {
    format!(
        "{}　苦手度 {:.2}（音符 {}）",
        section_text(Section::new(p.first, p.last)),
        p.weakness,
        p.notes
    )
}

/// 苦手度をどの記録から出したか
fn weak_source(w: &WeakMap) -> String {
    match (w.plays, w.practice_laps) {
        (0, 0) => "まだ記録がありません".to_owned(),
        (p, 0) => format!("直近のプレイ {p} 回の記録から"),
        (0, l) => format!("直近の練習 {l} 周の記録から"),
        (p, l) => format!("直近のプレイ {p} 回と練習 {l} 周の記録から"),
    }
}

/// 遊ぶ譜面のパス（ドロップや引数で渡されたもの）の扱い
pub fn is_chart_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dtx"))
}

/// フォルダを選ぶ（OS のダイアログ）
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).pick_folder()
}

/// 演奏の画面で、レーンの下に短い知らせを出す
pub fn play_toast(ctx: &egui::Context, c: &Canvas, msg: &str) {
    egui::Area::new(egui::Id::unique("play-toast"))
        .fixed_pos(c.pos(574.0, 676.0))
        .pivot(Align2::CENTER_TOP)
        .interactable(false)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_black_alpha(210))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(14, 6))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(msg)
                            .color(Color32::WHITE)
                            .size(15.0 * c.scale()),
                    );
                });
        });
}

/// 画面の下に短い知らせを出す
pub fn message_overlay(ctx: &egui::Context, msg: &str) {
    egui::Area::new(egui::Id::unique("message"))
        .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -24.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_black_alpha(200))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.label(RichText::new(msg).color(Color32::WHITE).size(16.0));
                });
        });
}

/// 演奏中に NX の画面へ重ねるもの
pub struct PlayOverlay<'a> {
    pub play: &'a Play,
    pub config: &'a Config,
    pub midi_ports: &'a [String],
    /// 設定と入力の情報を出す（I で切り替え）
    pub show_info: bool,
}

/// 演奏中の重ね書き。情報の窓（左、I で切り替え）。練習中の表示は practice_hud で NX の部品で描く
pub fn draw_play_overlay(ctx: &egui::Context, c: &Canvas, o: &PlayOverlay<'_>) {
    let s = c.scale();
    if o.show_info {
        nx_panel(ctx, "play-info", c.pos(6.0, 250.0), 282.0 * s, |ui| {
            play_info(ui, o, s);
        });
    }
}

/// NX の画面に重ねる、半透明の黒い欄
fn nx_panel(ctx: &egui::Context, id: &str, at: Pos2, width: f32, body: impl FnOnce(&mut egui::Ui)) {
    egui::Area::new(egui::Id::unique(id))
        .fixed_pos(at)
        .interactable(false)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_black_alpha(190))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(width - 16.0);
                    ui.style_mut().visuals.override_text_color = Some(Color32::from_gray(230));
                    ui.spacing_mut().item_spacing.y = 2.0;
                    body(ui);
                });
        });
}

/// 設定と入力の情報（I で出す）
fn play_info(ui: &mut egui::Ui, o: &PlayOverlay<'_>, s: f32) {
    let play = o.play;
    let small = |t: String| RichText::new(t).size(11.0 * s);
    match &play.audio {
        Some(a) => ui.label(small(format!("音: {}", a.device_name))),
        None => ui.label(small("音: 出力なし（時計はシステム時刻）".to_owned())),
    };
    if o.midi_ports.is_empty() {
        ui.label(small(
            "MIDI 入力: 見つかりません（キーボードで叩けます）".to_owned(),
        ));
    } else {
        ui.label(small(format!("MIDI 入力: {}", o.midi_ports.join(", "))));
    }
    ui.separator();
    ui.label(small(format!(
        "入力の補正 {:+.0}ms（- / =）",
        play.input_offset_ms
    )));
    ui.label(small(format!(
        "表示の補正 {:+.0}ms（[ / ]）",
        o.config.visual_offset_ms
    )));
    ui.label(small(format!(
        "譜面の速さ {:.2}（↑ / ↓）",
        o.config.scroll_px_per_ms
    )));
    ui.label(small(format!(
        "叩いた音 {}（{}）",
        if play.drum_sounds {
            "鳴らす"
        } else {
            "鳴らさない"
        },
        crate::HIT_SOUND_KEY_NAME
    )));
    ui.label(small(if play.practice.is_some() {
        "←→ 区間を 1 小節ずらす、, . 速度 ±5%、R 最初から、Esc か Space 一時停止（やめるのもここから）".to_owned()
    } else {
        "←→ 巻き戻し・早送り、R やり直し、Esc か Space 一時停止（やめるのもここから）".to_owned()
    }));
    ui.separator();
    match &play.last_input {
        Some(i) => {
            ui.label(small(format!(
                "最後の入力 {} / {} / {}",
                i.source,
                i.pad.map_or("-", pad_label),
                i.articulation.map_or("-", articulation_label)
            )));
            ui.label(small(format!(
                "レーン {} / ベロシティ {} / ハイハットのペダル {}",
                i.lane.map_or("-", lane_label),
                i.velocity,
                opt(i.hh_pedal)
            )));
        }
        None => {
            ui.label(small("最後の入力 -".to_owned()));
        }
    }
    ui.label(small(format!(
        "今のハイハットのペダル {}（0 で開、127 で閉）",
        opt(play.hh_pedal)
    )));
}

/// 機器のパッドの名前（演奏中の情報の窓に出す）
fn pad_label(p: Pad) -> &'static str {
    match p {
        Pad::Kick => "キック",
        Pad::Snare => "スネア",
        Pad::Tom1 => "タム 1",
        Pad::Tom2 => "タム 2",
        Pad::Tom3 => "タム 3",
        Pad::Tom4 => "タム 4",
        Pad::HiHat => "ハイハット",
        Pad::HiHatPedal => "ハイハットのペダル",
        Pad::Crash1 => "クラッシュ 1",
        Pad::Crash2 => "クラッシュ 2",
        Pad::Ride => "ライド",
        Pad::Aux => "AUX",
    }
}

/// 叩き方の名前（演奏中の情報の窓に出す）
fn articulation_label(a: Articulation) -> &'static str {
    match a {
        Articulation::Head => "ヘッド",
        Articulation::Rim => "リム",
        Articulation::CrossStick => "クロススティック",
        Articulation::Bow => "ボウ",
        Articulation::Edge => "エッジ",
        Articulation::Bell => "ベル",
    }
}

/// 無いときは「-」と出す
fn opt<T: std::fmt::Debug>(v: Option<T>) -> String {
    v.map_or_else(|| "-".to_owned(), |v| format!("{v:?}"))
}

/// 苦手な小節の帯の幅の上限
const WEAK_BAND_MAX_WIDTH: f32 = 720.0;

/// 選んだ区間の枠の色
const SELECTION_COLOR: Color32 = Color32::from_rgb(0x40, 0xC8, 0xFF);

/// 小節の帯。曲の小節を左から並べ、苦手なほど赤く塗る。苦手フレーズは白い枠、選んだ区間は青い枠で囲む。
/// `select` なら、ドラッグかクリックで選んだ区間を返す
fn measure_band(
    ui: &mut egui::Ui,
    weakness: &[Option<f64>],
    phrases: &[WeakPhrase],
    selected: Option<Section>,
    size: Vec2,
    select: bool,
) -> Option<Section> {
    let n = weakness.len();
    if n == 0 {
        return None;
    }
    let numbers_h = if size.y >= 20.0 { 18.0 } else { 0.0 };
    let sense = if select {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(size.x, size.y + numbers_h), sense);
    let band = Rect::from_min_size(rect.min, size);
    let cell = size.x / n as f32;
    let gap = if cell >= 4.0 { 1.0 } else { 0.0 };
    let x_of = |m: usize| band.left() + cell * m as f32;
    let measure_at = |x: f32| (((x - band.left()) / cell).floor().max(0.0) as usize).min(n - 1);
    let painter = ui.painter();
    for (m, v) in weakness.iter().enumerate() {
        let r = Rect::from_min_max(
            Pos2::new(x_of(m), band.top()),
            Pos2::new(x_of(m + 1) - gap, band.bottom()),
        );
        painter.rect_filled(r, 0.0, weakness_color(*v));
    }
    // 苦手フレーズを枠で囲む
    for p in phrases {
        let r = Rect::from_min_max(
            Pos2::new(x_of(p.first as usize), band.top()),
            Pos2::new(x_of(p.last as usize + 1) - gap, band.bottom()),
        );
        painter.rect_stroke(
            r,
            0.0,
            Stroke::new(2.0, Color32::WHITE),
            StrokeKind::Outside,
        );
    }
    if let Some(s) = selected {
        let r = Rect::from_min_max(
            Pos2::new(x_of(s.first as usize), band.top() - 3.0),
            Pos2::new(x_of(s.last as usize + 1) - gap, band.bottom() + 3.0),
        );
        painter.rect_filled(r, 2.0, SELECTION_COLOR.gamma_multiply(0.25));
        painter.rect_stroke(
            r,
            2.0,
            Stroke::new(2.5, SELECTION_COLOR),
            StrokeKind::Outside,
        );
    }
    if numbers_h > 0.0 {
        // 小節の番号（DTX の小節番号）。文字が重ならない間隔で
        let step = [1, 2, 4, 8, 16, 32, 64, 128]
            .into_iter()
            .find(|&s| cell * s as f32 >= 36.0)
            .unwrap_or(256);
        for m in (0..n).step_by(step) {
            painter.text(
                Pos2::new(x_of(m), band.bottom() + 5.0),
                Align2::LEFT_TOP,
                m.to_string(),
                FontId::proportional(11.0),
                Color32::GRAY,
            );
        }
    }
    let mut picked = None;
    if select
        && (response.dragged() || response.clicked())
        && let (Some(from), Some(to)) = (
            ui.input(|i| i.pointer.press_origin()),
            response.interact_pointer_pos(),
        )
    {
        picked = Some(Section::new(
            measure_at(from.x) as u32,
            measure_at(to.x) as u32,
        ));
    }
    if let Some(pos) = response.hover_pos() {
        let m = measure_at(pos.x);
        let text = match weakness[m] {
            Some(v) => format!("小節 {m}: 苦手度 {v:.2}"),
            None => format!("小節 {m}: 記録なし"),
        };
        response.on_hover_text_at_pointer(text);
    }
    picked
}

/// 苦手な小節の帯と、苦手フレーズの一覧。フレーズを押したらその区間を返す（練習へ）
fn draw_weak_map(ui: &mut egui::Ui, w: &WeakMap) -> Option<Section> {
    if w.weakness.is_empty() {
        return None;
    }
    ui.label(RichText::new(format!("苦手な小節（{}）", weak_source(w))).size(18.0));
    let width = ui.available_width().min(WEAK_BAND_MAX_WIDTH);
    measure_band(
        ui,
        &w.weakness,
        &w.phrases,
        None,
        Vec2::new(width, 28.0),
        false,
    );
    if w.phrases.is_empty() {
        ui.label("苦手な小節は見つかりませんでした。");
        return None;
    }
    let mut picked = None;
    for p in &w.phrases {
        if ui
            .button(format!("{}　→ 練習する", phrase_text(p)))
            .clicked()
        {
            picked = Some(Section::new(p.first, p.last));
        }
    }
    picked
}

/// 苦手度の色。0 は緑、0.25 で黄、0.6 以上は赤。記録の無い小節は暗い灰色
pub fn weakness_color(v: Option<f64>) -> Color32 {
    const GREEN: [f32; 3] = [70.0, 160.0, 100.0];
    const YELLOW: [f32; 3] = [230.0, 200.0, 70.0];
    const RED: [f32; 3] = [235.0, 75.0, 65.0];
    let Some(v) = v else {
        return Color32::from_gray(48);
    };
    let v = v.clamp(0.0, 1.0) as f32;
    let (a, b, t) = if v < 0.25 {
        (GREEN, YELLOW, v / 0.25)
    } else {
        (YELLOW, RED, ((v - 0.25) / 0.35).min(1.0))
    };
    let mix = |i: usize| (a[i] + (b[i] - a[i]) * t).round() as u8;
    Color32::from_rgb(mix(0), mix(1), mix(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weakness_colors() {
        assert_eq!(weakness_color(None), Color32::from_gray(48));
        assert_eq!(weakness_color(Some(0.0)), Color32::from_rgb(70, 160, 100));
        assert_eq!(weakness_color(Some(0.25)), Color32::from_rgb(230, 200, 70));
        assert_eq!(weakness_color(Some(0.6)), Color32::from_rgb(235, 75, 65));
        assert_eq!(weakness_color(Some(1.0)), Color32::from_rgb(235, 75, 65));
    }
}
