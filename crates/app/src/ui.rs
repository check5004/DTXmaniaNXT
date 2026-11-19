//! 画面。egui で描く。選曲・練習の区間選び・プレイ（練習中も）・結果。
//! 画面の関数は状態を変えず、押されたものを返す（変えるのは main.rs）。

use std::path::PathBuf;

use dtx::DrumLane;
use egui::{Align, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use engine::analysis::WeakPhrase;
use engine::judge::Judgement;
use engine::practice::{LoopStyle, PassRule, Section};
use library::{LEVELS, Node};

use crate::config::Config;
use crate::game::{Outcome, Play, PracticeOutcome, WeakMap};
use crate::practice::{LapSummary, Practice, PracticeSetup};
use crate::songs::SongList;

/// 左から並べるレーン（HHO は HH、LBD は LP の列に描く）
const COLUMNS: [DrumLane; 10] = [
    DrumLane::LC,
    DrumLane::HH,
    DrumLane::LP,
    DrumLane::SD,
    DrumLane::HT,
    DrumLane::BD,
    DrumLane::LT,
    DrumLane::FT,
    DrumLane::CY,
    DrumLane::RD,
];

fn column_of(lane: DrumLane) -> usize {
    let shown = match lane {
        DrumLane::HHO => DrumLane::HH,
        DrumLane::LBD => DrumLane::LP,
        l => l,
    };
    COLUMNS.iter().position(|&l| l == shown).unwrap_or(0)
}

fn lane_color(lane: DrumLane) -> Color32 {
    match lane {
        DrumLane::LC | DrumLane::CY => Color32::from_rgb(0xF2, 0x5C, 0x9A),
        DrumLane::HH => Color32::from_rgb(0x3F, 0xB8, 0xF0),
        DrumLane::HHO => Color32::from_rgb(0x9A, 0xE0, 0xFF),
        DrumLane::LP | DrumLane::LBD => Color32::from_rgb(0xF0, 0xA0, 0x40),
        DrumLane::SD => Color32::from_rgb(0xF5, 0xD2, 0x3C),
        DrumLane::HT => Color32::from_rgb(0x5C, 0xE0, 0x6C),
        DrumLane::BD => Color32::from_rgb(0xB0, 0x7C, 0xF5),
        DrumLane::LT => Color32::from_rgb(0xF0, 0x52, 0x52),
        DrumLane::FT => Color32::from_rgb(0xF5, 0x8C, 0x3C),
        DrumLane::RD => Color32::from_rgb(0x48, 0xD8, 0xC8),
    }
}

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
    /// 一覧の行を選んだ
    Select(usize),
    /// 選んでいる行に入る（BOX なら中へ、曲なら始める）
    Enter,
    Back,
    Level(usize),
    AddFolder,
    RemoveFolder(usize),
    Rescan,
    /// 選んでいる曲の区間を練習する
    Practice,
}

