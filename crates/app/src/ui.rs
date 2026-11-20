//! 画面。egui で描く。選曲・練習の区間選び・プレイ（練習中も）・結果。
//! 画面の関数は状態を変えず、押されたものを返す（変えるのは main.rs）。

use std::path::PathBuf;

use dtx::DrumLane;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use engine::analysis::{NOTABLE_MEAN_MS, Subdivision, Tendency, WeakPhrase, notable_tendencies};
use engine::judge::Judgement;
use engine::practice::{
    FAILS_TO_DOWN, LoopStyle, PASSES_TO_UP, PassRule, Section, SpeedChange, SpeedPlan,
    speed_percent,
};

use crate::config::Config;
use crate::game::{Outcome, Play, PracticeOutcome, WeakMap};
use crate::gfx::Canvas;
use crate::history::{OvercomeCheck, TENDENCY_MIN_HITS, TENDENCY_SESSIONS};
use crate::practice::{LapSummary, Practice, PracticeSetup};
use crate::songs::SongList;
use crate::stretch::Pitch;

fn judgement_label(j: Judgement) -> (&'static str, Color32) {
    match j {
        Judgement::Perfect => ("PERFECT", Color32::from_rgb(0xFF, 0xE0, 0x60)),
        Judgement::Great => ("GREAT", Color32::from_rgb(0x70, 0xE0, 0x90)),
        Judgement::Good => ("GOOD", Color32::from_rgb(0x70, 0xB0, 0xF0)),
        Judgement::Poor => ("POOR", Color32::from_rgb(0xB0, 0xB0, 0xB0)),
        Judgement::Miss => ("MISS", Color32::from_rgb(0xF0, 0x60, 0x60)),
    }
}

/// 選曲の画面（と設定の窓）で押されたもの
#[derive(Debug, Clone, PartialEq)]
pub enum SelectAction {
    /// 選んでいる行に入る（BOX なら中へ、曲なら始める）
    Enter,
    Back,
    AddFolder,
    RemoveFolder(usize),
    Rescan,
    /// 選んでいる曲の区間を練習する
    Practice,
    /// DTXManiaNX のフォルダを選ぶ
    ChooseNx,
    /// 設定の窓を閉じる
    CloseSettings,
}

/// 設定の窓に出すもの
pub struct SettingsInfo<'a> {
    pub config: &'a Config,
    /// 使っているスキンのフォルダ
    pub skin: Option<&'a std::path::Path>,
    pub songs: &'a SongList,
    pub midi_ports: &'a [String],
    pub message: Option<&'a str>,
}

/// 設定の窓（F1）。NX の設定画面に置き換えるまでの仮のもの
pub fn draw_settings(ctx: &egui::Context, info: &SettingsInfo<'_>) -> Vec<SelectAction> {
    let mut actions = Vec::new();
    let config = info.config;
    egui::Window::new("設定（F1 で閉じる）")
        .collapsible(false)
        .resizable(false)
        .default_width(560.0)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            if let Some(m) = info.message {
                ui.colored_label(Color32::from_rgb(0xF0, 0x80, 0x60), m);
                ui.separator();
            }
            ui.label(RichText::new("DTXManiaNX のフォルダ").strong());
            ui.label("画面の画像と音、曲のフォルダ、パッドの割り当てをここから読みます。");
            ui.horizontal(|ui| {
                match &config.nx_dir {
                    Some(d) => ui.label(d.display().to_string()),
                    None => ui.colored_label(Color32::from_rgb(0xF0, 0xC0, 0x60), "未設定"),
                };
                if ui.button("選ぶ…").clicked() {
                    actions.push(SelectAction::ChooseNx);
                }
            });
            if let Some(skin) = info.skin {
                ui.label(RichText::new(format!("スキン: {}", skin.display())).weak());
            }
            ui.add_space(8.0);
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
                if let Some(p) = info.songs.scanning() {
                    ui.spinner();
                    ui.label(format!("探しています（{} フォルダ、{} 曲）", p.folders, p.songs));
                }
            });
            ui.label("曲のフォルダや NX のフォルダは、このウィンドウにドロップしても追加できます。");
            ui.separator();
            ports_label(ui, info.midi_ports);
            ui.label(format!(
                "入力の補正 {:+.0}ms / 表示の補正 {:+.0}ms / 速さ {:.2}",
                config.input_offset_ms, config.visual_offset_ms, config.scroll_px_per_ms
            ));
            ui.separator();
            ui.label("↑↓ で選ぶ、Enter で決定、P で練習、←→ で難易度、Backspace で BOX を出る、Esc で終わる");
            ui.label("ドラム: HT / LT で選ぶ、CY か RD で決定、LC で戻る、HH を 2 回で難易度");
            ui.add_space(4.0);
            if ui.button("閉じる").clicked() {
                actions.push(SelectAction::CloseSettings);
            }
        });
    actions
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
    result_page(ui, |ui| {
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
                ui.add_space(24.0);
            });
        });
    });
    actions
}

