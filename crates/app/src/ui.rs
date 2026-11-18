//! 画面。egui で描く。選曲・プレイ・結果の 3 つ。
//! 画面の関数は状態を変えず、押されたものを返す（変えるのは main.rs）。

use std::path::PathBuf;

use dtx::DrumLane;
use egui::{Align, Align2, Color32, FontId, Pos2, Rect, RichText, Stroke, StrokeKind, Vec2};
use engine::judge::Judgement;
use library::{LEVELS, Node};

use crate::config::Config;
use crate::game::{Outcome, Play};
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
            ui.label("↑↓ で選ぶ、Enter で決定、←→ で難易度、Backspace で BOX を出る、Esc で終わる");
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
            if ui.button("はじめる（Enter）").clicked() {
                actions.push(SelectAction::Enter);
            }
        }
    }
}

/// 結果の画面で押されたもの
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResultAction {
    Back,
    Retry,
    SetInputOffset(f64),
}

/// 補正に反映してよいだけの数（これより少ないと平均がぶれる）
const MIN_HITS_FOR_OFFSET: u32 = 20;

pub fn draw_result(ui: &mut egui::Ui, o: &Outcome, config: &Config) -> Vec<ResultAction> {
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
        });
    });
    actions
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
        ui.label("R でやり直し、Esc で選曲に戻る");
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

            // 音符
            for (i, n) in play.judge.notes().iter().enumerate() {
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
            if now < 0.0 {
                painter.text(
                    Pos2::new(area.center().x, area.top() + area.height() * 0.2),
                    Align2::CENTER_CENTER,
                    format!("{:.1}", -now / 1000.0),
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