pub fn draw_select(
    ui: &mut egui::Ui,
    songs: &SongList,
    config: &Config,
    midi_ports: &[String],
    message: Option<&str>,
    scroll_to_cursor: bool,
) -> Vec<SelectAction> {
    let mut actions = Vec::new();

    egui::Panel::top("select-top").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading("DTXmaniaNXT");
            ui.separator();
            let mut crumbs = vec!["曲"];
            crumbs.extend(songs.breadcrumbs());
            ui.label(crumbs.join(" ＞ "));
            if let Some(p) = songs.scanning() {
                ui.separator();
                ui.spinner();
                ui.label(format!(
                    "探しています（{} フォルダ、{} 曲）",
                    p.folders, p.songs
                ));
            }
        });
        if let Some(m) = message {
            ui.colored_label(Color32::from_rgb(0xF0, 0x80, 0x60), m);
        }
    });

    egui::Panel::right("select-info")
        .min_size(320.0)
        .show(ui, |ui| {
            selected_info(ui, songs, &mut actions);
            ui.add_space(12.0);
            ui.separator();
            ui.label(RichText::new("曲のフォルダ").strong());
            for (i, dir) in config.song_dirs.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("×").on_hover_text("一覧から外す").clicked() {
                        actions.push(SelectAction::RemoveFolder(i));
                    }
                    ui.label(dir.display().to_string());
                });
            }
            ui.horizontal(|ui| {
                if ui.button("フォルダを追加…").clicked() {
                    actions.push(SelectAction::AddFolder);
                }
                if ui
                    .add_enabled(!config.song_dirs.is_empty(), egui::Button::new("探し直す"))
                    .on_hover_text("F5")
                    .clicked()
                {
                    actions.push(SelectAction::Rescan);
                }
            });
            ui.label("フォルダをこのウィンドウにドロップしても追加できます。");
            ui.separator();
            ports_label(ui, midi_ports);
            ui.label(format!(
                "入力の補正 {:+.0}ms / 表示の補正 {:+.0}ms / 速さ {:.2}",
                config.input_offset_ms, config.visual_offset_ms, config.scroll_px_per_ms
            ));
            ui.label("↑↓ で選ぶ、Enter で決定、P で練習、←→ で難易度、Backspace で BOX を出る、Esc で終わる");
            ui.label("ドラム: HT / LT で選ぶ、CY か RD で決定、LC で戻る、HH を 2 回で難易度");
        });

    egui::CentralPanel::default().show(ui, |ui| {
        if config.song_dirs.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.heading("曲のフォルダを追加してください");
                ui.add_space(8.0);
                ui.label("DTXMania の曲のフォルダ（DTXFiles など）を選ぶと、中の曲を探して一覧にします。");
                ui.label("DTX ファイルをドロップすると、その曲だけをすぐ遊べます。");
                ui.add_space(16.0);
                if ui.button("フォルダを追加…").clicked() {
                    actions.push(SelectAction::AddFolder);
                }
            });
            return;
        }
        let nodes = songs.nodes();
        if nodes.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                if songs.scanning().is_some() {
                    ui.label("探しています…");
                } else {
                    ui.label("曲が見つかりませんでした。");
                }
            });
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if !songs.breadcrumbs().is_empty()
                    && ui
                        .selectable_label(false, "← 戻る")
                        .on_hover_text("Backspace")
                        .clicked()
                {
                    actions.push(SelectAction::Back);
                }
                for (i, node) in nodes.iter().enumerate() {
                    let selected = i == songs.cursor;
                    let text = match node {
                        Node::Box(b) => RichText::new(format!("■ {}（{}）", b.title, b.nodes.len()))
                            .color(Color32::from_rgb(0xF0, 0xC8, 0x60)),
                        Node::Song(s) => {
                            let level = songs
                                .level_for(s)
                                .and_then(|l| s.charts[l].as_ref())
                                .and_then(|c| c.head.level)
                                .map_or(String::new(), |lv| format!("  Lv {lv:.2}"));
                            let artist = if s.artist.is_empty() {
                                String::new()
                            } else {
                                format!(" ／ {}", s.artist)
                            };
                            RichText::new(format!("{}{artist}{level}", s.title))
                        }
                    };
                    let r = ui.selectable_label(selected, text.size(16.0));
                    if selected && scroll_to_cursor {
                        r.scroll_to_me(Some(Align::Center));
                    }
                    if r.double_clicked() {
                        actions.push(SelectAction::Select(i));
                        actions.push(SelectAction::Enter);
                    } else if r.clicked() {
                        actions.push(SelectAction::Select(i));
                    }
                }
            });
    });
    actions
}