/// 結果の 2 ページ目の枠。NX の結果の背景の上に、半透明の黒い板を真ん中に置く
fn result_page(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
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
                    .fill(Color32::from_black_alpha(215))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(width - 32.0);
                        ui.set_max_height(height - 32.0);
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
                "{} の{}{}: 平均 {:.0}ms {}（ばらつき ±{:.0}ms、{} 打）",
                n.lane.name(),
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
                        ui.label(RichText::new(lane.name()).strong());
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

/// 練習の結果。周ごとの成績を並べる
fn draw_practice_result(ui: &mut egui::Ui, o: &Outcome, p: &PracticeOutcome) -> Vec<ResultAction> {
    let mut actions = Vec::new();
    result_page(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            ui.heading(RichText::new(&o.title).size(28.0));
            if !o.artist.is_empty() {
                ui.label(&o.artist);
            }
            ui.label(
                RichText::new(format!(
                    "練習 {}（{}・合格の基準 {}・{}）",
                    section_text(p.options.section),
                    style_name(p.options.style),
                    rule_name(p.options.rule),
                    plan_text(p.options.speed)
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
                let top = done
                    .iter()
                    .filter(|l| l.passed == Some(true))
                    .map(|l| speed_percent(l.speed))
                    .max();
                if let Some(top) = top {
                    ui.label(format!("合格した一番速い速度 {top}%"));
                }
            }
            if p.cleared
                && let SpeedPlan::Ladder { target, .. } = p.options.speed
            {
                ui.label(
                    RichText::new(format!(
                        "クリア！ 目標の {target}% で {PASSES_TO_UP} 周続けて合格しました"
                    ))
                    .size(20.0)
                    .color(PASS_COLOR),
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
const LAP_TABLE_WIDTH: f32 = 640.0;

/// 合格・クリアの色
const PASS_COLOR: Color32 = Color32::from_rgb(0x70, 0xE0, 0x90);

/// 周ごとの成績の表
fn lap_table(ui: &mut egui::Ui, laps: &[&LapSummary]) {
    egui::Grid::new("laps")
        .striped(true)
        .spacing(Vec2::new(18.0, 4.0))
        .show(ui, |ui| {
            for h in [
                "周",
                "速度",
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
                ui.label(format!("{}%", speed_percent(l.speed)));
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
    /// 段階アップと固定を切り替える
    ToggleLadder,
    /// 速度（段階アップなら始めの速度）を刻み単位で動かす
    Speed(i64),
    /// 段階アップの目標の速度
    Target(u32),
    Pitch(Pitch),
    Start,
    Back,
}

/// 区間を選ぶ帯の幅の上限
const SETUP_BAND_MAX_WIDTH: f32 = 1000.0;

pub fn draw_setup(ui: &mut egui::Ui, s: &PracticeSetup) -> Vec<SetupAction> {
    let mut actions = Vec::new();
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
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
        let cleared = s.cleared_sections();
        if !cleared.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("練習でクリアした区間").strong());
            for (sec, speed, overcome) in cleared {
                let state = if overcome {
                    "克服"
                } else {
                    "克服はまだ（普通のプレイで確かめます）"
                };
                if ui
                    .selectable_label(
                        s.section == Some(sec),
                        format!("{}　{speed}% でクリア・{state}", section_text(sec)),
                    )
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
        ui.add_space(8.0);
        ui.label(RichText::new("速度（L で段階アップと固定を切り替え、, と . で速度）").strong());
        let ladder = matches!(s.speed, SpeedPlan::Ladder { .. });
        for (on, text) in [
            (true, ladder_help()),
            (false, "固定: 同じ速度のまま回る".to_owned()),
        ] {
            if ui.radio(ladder == on, text).clicked() && ladder != on {
                actions.push(SetupAction::ToggleLadder);
            }
        }
        ui.horizontal(|ui| {
            if ui.button("−5%").clicked() {
                actions.push(SetupAction::Speed(-1));
            }
            let label = if ladder { "始めの速度" } else { "速度" };
            ui.label(
                RichText::new(format!("{label} {}%", s.speed.first_speed()))
                    .size(16.0)
                    .strong(),
            );
            if ui.button("+5%").clicked() {
                actions.push(SetupAction::Speed(1));
            }
            if let SpeedPlan::Ladder { target, .. } = s.speed {
                ui.add_space(24.0);
                ui.label("目標");
                for t in SpeedPlan::TARGETS {
                    if ui.radio(target == t, format!("{t}%")).clicked() {
                        actions.push(SetupAction::Target(t));
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("遅くするときの音");
            for pitch in Pitch::ALL {
                if ui.radio(s.pitch == pitch, pitch_name(pitch)).clicked() {
                    actions.push(SetupAction::Pitch(pitch));
                }
            }
        });
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
                "←→ で区間をずらす、↑↓ で長さ、T でつなぎ方。ドラム: HT / LT でずらす、CY か RD ではじめる、LC で戻る",
            )
            .weak(),
        );
        });
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

fn ladder_help() -> String {
    format!(
        "段階アップ: 合格が {PASSES_TO_UP} 周続くたびに 5% 上げ、不合格が {FAILS_TO_DOWN} 周続いたら 5% 下げる（始めの速度より下げない）。目標の速度で {PASSES_TO_UP} 周続けて合格したらクリア"
    )
}

/// 速度の決め方を短く
fn plan_text(p: SpeedPlan) -> String {
    match p {
        SpeedPlan::Fixed(s) => format!("速度 {s}%"),
        SpeedPlan::Ladder { start, target } => format!("段階アップ {start}%→{target}%"),
    }
}

fn pitch_name(p: Pitch) -> &'static str {
    match p {
        Pitch::Keep => "音程を保つ",
        Pitch::Tape => "音程ごと遅く（テープ）",
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
        Some(true) => RichText::new("合格").color(PASS_COLOR),
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

fn ports_label(ui: &mut egui::Ui, midi_ports: &[String]) {
    if midi_ports.is_empty() {
        ui.label("MIDI 入力: 見つかりません（キーボードで叩けます: A S Z D G F H J K L、Q で HH オープン）");
    } else {
        ui.label(format!("MIDI 入力: {}", midi_ports.join(", ")));
    }
}

/// 演奏中に NX の画面へ重ねるもの
pub struct PlayOverlay<'a> {
    pub play: &'a Play,
    pub config: &'a Config,
    pub midi_ports: &'a [String],
    /// 設定と入力の情報を出す（I で切り替え）
    pub show_info: bool,
    /// 表示の補正を足した曲の時刻
    pub now: f64,
}

/// 演奏中の重ね書き。練習の欄（左上、スコアの場所）、助走のカウント、情報の窓（左、I で切り替え）
pub fn draw_play_overlay(ctx: &egui::Context, c: &Canvas, o: &PlayOverlay<'_>) {
    let s = c.scale();
    if let Some(p) = &o.play.practice {
        nx_panel(ctx, "practice", c.pos(6.0, 6.0), 282.0 * s, |ui| {
            practice_panel(ui, p, o.now, s);
        });
        if let Some(text) = count_in(p, o.now) {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("count-in"),
            ));
            let at = c.pos(574.0, 300.0);
            let font = FontId::proportional(72.0 * s);
            painter.text(
                at + Vec2::splat(3.0 * s),
                Align2::CENTER_CENTER,
                &text,
                font.clone(),
                Color32::from_black_alpha(200),
            );
            painter.text(at, Align2::CENTER_CENTER, &text, font, Color32::WHITE);
        }
    }
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
        "速さ {:.2}（↑ / ↓）",
        o.config.scroll_px_per_ms
    )));
    ui.label(small(format!(
        "譜面の音 {}（M）",
        if play.drum_sounds {
            "鳴らす"
        } else {
            "鳴らさない"
        }
    )));
    ui.label(small(if play.practice.is_some() {
        "←→ 区間を 1 小節ずらす、, . 速度 ±5%、R 最初から、Esc 結果へ".to_owned()
    } else {
        "R やり直し、Esc 結果へ".to_owned()
    }));
    ui.separator();
    match &play.last_input {
        Some(i) => {
            ui.label(small(format!(
                "最後の入力 {} / {} / {}",
                i.source,
                opt(i.pad),
                opt(i.articulation)
            )));
            ui.label(small(format!(
                "レーン {} / ベロシティ {} / HH ペダル {}",
                i.lane.map_or("-", |l| l.name()),
                i.velocity,
                opt(i.hh_pedal)
            )));
        }
        None => {
            ui.label(small("最後の入力 -".to_owned()));
        }
    }
    ui.label(small(format!(
        "今の HH ペダル {}（0 で開、127 で閉）",
        opt(play.hh_pedal)
    )));
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

/// 練習の欄（左上）。区間、何周目と速度、段階アップの進み、直近の周の成績。`s` は画面の拡大率
fn practice_panel(ui: &mut egui::Ui, p: &Practice, now: f64, s: f32) {
    let o = p.options;
    let text = |t: String, size: f32| RichText::new(t).size(size * s);
    ui.label(text(format!("練習 {}", section_text(o.section)), 14.0));
    ui.label(text(
        format!("{}・合格の基準 {}", style_name(o.style), rule_name(o.rule)),
        11.0,
    ));
    let width = ui.available_width();
    // 区間の枠は帯の上下に 3 ポイントはみ出すので、そのぶん空ける
    ui.add_space(4.0);
    measure_band(
        ui,
        &p.band,
        &[],
        Some(o.section),
        Vec2::new(width, 10.0 * s),
        false,
    );
    ui.add_space(4.0);
    let current = p.current_lap(now);
    let lap = match current {
        Some(i) => format!("{} 周目", i + 1),
        None => "助走".to_owned(),
    };
    // 今鳴っている周の速度（助走中は 1 周目の速度）
    let speed = current
        .or((!p.laps.is_empty()).then_some(0))
        .map_or(p.ladder().speed(), |i| speed_percent(p.laps[i].plan.speed));
    ui.label(
        text(format!("{lap}　速度 {speed}%"), 18.0)
            .strong()
            .color(Color32::WHITE),
    );
    let ladder = p.ladder();
    let (passes, fails) = ladder.streaks();
    match ladder.plan() {
        SpeedPlan::Ladder { target, .. } => {
            ui.label(text(
                format!(
                    "段階アップ（目標 {target}%）合格 {passes}/{PASSES_TO_UP}・不合格 {fails}/{FAILS_TO_DOWN}"
                ),
                11.0,
            ));
        }
        SpeedPlan::Fixed(_) => {
            ui.label(text(
                format!("速度は固定　連続合格 {}", p.pass_streak()),
                11.0,
            ));
        }
    }
    // 決めた速度がまだ並べていない周からのとき
    if p.last_speed().is_some_and(|sp| sp != ladder.speed()) {
        ui.label(
            text(
                format!("{}% は {} 周目から", ladder.speed(), p.laps.len() + 1),
                11.0,
            )
            .color(SELECTION_COLOR),
        );
    }
    if ladder.cleared() {
        ui.label(text("クリア！".to_owned(), 18.0).strong().color(PASS_COLOR));
    }
    if let Some(c) = p.speed_note(now) {
        let t = match c.change {
            SpeedChange::Up => format!("合格が続いたので {}% に上げます", c.speed),
            SpeedChange::Down => format!("不合格が続いたので {}% に下げます", c.speed),
            SpeedChange::Cleared => format!("{}% で合格が続きました", c.speed),
            SpeedChange::Stay => String::new(),
        };
        if !t.is_empty() {
            ui.label(text(t, 11.0).color(SELECTION_COLOR));
        }
    }
    let done: Vec<LapSummary> = p
        .summaries(now)
        .into_iter()
        .filter(|l| l.completed)
        .collect();
    let passed = done.iter().filter(|l| l.passed == Some(true)).count();
    ui.label(text(format!("合格 {passed} / {} 周", done.len()), 11.0));
    for l in done.iter().rev().take(3) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{:>3} {:>3}% P{:>3.0}% M{} P{} {}",
                    l.index + 1,
                    speed_percent(l.speed),
                    l.perfect_rate() * 100.0,
                    l.tally.miss,
                    l.tally.poor,
                    offset_text(l)
                ))
                .monospace()
                .size(10.0 * s),
            );
            ui.label(verdict(l).size(10.0 * s));
        });
    }
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
