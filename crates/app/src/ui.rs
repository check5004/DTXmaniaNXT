//! 画面。フェーズ 1 は egui の描画だけで、レーンと落ちてくる音符、判定、入力の確認を出す。

use dtx::DrumLane;
use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2};
use engine::judge::Judgement;

use crate::game::Play;

/// 画面の設定
pub struct View {
    /// 1 ミリ秒あたりに落ちるピクセル
    pub scroll_px_per_ms: f32,
    /// 表示の遅れの補正（ミリ秒）。画面が遅れて見えるなら増やす
    pub visual_offset_ms: f64,
}

impl Default for View {
    fn default() -> Self {
        Self {
            scroll_px_per_ms: 0.6,
            visual_offset_ms: 0.0,
        }
    }
}

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

pub fn draw_idle(ui: &mut egui::Ui, midi_ports: &[String], message: Option<&str>) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.heading("DTXmaniaNXT");
            ui.add_space(16.0);
            ui.label("DTX ファイルをこのウィンドウにドロップすると始まります。");
            ui.label("起動するときの引数に DTX ファイルを渡しても始まります。");
            ui.add_space(16.0);
            if let Some(m) = message {
                ui.colored_label(Color32::from_rgb(0xF0, 0x80, 0x60), m);
            }
            ui.add_space(16.0);
            ports_label(ui, midi_ports);
        });
    });
}

fn ports_label(ui: &mut egui::Ui, midi_ports: &[String]) {
    if midi_ports.is_empty() {
        ui.label("MIDI 入力: 見つかりません（キーボードで叩けます: A S Z D G F H J K L、Q で HH オープン）");
    } else {
        ui.label(format!("MIDI 入力: {}", midi_ports.join(", ")));
    }
}

pub fn draw_play(ui: &mut egui::Ui, play: &Play, view: &View, midi_ports: &[String]) {
    let now = play.now_ms() + view.visual_offset_ms;
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
            view.visual_offset_ms
        ));
        ui.label(format!("速さ {:.2}（↑ / ↓ で変更）", view.scroll_px_per_ms));
        ui.label(format!(
            "譜面の音 {}（M で切り替え）",
            if play.drum_sounds {
                "鳴らす"
            } else {
                "鳴らさない"
            }
        ));
        ui.label("R でやり直し、Esc で終わる");
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
                let y = judge_y - ((n.ms - now) as f32) * view.scroll_px_per_ms;
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
                    "おわり（R でもう一度、Esc で戻る）",
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