fn selected_info(ui: &mut egui::Ui, songs: &SongList, actions: &mut Vec<SelectAction>) {
    match songs.selected() {
        None => {
            ui.label("-");
        }
        Some(Node::Box(b)) => {
            ui.heading(&b.title);
            if !b.genre.is_empty() {
                ui.label(&b.genre);
            }
            ui.label(format!("{} 件", b.nodes.len()));
            ui.add_space(8.0);
            if ui.button("開く（Enter）").clicked() {
                actions.push(SelectAction::Enter);
            }
        }
        Some(Node::Song(s)) => {
            ui.heading(&s.title);
            if !s.artist.is_empty() {
                ui.label(&s.artist);
            }
            let top = s.charts.iter().rev().flatten().next();
            let bpm = top.and_then(|c| c.head.bpm);
            let mut line = Vec::new();
            if !s.genre.is_empty() {
                line.push(s.genre.clone());
            }
            if let Some(b) = bpm {
                line.push(format!("BPM {b:.0}"));
            }
            if !line.is_empty() {
                ui.label(line.join(" / "));
            }
            if let Some(c) = top.filter(|c| !c.head.comment.is_empty()) {
                ui.label(RichText::new(&c.head.comment).weak());
            }
            ui.add_space(8.0);
            let current = songs.level_for(s);
            for i in (0..LEVELS).rev() {
                let Some(c) = &s.charts[i] else {
                    continue;
                };
                let label = if c.label.is_empty() {
                    "DRUMS".to_owned()
                } else {
                    c.label.clone()
                };
                let lv = c.head.level.map_or("-".to_owned(), |l| format!("{l:.2}"));
                if ui
                    .selectable_label(
                        current == Some(i),
                        RichText::new(format!("{label}  {lv}")).size(16.0),
                    )
                    .clicked()
                {
                    actions.push(SelectAction::Level(i));
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("はじめる（Enter）").clicked() {
                    actions.push(SelectAction::Enter);
                }
                if ui
                    .button("区間を練習（P）")
                    .on_hover_text("小節の範囲を選び、そこだけを繰り返します")
                    .clicked()
                {
                    actions.push(SelectAction::Practice);
                }
            });
        }
    }
}

/// 結果の画面で押されたもの
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResultAction {
    Back,
    /// もう一度（練習なら同じ区間を）
    Retry,
    SetInputOffset(f64),
    /// 区間を選ぶ画面へ。`Some` ならその区間を選んでおく
    Practice(Option<Section>),
}

/// 補正に反映してよいだけの数（これより少ないと平均がぶれる）
const MIN_HITS_FOR_OFFSET: u32 = 20;

pub fn draw_result(ui: &mut egui::Ui, o: &Outcome, config: &Config) -> Vec<ResultAction> {
    if let Some(p) = &o.practice {
        return draw_practice_result(ui, o, p);
    }
    let mut actions = Vec::new();
    egui::CentralPanel::default().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
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
            ui.label(RichText::new(format!("最大コンボ {} / {}", t.max_combo, o.notes)).size(18.0));
            ui.label(format!("空打ち {}", t.stray));
            ui.add_space(16.0);
            let hits = t.perfect + t.great + t.good + t.poor;
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
                if hits >= MIN_HITS_FOR_OFFSET && mean.abs() >= 3.0 {
                    let next = (config.input_offset_ms + mean).round();
                    ui.label(
                        RichText::new("いつも同じ向きにずれるなら、機材の遅れかもしれません。")
                            .weak(),
                    );
                    if ui
                        .button(format!(
                            "入力の補正を {:+.0}ms → {next:+.0}ms にする",
                            config.input_offset_ms
                        ))
                        .clicked()
                    {
                        actions.push(ResultAction::SetInputOffset(next));
                    }
                }
            }
            if let Some(w) = &o.weak {
                ui.add_space(24.0);
                if let Some(section) = draw_weak_map(ui, w) {
                    actions.push(ResultAction::Practice(Some(section)));
                }
            }
            ui.add_space(24.0);
            if ui
                .button("選曲に戻る（Enter、ドラムは CY か RD）")
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
        });
    });
    actions
}

/// 練習の結果。周ごとの成績を並べる
fn draw_practice_result(ui: &mut egui::Ui, o: &Outcome, p: &PracticeOutcome) -> Vec<ResultAction> {
    let mut actions = Vec::new();
    egui::CentralPanel::default().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(32.0);
            ui.heading(RichText::new(&o.title).size(28.0));
            if !o.artist.is_empty() {
                ui.label(&o.artist);
            }
            ui.label(
                RichText::new(format!(
                    "練習 {}（{}・合格の基準 {}）",
                    section_text(p.options.section),
                    style_name(p.options.style),
                    rule_name(p.options.rule)
                ))
                .size(18.0),
            );
            ui.add_space(16.0);
            let done: Vec<&LapSummary> = p.laps.iter().filter(|l| l.completed).collect();
            if done.is_empty() {
                ui.label("最後まで叩いた周はありませんでした。");
            } else {
                let passed = done.iter().filter(|l| l.passed == Some(true)).count();
                let rate = done.iter().map(|l| l.perfect_rate()).sum::<f64>() / done.len() as f64;
                ui.label(
                    RichText::new(format!(
                        "{} 周のうち合格 {} 周、PERFECT の割合の平均 {:.0}%",
                        done.len(),
                        passed,
                        rate * 100.0
                    ))
                    .size(18.0),
                );
            }
            if p.laps.len() > done.len() {
                ui.label(RichText::new("途中でやめた周は数えず、記録にも残しません。").weak());
            }
            if !done.is_empty() {
                ui.add_space(12.0);
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height() - 140.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(((ui.available_width() - LAP_TABLE_WIDTH) / 2.0).max(0.0));
                            lap_table(ui, &done);
                        });
                    });
            }
            ui.add_space(16.0);
            if ui.button("もう一度（R）").clicked() {
                actions.push(ResultAction::Retry);
            }
            if ui.button("区間を選び直す（P）").clicked() {
                actions.push(ResultAction::Practice(None));
            }
            if ui
                .button("選曲に戻る（Enter、ドラムは CY か RD）")
                .clicked()
            {
                actions.push(ResultAction::Back);
            }
        });
    });
    actions
}

/// 周ごとの成績の表のおおよその幅（真ん中に寄せるのに使う）
const LAP_TABLE_WIDTH: f32 = 580.0;

/// 周ごとの成績の表
fn lap_table(ui: &mut egui::Ui, laps: &[&LapSummary]) {
    egui::Grid::new("laps")
        .striped(true)
        .spacing(Vec2::new(18.0, 4.0))
        .show(ui, |ui| {
            for h in [
                "周",
                "PERFECT",
                "GREAT",
                "GOOD",
                "POOR",
                "MISS",
                "空打ち",
                "ずれ",
                "",
            ] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for l in laps {
                let t = &l.tally;
                ui.label(format!("{}", l.index + 1));
                ui.label(format!("{} ({:.0}%)", t.perfect, l.perfect_rate() * 100.0));
                ui.label(t.great.to_string());
                ui.label(t.good.to_string());
                ui.label(t.poor.to_string());
                ui.label(t.miss.to_string());
                ui.label(t.stray.to_string());
                ui.label(offset_text(l));
                ui.label(verdict(l));
                ui.end_row();
            }
        });
}

/// 区間を選ぶ画面で押されたもの
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SetupAction {
    Select(Section),
    /// 区間を小節単位でずらす
    Shift(i64),
    /// 区間の終わりを小節単位でのばす（縮める）
    Resize(i64),
    Style(LoopStyle),
    Rule(PassRule),
    Start,
    Back,
}

/// 区間を選ぶ帯の幅の上限
const SETUP_BAND_MAX_WIDTH: f32 = 1000.0;

pub fn draw_setup(ui: &mut egui::Ui, s: &PracticeSetup) -> Vec<SetupAction> {
    let mut actions = Vec::new();
    egui::CentralPanel::default().show(ui, |ui| {
        ui.add_space(16.0);
        ui.heading(RichText::new(format!("練習: {}", s.title)).size(26.0));
        if !s.artist.is_empty() {
            ui.label(&s.artist);
        }
        ui.add_space(16.0);
        ui.label(RichText::new("練習する区間").size(18.0).strong());
        ui.label(format!(
            "帯をドラッグして選びます（クリックで 1 小節）。色は苦手度で、{}。",
            weak_source(&s.weak)
        ));
        let width = ui.available_width().min(SETUP_BAND_MAX_WIDTH);
        if let Some(sec) = measure_band(
            ui,
            &s.weak.weakness,
            &s.weak.phrases,
            s.section,
            Vec2::new(width, 44.0),
            true,
        ) {
            actions.push(SetupAction::Select(sec));
        }
        match s.section {
            Some(sec) => {
                let n = s.notes_in(sec);
                ui.label(
                    RichText::new(format!(
                        "{}（{} 小節、音符 {}）",
                        section_text(sec),
                        sec.measures(),
                        n
                    ))
                    .size(18.0),
                );
                if n == 0 {
                    ui.colored_label(
                        Color32::from_rgb(0xF0, 0x80, 0x60),
                        "この区間には音符がありません。",
                    );
                }
            }
            None => {
                ui.label("区間を選んでください。");
            }
        }
        ui.horizontal(|ui| {
            for (text, a) in [
                ("◀ 1 小節前へ", SetupAction::Shift(-1)),
                ("1 小節後へ ▶", SetupAction::Shift(1)),
                ("短く", SetupAction::Resize(-1)),
                ("長く", SetupAction::Resize(1)),
            ] {
                if ui.add_enabled(s.section.is_some(), egui::Button::new(text)).clicked() {
                    actions.push(a);
                }
            }
        });
        if !s.weak.phrases.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("苦手フレーズから選ぶ").strong());
            for p in &s.weak.phrases {
                let sec = Section::new(p.first, p.last);
                if ui
                    .selectable_label(s.section == Some(sec), phrase_text(p))
                    .clicked()
                {
                    actions.push(SetupAction::Select(sec));
                }
            }
        }
        ui.add_space(12.0);
        ui.label(RichText::new("周のつなぎ方（T で切り替え）").strong());
        for style in LoopStyle::ALL {
            if ui
                .radio(s.style == style, style_help(style))
                .clicked()
            {
                actions.push(SetupAction::Style(style));
            }
        }
        ui.add_space(8.0);
        ui.label(RichText::new("周の合格の基準").strong());
        ui.horizontal(|ui| {
            for rule in PassRule::ALL {
                if ui
                    .radio(s.rule == rule, rule_name(rule))
                    .on_hover_text(rule_help(rule))
                    .clicked()
                {
                    actions.push(SetupAction::Rule(rule));
                }
            }
        });
        ui.label(RichText::new(rule_help(s.rule)).weak());
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    s.options().is_some(),
                    egui::Button::new(RichText::new("はじめる（Enter）").size(18.0)),
                )
                .clicked()
            {
                actions.push(SetupAction::Start);
            }
            if ui.button("選曲に戻る（Esc）").clicked() {
                actions.push(SetupAction::Back);
            }
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "←→ で区間をずらす、↑↓ で長さ。ドラム: HT / LT でずらす、CY か RD ではじめる、LC で戻る",
            )
            .weak(),
        );
    });
    actions
}

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

fn style_name(s: LoopStyle) -> &'static str {
    match s {
        LoopStyle::Connect => "つなぐ",
        LoopStyle::Restart => "区切る",
    }
}

fn style_help(s: LoopStyle) -> &'static str {
    match s {
        LoopStyle::Connect => {
            "つなぐ: 区間の終わりから切れ目なく頭へ戻る（ビートや繰り返すパターン向き）"
        }
        LoopStyle::Restart => {
            "区切る: 次の小節の頭の 1 打（着地）まで叩き、助走からやり直す（フィル向き）"
        }
    }
}

fn rule_name(r: PassRule) -> &'static str {
    match r {
        PassRule::Easy => "ゆるめ",
        PassRule::Normal => "ふつう",
        PassRule::Strict => "きびしめ",
    }
}

fn rule_help(r: PassRule) -> &'static str {
    match r {
        PassRule::Easy => "MISS と POOR が 0 なら合格",
        PassRule::Normal => "MISS と POOR が 0、PERFECT が 8 割以上なら合格",
        PassRule::Strict => {
            "MISS と POOR が 0、PERFECT が 9 割以上、空打ち 0、ずれのばらつき 15ms 以下なら合格"
        }
    }
}

/// 周のずれ（平均とばらつき）
fn offset_text(l: &LapSummary) -> String {
    match (l.offset_mean_ms, l.offset_sd_ms) {
        (Some(m), Some(sd)) => format!("{m:+.1}ms ±{sd:.1}"),
        (Some(m), None) => format!("{m:+.1}ms"),
        _ => "-".to_owned(),
    }
}

/// 周の合否
fn verdict(l: &LapSummary) -> RichText {
    match l.passed {
        Some(true) => RichText::new("合格").color(Color32::from_rgb(0x70, 0xE0, 0x90)),
        Some(false) => RichText::new("不合格").color(Color32::from_rgb(0xF0, 0x80, 0x60)),
        None => RichText::new("途中").weak(),
    }
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
pub fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("曲のフォルダを選ぶ")
        .pick_folder()
}

fn ports_label(ui: &mut egui::Ui, midi_ports: &[String]) {
    if midi_ports.is_empty() {
        ui.label("MIDI 入力: 見つかりません（キーボードで叩けます: A S Z D G F H J K L、Q で HH オープン）");
    } else {
        ui.label(format!("MIDI 入力: {}", midi_ports.join(", ")));
    }
}

pub fn draw_play(ui: &mut egui::Ui, play: &Play, config: &Config, midi_ports: &[String]) {
    let now = play.now_ms() + config.visual_offset_ms;
    egui::Panel::right("info").min_size(280.0).show(ui, |ui| {
        ui.heading(&play.title);
        ui.label(&play.artist);
        ui.separator();
        if let Some(p) = &play.practice {
            practice_panel(ui, p, now);
        } else {
            let t = play.judge.tally();
            ui.label(format!("PERFECT {}", t.perfect));
            ui.label(format!("GREAT {}", t.great));
            ui.label(format!("GOOD {}", t.good));
            ui.label(format!("POOR {}", t.poor));
            ui.label(format!("MISS {}", t.miss));
            ui.label(format!("空打ち {}", t.stray));
            ui.label(format!("最大コンボ {}", t.max_combo));
            ui.separator();
            ui.label(format!(
                "曲の時刻 {:.2}s / {:.0}s",
                now / 1000.0,
                play.end_ms / 1000.0
            ));
        }
        match &play.audio {
            Some(a) => ui.label(format!("音: {}", a.device_name)),
            None => ui.label("音: 出力なし（時計はシステム時刻）"),
        };
        ports_label(ui, midi_ports);
        ui.separator();
        ui.label(format!(
            "入力の補正 {:+.0}ms（- / = で変更）",
            play.input_offset_ms
        ));
        ui.label(format!(
            "表示の補正 {:+.0}ms（[ / ] で変更）",
            config.visual_offset_ms
        ));
        ui.label(format!(
            "速さ {:.2}（↑ / ↓ で変更）",
            config.scroll_px_per_ms
        ));
        ui.label(format!(
            "譜面の音 {}（M で切り替え）",
            if play.drum_sounds {
                "鳴らす"
            } else {
                "鳴らさない"
            }
        ));
        if play.practice.is_some() {
            ui.label("←→ で区間を 1 小節ずらす、R で最初から、Esc で終わって結果へ");
        } else {
            ui.label("R でやり直し、Esc で選曲に戻る");
        }
        ui.separator();
        ui.label("最後の入力");
        match &play.last_input {
            Some(i) => {
                ui.label(format!(
                    "{} / {} / {}",
                    i.source,
                    opt(i.pad),
                    opt(i.articulation)
                ));
                ui.label(format!(
                    "レーン {} / ベロシティ {}",
                    i.lane.map_or("-", |l| l.name()),
                    i.velocity
                ));
                ui.label(format!("叩いた瞬間の HH ペダル {}", opt(i.hh_pedal)));
            }
            None => {
                ui.label("-");
            }
        }
        ui.label(format!(
            "今の HH ペダル {}（0 で開、127 で閉）",
            opt(play.hh_pedal)
        ));
    });

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(Color32::from_rgb(8, 8, 14)))
        .show(ui, |ui| {
            let area = ui.max_rect();
            let painter = ui.painter_at(area);
            let col_w = (area.width() / COLUMNS.len() as f32).min(72.0);
            let left = area.center().x - col_w * COLUMNS.len() as f32 / 2.0;
            let judge_y = area.top() + area.height() * 0.85;

            for (i, lane) in COLUMNS.iter().enumerate() {
                let x = left + col_w * i as f32;
                let r =
                    Rect::from_min_size(Pos2::new(x, area.top()), Vec2::new(col_w, area.height()));
                painter.rect_filled(r, 0.0, Color32::from_rgb(16, 16, 26));
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(1.0, Color32::from_rgb(40, 40, 56)),
                    StrokeKind::Inside,
                );
                painter.text(
                    Pos2::new(x + col_w / 2.0, judge_y + 24.0),
                    Align2::CENTER_CENTER,
                    lane.name(),
                    FontId::proportional(14.0),
                    Color32::from_gray(160),
                );
            }
            painter.line_segment(
                [
                    Pos2::new(left, judge_y),
                    Pos2::new(left + col_w * COLUMNS.len() as f32, judge_y),
                ],
                Stroke::new(3.0, Color32::from_gray(220)),
            );

            let y_of = |ms: f64| judge_y - ((ms - now) as f32) * config.scroll_px_per_ms;
            if let Some(p) = &play.practice {
                // 周の頭に線を引く
                for l in &p.laps {
                    let y = y_of(l.plan.start_ms);
                    if y < area.top() || y > area.bottom() {
                        continue;
                    }
                    let x1 = left + col_w * COLUMNS.len() as f32;
                    painter.line_segment(
                        [Pos2::new(left, y), Pos2::new(x1, y)],
                        Stroke::new(1.5, Color32::from_rgb(0x40, 0x90, 0xC0)),
                    );
                    painter.text(
                        Pos2::new(x1 + 6.0, y),
                        Align2::LEFT_CENTER,
                        format!("{} 周目", l.plan.index + 1),
                        FontId::proportional(12.0),
                        Color32::from_rgb(0x70, 0xB0, 0xE0),
                    );
                }
                // 助走など、判定しない音符は薄く
                for &(ms, lane) in &p.faint {
                    let y = y_of(ms);
                    if y < area.top() - 20.0 || y > area.bottom() + 20.0 {
                        continue;
                    }
                    let x = left + col_w * column_of(lane) as f32;
                    let r = Rect::from_center_size(
                        Pos2::new(x + col_w / 2.0, y),
                        Vec2::new(col_w - 6.0, 10.0),
                    );
                    painter.rect_filled(r, 3.0, lane_color(lane).gamma_multiply(0.3));
                }
            }

            // 音符（過ぎた音符は飛ばして探す）
            let notes = play.judge.notes();
            let from = notes.partition_point(|n| n.ms < now - 1000.0);
            for (i, n) in notes.iter().enumerate().skip(from) {
                if play.judge.is_done(i) {
                    continue;
                }
                let y = judge_y - ((n.ms - now) as f32) * config.scroll_px_per_ms;
                if y < area.top() - 20.0 {
                    break;
                }
                if y > area.bottom() + 20.0 {
                    continue;
                }
                let x = left + col_w * column_of(n.lane) as f32;
                let r = Rect::from_center_size(
                    Pos2::new(x + col_w / 2.0, y),
                    Vec2::new(col_w - 6.0, 10.0),
                );
                painter.rect_filled(r, 3.0, lane_color(n.lane));
            }

            // 判定（0.4 秒だけ出す）
            for (lane, f) in &play.flashes {
                if now - f.song_ms > 400.0 {
                    continue;
                }
                let (text, color) = judgement_label(f.judgement);
                let x = left + col_w * (column_of(*lane) as f32 + 0.5);
                painter.text(
                    Pos2::new(x, judge_y - 40.0),
                    Align2::CENTER_CENTER,
                    text,
                    FontId::proportional(13.0),
                    color,
                );
                if let Some(o) = f.offset_ms {
                    painter.text(
                        Pos2::new(x, judge_y - 24.0),
                        Align2::CENTER_CENTER,
                        format!("{o:+.0}"),
                        FontId::monospace(11.0),
                        Color32::from_gray(200),
                    );
                }
            }

            let combo = play.judge.tally().combo;
            if combo >= 2 {
                painter.text(
                    Pos2::new(area.center().x, area.top() + area.height() * 0.35),
                    Align2::CENTER_CENTER,
                    format!("{combo} COMBO"),
                    FontId::proportional(32.0),
                    Color32::from_gray(230),
                );
            }
            if play.finished() {
                painter.text(
                    Pos2::new(area.center().x, area.top() + area.height() * 0.5),
                    Align2::CENTER_CENTER,
                    "おわり",
                    FontId::proportional(24.0),
                    Color32::from_gray(230),
                );
            }
            let countdown = match &play.practice {
                Some(p) => count_in(p, now),
                None => (now < 0.0).then(|| format!("{:.1}", -now / 1000.0)),
            };
            if let Some(text) = countdown {
                painter.text(
                    Pos2::new(area.center().x, area.top() + area.height() * 0.2),
                    Align2::CENTER_CENTER,
                    text,
                    FontId::proportional(28.0),
                    Color32::from_gray(200),
                );
            }
        });
}

/// 無いときは「-」と出す
fn opt<T: std::fmt::Debug>(v: Option<T>) -> String {
    v.map_or_else(|| "-".to_owned(), |v| format!("{v:?}"))
}

/// 助走のカウント（1〜4）。カウントの前は「助走」、周の中は None
fn count_in(p: &Practice, now: f64) -> Option<String> {
    let lap = p.laps.iter().find(|l| {
        !l.plan.clicks.is_empty() && now >= l.plan.audio.at_ms && now < l.plan.start_ms
    })?;
    let n = lap.plan.clicks.iter().filter(|&&c| c <= now).count();
    Some(if n == 0 {
        "助走".to_owned()
    } else {
        n.to_string()
    })
}

/// 練習中の右の欄。区間、何周目、連続合格、直近の周の成績
fn practice_panel(ui: &mut egui::Ui, p: &Practice, now: f64) {
    let o = p.options;
    ui.label(RichText::new(format!("練習 {}", section_text(o.section))).size(16.0));
    ui.label(format!(
        "{}・合格の基準 {}",
        style_name(o.style),
        rule_name(o.rule)
    ));
    let width = ui.available_width();
    measure_band(
        ui,
        &p.band,
        &[],
        Some(o.section),
        Vec2::new(width, 12.0),
        false,
    );
    let lap = match p.current_lap(now) {
        Some(i) => format!("{} 周目", i + 1),
        None => "助走".to_owned(),
    };
    ui.label(
        RichText::new(format!("{lap}　連続合格 {}", p.pass_streak()))
            .size(18.0)
            .strong(),
    );
    let done: Vec<LapSummary> = p
        .summaries(now)
        .into_iter()
        .filter(|l| l.completed)
        .collect();
    let passed = done.iter().filter(|l| l.passed == Some(true)).count();
    ui.label(format!("合格 {passed} / {} 周", done.len()));
    for l in done.iter().rev().take(6) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{:>3}  P {:>3.0}%  MISS {} POOR {}  {}",
                    l.index + 1,
                    l.perfect_rate() * 100.0,
                    l.tally.miss,
                    l.tally.poor,
                    offset_text(l)
                ))
                .monospace()
                .size(11.0),
            );
            ui.label(verdict(l));
        });
    }
    ui.separator();
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
fn weakness_color(v: Option<f64>) -> Color32 {
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
