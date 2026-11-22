//! 演奏画面（ドラム）。上流 DTXManiaNX の `CStagePerfDrumsScreen` とその部品（`CActPerfDrumsLaneFlushD`、
//! `CActPerfDrumsPad`、`CActPerfDrumsChipFireD`、`CActPerfDrumsJudgementString`、`CActPerfCommonCombo`、
//! `CActPerfDrumsComboDGB`、`CActPerfDrumsGauge`、`CActPerfDrumsScore`、`CActPerfDrumsStatusPanel`、
//! `CActPerfPanelString`、`CActPerfProgressBar`）と同じ画像を同じ座標と順で描く。
//! 座標と動きの速さは上流のコードから写している。レーンの並び（LaneType・RDPosition）は `lane_layout`。
//! 判定の文字はコマ送り、スコアとゲージは XG のもの（上流の既定）だけ。
//!
//! 判定は `Play` が MIDI の時刻と音の再生位置で行う。この画面は `Play` から出来事（叩いた・判定した）を
//! 受け取り、光やコンボなどの動きを実時間で進めて描くだけ。

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use dtx::DrumLane;
use egui::Color32;
use engine::judge::{Judgement, Tally};

use crate::game::{Fx, GaugeRule, Play};
use crate::gfx::{Canvas, Gfx, Glyphs, Opt, Src, Tex, fit_scale, src};
use crate::lag_view::HitErrorBar;
use crate::lane_layout::{LANES, LANES_LEFT, Lane, Layout};
use crate::text::Style;

/// 判定の線の y（上流 `nJudgeLinePosY.Drums` の既定）
pub const JUDGE_LINE_Y: f32 = 561.0;
/// 下から上へ流すときの判定の線の y（上流の Reverse）
const REVERSE_JUDGE_LINE_Y: f32 = 159.0;
/// 進み具合のバーの左（上流 `CActPerfProgressBar`。左バスドラムのレーンを足したらそのぶん右へ）
pub const PROGRESS_X: f32 = 855.0;
/// 判定の線より上のどこまでチップを描くか（上流は判定の線から 600 ドットを超えたら打ち切る）
const DRAW_AHEAD_PX: f32 = 600.0;
/// 画面に入ってから暗いところから明るくなるまで
const FADE_IN: Duration = Duration::from_millis(500);

/// 判定の線より上に見えている時間（ミリ秒）。`px_per_ms` は落ちる速さ
pub fn view_ahead_ms(px_per_ms: f32) -> f64 {
    f64::from(DRAW_AHEAD_PX / px_per_ms.max(0.01))
}

/// レーンの枠（7_Paret.png）を描く順と、切り出す x・幅（上流 `CActPerfDrumsLaneFlushD` の「動くレーン」。
/// 置く x はレーンの並びで変わるので [`Layout`]）。足した左バスドラムのレーンは LP の枠の色を変えたもの
const PANEL: [(Lane, f32, f32); LANES] = [
    (Lane::LP, 121.0, 51.0),
    (Lane::LBD, 121.0, 51.0),
    (Lane::SD, 172.0, 57.0),
    (Lane::HT, 229.0, 49.0),
    (Lane::FT, 396.0, 54.0),
    (Lane::LT, 347.0, 49.0),
    (Lane::BD, 278.0, 69.0),
    (Lane::HH, 72.0, 49.0),
    (Lane::RD, 520.0, 38.0),
    (Lane::CY, 450.0, 70.0),
    (Lane::LC, 0.0, 72.0),
];

/// レーンの光の幅と画像（上流 `stレーンサイズ` と `strファイル名`、`ELane` の順。左バスドラムは LP の色を変えたもの）
const FLUSH: [(f32, &str); LANES] = [
    (64.0, "leftcymbal"),
    (46.0, "hihat"),
    (54.0, "snare"),
    (60.0, "bass"),
    (46.0, "hitom"),
    (46.0, "lowtom"),
    (46.0, "floortom"),
    (64.0, "cymbal"),
    (48.0, "leftpedal"),
    (38.0, "ridecymbal"),
    (48.0, "leftpedal"),
];

/// パッドの 7_pads.png の切り出しの左上（上流 `CActPerfDrumsPad.st基本位置`、`ELane` の順に並べ替えた）
const PADS: [(f32, f32); LANES] = [
    (0.0, 0.0),    // LC
    (96.0, 0.0),   // HH
    (0.0, 96.0),   // SD
    (0.0, 192.0),  // BD
    (96.0, 96.0),  // HT
    (192.0, 96.0), // LT
    (288.0, 96.0), // FT
    (192.0, 0.0),  // CY
    (96.0, 192.0), // LP
    (288.0, 0.0),  // RD
    (96.0, 192.0), // LBD（LP のペダルの絵）
];
/// パッドを描く順（上流 `CActPerfDrumsPad.n描画順`。足した左バスドラムは LP の次）
const PAD_ORDER: [Lane; LANES] = [
    Lane::LP,
    Lane::LBD,
    Lane::BD,
    Lane::SD,
    Lane::FT,
    Lane::LT,
    Lane::HT,
    Lane::RD,
    Lane::CY,
    Lane::HH,
    Lane::LC,
];

/// 火花の画像の名前（`ELane` の順。左バスドラムは LP の色を変えたもの）
const FIRE: [&str; LANES] = [
    "LC", "HH", "SD", "BD", "HT", "LT", "FT", "CY", "LP", "RD", "LP",
];
/// 飛び散るチップの切り出しの左と幅（上流 `nノーツの左上X座標`、`nノーツの幅`）。
/// 左バスドラムのレーンは [`left_bass_chips`] の絵の LP の列
const PIECES: [(f32, f32); LANES] = [
    (538.0, 64.0),
    (70.0, 46.0),
    (126.0, 54.0),
    (0.0, 60.0),
    (190.0, 46.0),
    (246.0, 46.0),
    (302.0, 46.0),
    (358.0, 60.0),
    (660.0, 48.0),
    (432.0, 48.0),
    (LEFT_BASS_LP_SX, 48.0),
];

/// 判定の文字の縦の段（上流 `n文字の縦表示位置`、`ELane` の順）
const JUDGE_ROW: [i32; LANES] = [-1, 1, 1, 2, 0, 0, 1, -1, 2, 1, 2];

/// チップの x と、7_chips_drums.png の切り出しの左と幅、まわりの模様（下の段のコマ送り）を描くか
/// （上流 `tUpdateAndDraw_Chip_Drums` と `tUpdateAndDraw_Chip_PatternOnly_Drums`）。
/// 水色の左バスドラムは、[`left_bass_chips`] の絵から切り出す（BD のレーンなら BD の列、足したレーンなら LP の列）
fn chip(looks: &Looks, l: DrumLane) -> (f32, f32, f32, bool) {
    let x = if l == DrumLane::HHO {
        looks.layout.chip_x(l)
    } else {
        looks.layout.chip[looks.lane_of(l).i()]
    };
    match l {
        DrumLane::HH => (x, 70.0, 56.0, true),
        DrumLane::SD => (x, 126.0, 64.0, true),
        DrumLane::BD => (x, 0.0, 70.0, true),
        DrumLane::HT => (x, 190.0, 56.0, true),
        DrumLane::LT => (x, 246.0, 56.0, true),
        DrumLane::CY => (x, 358.0, 74.0, true),
        DrumLane::FT => (x, 302.0, 56.0, true),
        // HH オープンの A と B は 1 ドット右（上流の x + 14 - 13）。B は丸（模様）を描かない。
        // C はクローズの絵を 3 ドット左（上流の x + 13 - 16）
        DrumLane::HHO => match looks.hho {
            HhoLook::Nx => (x + 1.0, 612.0, 48.0, true),
            HhoLook::NoCircle => (x + 1.0, 612.0, 48.0, false),
            HhoLook::Closed => (x - 3.0, 70.0, 56.0, true),
        },
        DrumLane::RD => (x, 432.0, 48.0, true),
        DrumLane::LC => (x, 538.0, 74.0, true),
        DrumLane::LP => (x, 660.0, 58.0, true),
        DrumLane::LBD => match looks.lbd {
            LbdLook::BdLane => (x, 0.0, BD_CHIP_W as f32, true),
            LbdLook::OwnLane => (x, LEFT_BASS_LP_SX, LP_CHIP_W as f32, true),
            LbdLook::SameAsLp => (x, 660.0, 58.0, true),
            LbdLook::Own => (x, 480.0, 58.0, true),
        },
    }
}

/// 左バスドラムのレーンを足したときに、コンボを右へずらす幅（画面の右の端に収まるだけ）
const COMBO_SHIFT: f32 = 30.0;

/// 左バスドラムの色（色相、度。水色）。BD（青紫、およそ 251 度）と LP（ピンク、およそ 331 度）の絵をこの色相に回す
const LEFT_BASS_HUE: f32 = 190.0;
/// 左バスドラムの絵の鮮やかさを元の何倍にするか（BD の絵は淡いので、ほかのレーンの絵と同じくらいに上げる）
const LEFT_BASS_SATURATION_BD: f32 = 1.9;
const LEFT_BASS_SATURATION_LP: f32 = 1.1;
/// 左バスドラムの色の見本（設定の配置図に使う）
pub const LEFT_BASS_COLOR: Color32 = Color32::from_rgb(102, 230, 255);
/// 7_chips_drums.png の BD の列の幅と、LP の列の左と幅
const BD_CHIP_W: u32 = 70;
const LP_CHIP_SX: u32 = 660;
const LP_CHIP_W: u32 = 58;
/// [`left_bass_chips`] の絵の中の、LP の列から作った絵の左
const LEFT_BASS_LP_SX: f32 = BD_CHIP_W as f32;
/// 7_Paret.png の LP の枠の左と幅
const LP_PANEL_SX: u32 = 121;
const LP_PANEL_W: u32 = 51;

/// 1 ドットの色相を [`LEFT_BASS_HUE`] にし、彩度を `sat` 倍にする（明るさと透明さはそのまま）
fn tint(p: image::Rgba<u8>, sat: f32) -> image::Rgba<u8> {
    let image::Rgba([r, g, b, a]) = p;
    if a == 0 {
        return p;
    }
    let (_, s, v) = rgb_to_hsv(r, g, b);
    let (r, g, b) = hsv_to_rgb(LEFT_BASS_HUE, (s * sat).min(1.0), v);
    image::Rgba([r, g, b, a])
}

/// 水色の左バスドラムのチップ。左から、7_chips_drums.png の BD の列（チップ・模様・飛び散るかけら）を
/// 左右に返した（足の模様を左足に）もの（BD のレーンに出すとき）と、LP の列（もとから左足）のもの（足したレーン）
pub fn left_bass_chips(src: image::RgbaImage) -> image::RgbaImage {
    let bd = BD_CHIP_W.min(src.width());
    let lp = LP_CHIP_W.min(src.width().saturating_sub(LP_CHIP_SX));
    let mut out = image::RgbaImage::new(bd + lp, src.height());
    for (x, y, p) in out.enumerate_pixels_mut() {
        *p = if x < bd {
            tint(*src.get_pixel(bd - 1 - x, y), LEFT_BASS_SATURATION_BD)
        } else {
            tint(
                *src.get_pixel(LP_CHIP_SX + x - bd, y),
                LEFT_BASS_SATURATION_LP,
            )
        };
    }
    out
}

/// 足した左バスドラムのレーンの枠（7_Paret.png の LP の枠を水色に）
fn left_bass_panel(src: image::RgbaImage) -> image::RgbaImage {
    let w = LP_PANEL_W.min(src.width().saturating_sub(LP_PANEL_SX));
    image::RgbaImage::from_fn(w, src.height(), |x, y| {
        tint(*src.get_pixel(LP_PANEL_SX + x, y), 1.0)
    })
}

/// 絵の全体を水色に（足したレーンの光・火花・星。LP のものから作る）
fn left_bass_tinted(mut src: image::RgbaImage) -> image::RgbaImage {
    for p in src.pixels_mut() {
        *p = tint(*p, 1.0);
    }
    src
}

/// 水色の左バスドラムのチップの絵（[`left_bass_chips`]。一度作ったら覚えておく）
fn left_bass_tex(g: &Gfx) -> Option<Tex> {
    g.skin
        .tex_made("Graphics/7_chips_drums.png", "left_bass", left_bass_chips)
}

/// スキンの絵 `rel` を水色にしたもの（足したレーンの光・火花・星）
fn left_bass_tinted_tex(g: &Gfx, rel: &str) -> Option<Tex> {
    g.skin.tex_made(rel, "left_bass", left_bass_tinted)
}

/// RGB（0〜255）を色相（0〜360 度）・彩度・明度（0〜1）に
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

/// [`rgb_to_hsv`] の逆
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to(r), to(g), to(b))
}

/// 判定の画像の列（上流の 7_judge strings.png の並び）
fn judge_column(j: Judgement) -> f32 {
    match j {
        Judgement::Perfect => 0.0,
        Judgement::Great => 1.0,
        Judgement::Good => 2.0,
        Judgement::Poor => 3.0,
        Judgement::Miss => 4.0,
    }
}

/// 経過した時間を、`step` ごとに 1 進むカウンタの値に（上流 `CCounter`）
fn count(since: Instant, now: Instant, step_ms: f64) -> f64 {
    (now.saturating_duration_since(since).as_secs_f64() * 1000.0 / step_ms).floor()
}

/// 動きのばらつきに使う乱数（xorshift）
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64)
            | 1;
        Self(seed)
    }

    /// 0 以上 `n` 未満
    fn below(&mut self, n: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % u64::from(n.max(1))) as u32
    }
}

/// パッドの動き（上流 `STパッド状態`）
#[derive(Debug, Clone, Copy)]
struct PadState {
    /// 明るさ（叩くと 6、18 ミリ秒ごとに 1 下がる）
    bright: i32,
    /// 下への沈み（0〜11）と、その速さ
    dip: i32,
    dip_speed: i32,
    bright_at: Instant,
    dip_at: Instant,
}

impl PadState {
    fn new(now: Instant) -> Self {
        Self {
            bright: 0,
            dip: 0,
            dip_speed: 0,
            bright_at: now,
            dip_at: now,
        }
    }

    fn hit(&mut self, now: Instant) {
        if self.bright == 0 {
            self.bright_at = now;
        }
        if self.dip_speed == 0 && self.dip == 0 {
            self.dip_at = now;
        }
        self.bright = 6;
        self.dip_speed = 2;
    }

    fn update(&mut self, now: Instant) {
        const BRIGHT_STEP: Duration = Duration::from_millis(18);
        const DIP_STEP: Duration = Duration::from_millis(8);
        while self.bright > 0 && now.saturating_duration_since(self.bright_at) >= BRIGHT_STEP {
            self.bright -= 1;
            self.bright_at += BRIGHT_STEP;
        }
        while (self.dip_speed != 0 || self.dip != 0)
            && now.saturating_duration_since(self.dip_at) >= DIP_STEP
        {
            self.dip += self.dip_speed;
            if self.dip > 11 {
                self.dip = 11;
                self.dip_speed = -1;
            } else if self.dip < 0 {
                self.dip = 0;
                self.dip_speed = 0;
            }
            self.dip_at += DIP_STEP;
        }
    }
}

/// 火花のまわりに散る星 1 つ（上流 `ST青い星`）
#[derive(Debug, Clone, Copy)]
struct Star {
    lane: Lane,
    start: Instant,
    steps: u32,
    x: f32,
    y: f32,
    ax: f32,
    ay: f32,
    radius: f32,
}

/// 飛び散るチップのかけら（上流 `ST飛び散るチップ`）
#[derive(Debug, Clone, Copy)]
struct Piece {
    lane: Lane,
    /// 左バスドラムの色の絵から切り出す
    left_bass: bool,
    start: Instant,
    steps: u32,
    xl: f32,
    xr: f32,
    y: f32,
    ax: f32,
    ay: f32,
}

/// 判定の文字（上流 `CActPerfCommonJudgementString.st状態`）
#[derive(Debug, Clone, Copy)]
struct JudgeShow {
    judgement: Judgement,
    /// ずれ（ミリ秒、マイナスは早い）。見逃しは None
    lag: Option<i64>,
    start: Instant,
}

/// スコア（上流 `CActPerfCommonScore`：本当の点数へ 10 ミリ秒ごとに近づける）
#[derive(Debug, Clone, Copy)]
struct Score {
    truth: f64,
    shown: i64,
    step: i64,
    at: Instant,
}

/// 進み具合のバーの 1 区間（上流 `CProgressSection`）
#[derive(Debug, Clone, Copy, Default)]
struct Section {
    notes: u32,
    hits: u32,
}

/// 進み具合のバーの区間の数（上流 `nSectionIntervalCount`）
const SECTIONS: usize = 64;

pub struct PlayView {
    entered: Instant,
    /// 難易度の名前（7_Difficulty.png のどこを切り出すかを Script/difficult.dtxs で引く）
    diff_panel: (f32, f32),
    /// 叩いた時刻と強さ（レーンの光）
    flush: [Option<(Instant, u8)>; LANES],
    pads: [PadState; LANES],
    fires: [Option<Instant>; LANES],
    stars: Vec<Star>,
    pieces: Vec<Piece>,
    judges: [Option<JudgeShow>; LANES],
    tally: Tally,
    /// コンボが増えた時刻（数字が跳ねる）
    combo_jump: Option<Instant>,
    /// 100 コンボごとの爆発（何百コンボ目か、始めた時刻）
    bomb: Option<(u32, Instant)>,
    bombed: Vec<u32>,
    gauge: Gauge,
    score: Score,
    sections: [Section; SECTIONS],
    /// 音符の数（スコアと達成率の分母）
    notes: usize,
    rng: Rng,
    /// STAGE FAILED を始めた時刻と、そのときの曲の時刻（譜面はそこで止める）
    failed: Option<(Instant, f64)>,
    /// 早く叩いた数と遅く叩いた数（上流 `nTimingHitCount`。見逃しは数えない）
    early: u32,
    late: u32,
    lag_digits: LagDigits,
    looks: Looks,
    /// スコアの下のずれのバー（出さない設定でも印は集めておく。一時停止で ON にしたらすぐ出す）
    hit_error: HitErrorBar,
}

/// 判定のずれの数字の出し方（NX の `ShowLagTime` と `ShowLagTimeColor`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LagDigits {
    pub show: LagShow,
    /// 早いを赤、遅いを青にする（NX の既定は逆）
    pub swapped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LagShow {
    Off,
    On,
    /// PERFECT 以外だけ
    NotPerfect,
}

impl LagDigits {
    /// 設定の `show_lag_time` と `show_lag_time_color` から
    pub fn from_config(c: &crate::config::Config) -> Self {
        let show = match c.show_lag_time {
            0 => LagShow::Off,
            2 => LagShow::NotPerfect,
            _ => LagShow::On,
        };
        Self {
            show,
            swapped: c.show_lag_time_color == 1,
        }
    }

    fn shows(self, j: Judgement) -> bool {
        match self.show {
            LagShow::Off => false,
            LagShow::On => true,
            LagShow::NotPerfect => j != Judgement::Perfect,
        }
    }
}

/// チップの見えにくくし方（上流 `nHidSud`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidSud {
    Off,
    /// 判定ラインの近くで消える
    Hidden,
    /// 判定ラインの近くまで来てから出る
    Sudden,
    /// 両方
    Both,
    /// ずっと見えない
    Stealth,
}

/// 叩いたときの効果（上流 `eAttackEffect`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackEffect {
    AllOn,
    /// 飛び散るチップを出さない
    ChipOff,
    /// 火花だけ
    EffectOnly,
    AllOff,
}

/// 判定の文字の位置（上流 `JudgementStringPosition`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgePosition {
    /// レーンの上（P-A）
    Lane,
    /// 判定ラインの下（P-B）
    Below,
    Off,
}

/// オープンハイハットのチップの絵（上流 `eHHOGraphics`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HhoLook {
    /// NX の絵（A）
    Nx,
    /// A のまわりの丸を描かない（B）
    NoCircle,
    /// クローズと同じ絵（C）
    Closed,
}

/// 左バスドラムのチップの出し方（NXT の 2 つと、上流 `eLBDGraphics`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LbdLook {
    /// BD のレーンに、BD の絵を水色にして出す（NXT の既定）
    BdLane,
    /// LP の右に足したレーンに、LP の絵を水色にして出す
    OwnLane,
    /// LP のレーンに LP と同じ絵（A）
    SameAsLp,
    /// LP のレーンに LP と色の違う絵（B）
    Own,
}

/// 譜面の見た目の設定（設定の表の「譜面の見た目」から）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Looks {
    /// 判定ラインを下から上げるドット（上流 `nJudgeLine`。Reverse では上から下げる）
    pub judge_line_up: f32,
    /// 下から上へ流す（上流 `bReverse.Drums`）
    pub reverse: bool,
    pub hid_sud: HidSud,
    /// シャッター（0〜100。100 で画面の高さ全部）
    pub shutter_in: f32,
    pub shutter_out: f32,
    /// レーンの背景を暗くする（上流 `nLaneDisp` の 1 と 3）
    pub dark_lanes: bool,
    /// 小節線と拍線を出す（上流 `nLaneDisp` の 0 と 1）
    pub bar_lines: bool,
    pub judge_line: bool,
    pub lane_flush: bool,
    pub attack: AttackEffect,
    pub judge_position: JudgePosition,
    pub combo: bool,
    /// コンボをこの数から出す
    pub min_combo: u32,
    /// 進み具合のバーを出す（Dark が FULL なら出さない）
    pub progress: bool,
    /// スコアの下にずれのバーを出す
    pub hit_error: bool,
    /// レーンの並び
    pub layout: Layout,
    pub hho: HhoLook,
    pub lbd: LbdLook,
}

impl Default for Looks {
    fn default() -> Self {
        Self::from_settings(&crate::settings::Values::default())
    }
}

impl Looks {
    pub fn from_settings(v: &crate::settings::Values) -> Self {
        use crate::settings as s;
        let lane_disp = v.choice(s::LANE_DISP);
        Self {
            judge_line_up: v.number(s::JUDGE_LINE_POS) as f32,
            reverse: v.flag(s::REVERSE),
            hid_sud: match v.choice(s::HID_SUD) {
                1 => HidSud::Hidden,
                2 => HidSud::Sudden,
                3 => HidSud::Both,
                4 => HidSud::Stealth,
                _ => HidSud::Off,
            },
            shutter_in: v.number(s::SHUTTER_IN) as f32,
            shutter_out: v.number(s::SHUTTER_OUT) as f32,
            dark_lanes: matches!(lane_disp, 1 | 3),
            bar_lines: matches!(lane_disp, 0 | 1),
            judge_line: v.flag(s::JUDGE_LINE_DISP),
            lane_flush: v.flag(s::LANE_FLUSH),
            attack: match v.choice(s::ATTACK_EFFECT) {
                1 => AttackEffect::ChipOff,
                2 => AttackEffect::EffectOnly,
                3 => AttackEffect::AllOff,
                _ => AttackEffect::AllOn,
            },
            judge_position: match v.choice(s::JUDGE_POSITION) {
                1 => JudgePosition::Below,
                2 => JudgePosition::Off,
                _ => JudgePosition::Lane,
            },
            combo: v.flag(s::COMBO_DISP),
            min_combo: v.number(s::MIN_COMBO) as u32,
            progress: !v.dark_full(),
            hit_error: v.flag(s::HIT_ERROR_BAR),
            layout: Layout::from_settings(v),
            hho: match v.choice(s::HHO_GRAPHICS) {
                1 => HhoLook::NoCircle,
                2 => HhoLook::Closed,
                _ => HhoLook::Nx,
            },
            lbd: match v.choice(s::LBD_GRAPHICS) {
                1 => LbdLook::OwnLane,
                2 => LbdLook::SameAsLp,
                3 => LbdLook::Own,
                _ => LbdLook::BdLane,
            },
        }
    }

    /// 叩いた・判定したレーンを、光や判定の文字を出すレーンに（左バスドラムは出し方で変わる）
    pub fn lane_of(&self, l: DrumLane) -> Lane {
        match (l, self.lbd) {
            (DrumLane::LBD, LbdLook::BdLane) => Lane::BD,
            (DrumLane::LBD, LbdLook::OwnLane) => Lane::LBD,
            _ => Lane::of(l),
        }
    }

    /// 水色の左バスドラムか（[`left_bass_chips`] の絵を使う）
    fn left_bass_tinted(&self, l: DrumLane) -> bool {
        l == DrumLane::LBD && matches!(self.lbd, LbdLook::BdLane | LbdLook::OwnLane)
    }

    /// 判定ラインの y（上流 `nJudgeLinePosY.Drums`）
    fn judge_y(&self) -> f32 {
        if self.reverse {
            REVERSE_JUDGE_LINE_Y + self.judge_line_up
        } else {
            JUDGE_LINE_Y - self.judge_line_up
        }
    }

    /// 判定ラインから `ahead` ドット先（まだ来ていない側）の y
    fn ahead_y(&self, ahead: f32) -> f32 {
        if self.reverse {
            self.judge_y() + ahead
        } else {
            self.judge_y() - ahead
        }
    }

    /// 画面の y が、チップが出てくる側の端より先か（そこから先は描かない）
    fn beyond_edge(&self, y: f32) -> bool {
        if self.reverse {
            y > 720.0 + 64.0
        } else {
            y < -64.0
        }
    }

    /// 火花・星・飛び散るチップの基準の y（上流 `actChipFireD.iPosY`）
    fn fire_y(&self) -> f32 {
        self.judge_y() - if self.reverse { 183.0 } else { 186.0 }
    }

    /// 判定ラインから `distance` ドット上にあるチップの濃さ（上流の Sudden・Hidden・ステルスの処理）。
    /// 見えなければ None
    fn chip_alpha(&self, distance: f32) -> Option<f32> {
        let mut alpha = 1.0;
        if matches!(self.hid_sud, HidSud::Sudden | HidSud::Both) {
            if distance >= 250.0 {
                return None;
            } else if distance >= 200.0 {
                alpha = 1.0 - (distance - 200.0) / 50.0;
            }
        }
        if matches!(self.hid_sud, HidSud::Hidden | HidSud::Both) {
            if distance < 100.0 {
                return None;
            } else if distance < 150.0 {
                alpha = (distance - 100.0) / 50.0;
            }
        }
        if self.hid_sud == HidSud::Stealth {
            return None;
        }
        Some(alpha)
    }
}

/// 演奏を終えたときの数字（結果の画面と記録に渡す）
#[derive(Debug, Clone, PartialEq)]
pub struct Finish {
    pub score: i64,
    /// 達成率（練習は 0）
    pub rate: f64,
    /// 進み具合のバー（上流の score.ini と同じ 64 文字）
    pub progress: String,
    pub early: u32,
    pub late: u32,
    /// 7_Difficulty.png の切り出しの左上
    pub diff_panel: (f32, f32),
}

impl PlayView {
    /// `label` は選んだ難易度の名前（set.def の `#L1LABEL` など。無ければ空）
    pub fn new(play: &Play, label: &str, lag_digits: LagDigits, looks: Looks) -> Self {
        let now = Instant::now();
        let mut sections = [Section::default(); SECTIONS];
        if play.practice.is_none() {
            for n in play.judge.notes() {
                if let Some(s) = sections.get_mut(section_of(n.ms, play.last_chip_ms)) {
                    s.notes += 1;
                }
            }
        }
        Self {
            entered: now,
            diff_panel: difficulty_cell("7", label).unwrap_or((0.0, 0.0)),
            flush: [None; LANES],
            pads: [PadState::new(now); LANES],
            fires: [None; LANES],
            stars: Vec::new(),
            pieces: Vec::new(),
            judges: [None; LANES],
            tally: Tally::default(),
            combo_jump: None,
            bomb: None,
            bombed: Vec::new(),
            gauge: Gauge::new(play.gauge),
            score: Score {
                truth: 0.0,
                shown: 0,
                step: 0,
                at: now,
            },
            sections,
            notes: play.judge.notes().len(),
            rng: Rng::new(),
            failed: None,
            early: 0,
            late: 0,
            lag_digits,
            looks,
            hit_error: HitErrorBar::new(play.judge.windows()),
        }
    }

    /// ずれのバーを出すか（一時停止の板で変えたとき）
    pub fn set_hit_error(&mut self, on: bool) {
        self.looks.hit_error = on;
    }

    /// 演奏を終えたときの数字。`now_ms` は曲の時刻（ここまでの区間をたどり着いたことにする）
    pub fn finish(&self, play: &Play, now_ms: f64) -> Finish {
        let practice = play.practice.is_some();
        Finish {
            score: self.score.truth as i64,
            rate: self.rate(practice),
            progress: if practice {
                String::new()
            } else {
                progress_text(&self.sections, play.last_chip_ms, now_ms)
            },
            early: self.early,
            late: self.late,
            diff_panel: self.diff_panel,
        }
    }

    /// ゲージが尽きた（上流 `IsFailed`。Risky なら残りの回数が尽きた）
    pub fn gauge_empty(&self) -> bool {
        self.gauge.empty()
    }

    /// STAGE FAILED を始める。`now_ms` は今の曲の時刻（譜面をそこで止める）
    pub fn start_failure(&mut self, now_ms: f64) {
        self.failed.get_or_insert((Instant::now(), now_ms));
    }

    /// STAGE FAILED の途中なら、止めた曲の時刻
    pub fn frozen_ms(&self) -> Option<f64> {
        self.failed.map(|(_, ms)| ms)
    }

    /// STAGE FAILED を出し終えた
    pub fn failure_done(&self) -> bool {
        self.failed.is_some_and(|(at, _)| at.elapsed() >= FAIL_FOR)
    }

    /// `Play` から出来事を受け取る。毎フレーム描く前に呼ぶ
    pub fn feed(&mut self, play: &mut Play) {
        let now = Instant::now();
        let practice = play.practice.is_some();
        for fx in play.take_fx() {
            match fx {
                Fx::Hit { lane, velocity } => {
                    let l = self.looks.lane_of(lane);
                    self.flush[l.i()] = Some((now, velocity));
                    self.pads[l.i()].hit(now);
                }
                Fx::Judged {
                    lane,
                    judgement,
                    offset_ms,
                    note_ms,
                } => {
                    let l = self.looks.lane_of(lane);
                    match offset_ms {
                        Some(o) if o > 0.0 => self.late += 1,
                        Some(_) => self.early += 1,
                        None => {}
                    }
                    if let Some(o) = offset_ms {
                        self.hit_error.push(now, o, judgement);
                    }
                    self.judges[l.i()] = Some(JudgeShow {
                        judgement,
                        lag: offset_ms.map(|o| o as i64),
                        start: now,
                    });
                    if !matches!(judgement, Judgement::Poor | Judgement::Miss) {
                        self.start_fire(l, self.looks.left_bass_tinted(lane), now);
                    }
                    let before = self.tally.combo;
                    self.tally.add(judgement);
                    if self.tally.combo > before {
                        self.combo_jump = Some(now);
                    }
                    self.gauge.apply(judgement);
                    if !practice {
                        self.add_score(judgement, now);
                        if matches!(
                            judgement,
                            Judgement::Perfect | Judgement::Great | Judgement::Good
                        ) && let Some(s) = self
                            .sections
                            .get_mut(section_of(note_ms, play.last_chip_ms))
                        {
                            s.hits += 1;
                        }
                    }
                }
                Fx::StrayMiss { lane } => {
                    // 上流の Tight: MISS を出し、コンボを切り、MISS と同じだけゲージを減らす（MISS の数には入れない）
                    self.judges[self.looks.lane_of(lane).i()] = Some(JudgeShow {
                        judgement: Judgement::Miss,
                        lag: None,
                        start: now,
                    });
                    self.tally.combo = 0;
                    self.gauge.apply(Judgement::Miss);
                }
            }
        }
        // 練習は周を足すたびに音符が増える
        self.notes = play.judge.notes().len();
    }

    /// 火花と星と飛び散るチップを出す（上流 `CActPerfDrumsChipFireD.Start`）
    /// 叩いたときの効果の設定（上流 `eAttackEffect`）で、出すものを選ぶ。判定ラインを上げたら一緒に上げる
    /// `left_bass` なら、飛び散るチップを左バスドラムの色にする
    fn start_fire(&mut self, l: Lane, left_bass: bool, now: Instant) {
        let attack = self.looks.attack;
        if attack == AttackEffect::AllOff {
            return;
        }
        let fire_y = self.looks.fire_y();
        self.fires[l.i()] = Some(now);
        let cx = self.looks.layout.fire[l.i()] + 320.0;
        if matches!(attack, AttackEffect::AllOn | AttackEffect::ChipOff) && self.stars.len() < 240 {
            for _ in 0..16 {
                let deg = self.rng.below(360) as f32;
                let (sin, cos) = deg.to_radians().sin_cos();
                self.stars.push(Star {
                    lane: l,
                    start: now,
                    steps: 0,
                    x: cx,
                    y: fire_y + 180.0,
                    ax: 0.9 * cos,
                    ay: 0.9 * sin - 0.1,
                    radius: 0.3 + self.rng.below(30) as f32 / 100.0,
                });
            }
        }
        if attack == AttackEffect::AllOn && self.pieces.len() < 8 {
            let w = PIECES[l.i()].1;
            let x = self.looks.layout.fire[l.i()] + w + 312.0;
            let shift = match l {
                Lane::LC | Lane::BD | Lane::CY => 0.0,
                Lane::HH => 20.0,
                _ => 10.0,
            };
            let deg = 1.0_f32.to_radians();
            self.pieces.push(Piece {
                lane: l,
                left_bass,
                start: now,
                steps: 0,
                xl: x + shift,
                xr: x,
                y: fire_y + 189.0,
                ax: 0.91 * deg.cos() + 0.3,
                ay: 0.91 * (deg.sin() - 0.8),
            });
        }
    }

    /// XG のスコア（上流 `tProcessChipHit` の「100万/{1275+50×(総ノーツ数-50)}」）
    fn add_score(&mut self, j: Judgement, now: Instant) {
        let s = &mut self.score;
        let delta = score_delta(j, &self.tally, self.notes, s.truth);
        if delta <= 0.0 {
            return;
        }
        s.truth = (s.truth + delta).min(MAX_SCORE);
        s.step = (((s.truth - s.shown as f64) / 20.0) as i64).max(1);
        s.at = now;
    }

    /// 演奏の達成率（上流 `tCalculatePlayingSkill`）。練習では出さない（上流のトレーニングと同じ）
    fn rate(&self, practice: bool) -> f64 {
        if practice || self.notes == 0 {
            return 0.0;
        }
        let n = self.notes as f64;
        let t = &self.tally;
        100.0 * f64::from(t.perfect) / n * 0.85
            + 100.0 * f64::from(t.great) / n * 0.35
            + 100.0 * f64::from(t.max_combo) / n * 0.15
    }

    /// 1 フレーム描く。`now_ms` は曲の時刻（表示の補正を足したもの）
    pub fn draw(&mut self, c: &Canvas, g: &mut Gfx, play: &Play, now_ms: f64, px_per_ms: f32) {
        let now = Instant::now();
        let skin = &g.skin;
        // 背景
        match skin.tex("Graphics/7_background.jpg") {
            Some(t) => c.draw(&t, 0.0, 0.0),
            None => c.cover(Color32::from_rgb(8, 8, 14)),
        }
        self.draw_jacket(c, g, play);
        self.draw_lanes(c, g, now);
        self.draw_lines(c, g, play, now_ms, px_per_ms);
        self.draw_chips(c, g, play, now_ms, px_per_ms, now);
        self.draw_shutters(c, g);
        if self.looks.progress {
            self.draw_progress(c, g, play, now_ms);
        }
        // 判定の線（8×8 の画像を横に並べる。上流は D3D の繰り返しで 559 ドットに広げている）
        if self.looks.judge_line
            && let Some(t) = g.skin.tex("Graphics/ScreenPlayDrums hit-bar.png")
        {
            c.draw_opt(
                &t,
                LANES_LEFT,
                self.looks.judge_y(),
                Opt {
                    src: Some(src(0.0, 0.0, t.w, 6.0)),
                    sx: self.looks.layout.width() / t.w,
                    ..Default::default()
                },
            );
        }
        self.draw_pads(c, g, now);
        let practice = play.practice.is_some();
        self.draw_score(c, g, now, practice);
        self.draw_status(c, g, play, practice);
        self.draw_gauge(c, g, px_per_ms);
        if self.looks.hit_error {
            self.hit_error.draw(c, g, now, practice);
        }
        self.draw_combo(c, g, play, now);
        self.draw_judges(c, g, now);
        self.draw_fire(c, g, now);
        if let Some(p) = &play.practice {
            crate::practice_hud::draw(
                c,
                g,
                p,
                now_ms,
                play.last_chip_ms,
                px_per_ms,
                self.looks.reverse,
                self.looks.layout.extra_w(),
            );
        }
        self.draw_failed(c, g, now);
        // 入ってきたときのフェード
        let t = now.saturating_duration_since(self.entered).as_secs_f32() / FADE_IN.as_secs_f32();
        if t < 1.0 {
            c.cover(Color32::from_black_alpha(((1.0 - t) * 255.0) as u8));
        }
    }

    /// ジャケットと曲名（上流 `CActPerfPanelString`）
    fn draw_jacket(&self, c: &Canvas, g: &mut Gfx, play: &Play) {
        // 練習中は、ジャケットの場所に段階アップのはしごを出す（practice_hud）。曲名は出す
        if play.practice.is_none() {
            self.draw_jacket_image(c, g, play);
        }
        let edge = |px: f32| Style {
            px,
            color: Color32::WHITE,
            bold: false,
            shadow: None,
            edge: Some((Color32::BLACK, px / 12.0)),
            gradient: None,
        };
        if let Some(t) = g.texts.get(&play.title, &edge(20.0)) {
            t.draw_fit(c, 950.0, 630.0, 320.0);
        }
        if let Some(t) = g.texts.get(&play.artist, &edge(15.0)) {
            t.draw_fit(c, 950.0, 665.0, 320.0);
        }
    }

    /// ジャケットの板と絵
    fn draw_jacket_image(&self, c: &Canvas, g: &mut Gfx, play: &Play) {
        if let Some(t) = g.skin.tex("Graphics/7_JacketPanel.png") {
            c.draw(&t, 915.0, 287.0);
        }
        let jacket = match &play.preimage {
            Some(p) => g.images.get(p).or_else(|| {
                g.images
                    .failed(p)
                    .then(|| g.skin.tex("Graphics/5_preimage default.png"))
                    .flatten()
            }),
            None => g.skin.tex("Graphics/5_preimage default.png"),
        };
        if let Some(t) = jacket {
            // 上流は 245 に縮めて (400, -227) へ動かし、画面の中心のまわりに 0.3 ラジアン回す
            let f = fit_scale(&t, 245.0);
            let (cx, cy) = rotate_about_center(400.0, -227.0, 0.3);
            c.draw_rot(
                &t,
                cx,
                cy,
                0.3,
                Opt {
                    sx: f,
                    sy: f,
                    ..Default::default()
                },
            );
        }
    }

    /// レーンの枠と光（上流 `CActPerfDrumsLaneFlushD`）
    fn draw_lanes(&mut self, c: &Canvas, g: &Gfx, now: Instant) {
        // レーンを暗くするなら暗いほうの絵（上流 `nLaneDisp`）
        let paret = if self.looks.dark_lanes {
            "Graphics/7_Paret_Dark.png"
        } else {
            "Graphics/7_Paret.png"
        };
        let layout = self.looks.layout;
        if let Some(t) = g.skin.tex(paret) {
            for (l, sx, w) in PANEL {
                if l == Lane::LBD {
                    // 足した左バスドラムのレーン（LP の枠を水色に）
                    if layout.left_bass_lane
                        && let Some(t) = g.skin.tex_made(paret, "left_bass", left_bass_panel)
                    {
                        c.part(&t, layout.panel[l.i()], 0.0, src(0.0, 0.0, w, 720.0));
                    }
                    continue;
                }
                c.part(&t, layout.panel[l.i()], 0.0, src(sx, 0.0, w, 720.0));
            }
        }
        let (flush, reverse) = (self.looks.lane_flush, self.looks.reverse);
        for (i, slot) in self.flush.iter_mut().enumerate() {
            let Some((at, velocity)) = *slot else {
                continue;
            };
            if !flush {
                *slot = None;
                continue;
            }
            // 強く叩くほど判定ラインの側から始まる（上流：(1 - 強さ) × 55 から 90 まで 3 ミリ秒ごと）
            let from = ((1.0 - f64::from(velocity) / 127.0) * 55.0).floor();
            let v = from + count(at, now, 3.0);
            if v >= 90.0 {
                *slot = None;
                continue;
            }
            let x = layout.flush[i];
            let (w, name) = FLUSH[i];
            let rev = if reverse { " reverse" } else { "" };
            let rel = format!("Graphics/ScreenPlayDrums lane flush {name}{rev}.png");
            let tex = if i == Lane::LBD.i() {
                left_bass_tinted_tex(g, &rel)
            } else {
                g.skin.tex(&rel)
            };
            let Some(t) = tex else {
                continue;
            };
            let step = (v as i32 * 740 / 100) as f32;
            // 下から上へ流すときは上から下りてくる絵で、薄くしない（上流と同じ）
            let (y, alpha) = if reverse {
                (32.0 + step, 1.0)
            } else {
                let y = 700.0 - step;
                (y, (y / 255.0).min(1.0))
            };
            for k in 0..3 {
                let mut n = 0.0;
                while n < w {
                    c.draw_opt(
                        &t,
                        x + n,
                        y,
                        Opt {
                            src: Some(src(k as f32 * 42.0, 0.0, (w - n).min(42.0), 128.0)),
                            alpha,
                            ..Default::default()
                        },
                    );
                    n += 42.0;
                }
            }
        }
    }

    /// 小節線と拍線（上流 `tUpdateAndDraw_Chip_BarLine` と拍線）。練習では周ごとに並べる
    fn draw_lines(&self, c: &Canvas, g: &mut Gfx, play: &Play, now_ms: f64, px_per_ms: f32) {
        let Some(t) = g.skin.tex("Graphics/7_chips_drums.png") else {
            return;
        };
        let looks = self.looks;
        let y_of = |ms: f64| looks.ahead_y(((ms - now_ms) as f32) * px_per_ms);
        let bar_lines = looks.bar_lines;
        let width = looks.layout.width();
        let draw = |ms: f64, bar: bool| {
            let y = y_of(ms);
            if bar_lines && (-4.0..724.0).contains(&y) {
                let sy = if bar { 769.0 } else { 772.0 };
                c.draw_opt(
                    &t,
                    LANES_LEFT,
                    y - 1.0,
                    Opt {
                        src: Some(src(0.0, sy, 559.0, 2.0)),
                        sx: width / 559.0,
                        ..Default::default()
                    },
                );
            }
        };
        match &play.practice {
            None => {
                for l in &play.lines {
                    draw(l.ms, l.bar);
                }
            }
            Some(p) => {
                for lap in &p.laps {
                    let span = lap.plan.audio;
                    for l in play
                        .lines
                        .iter()
                        .filter(|l| l.ms >= span.from_ms && l.ms < span.to_ms)
                    {
                        draw(lap.plan.real_ms(l.ms), l.bar);
                    }
                    // 周の頭に何周目かを出す
                    let y = y_of(lap.plan.start_ms);
                    let judge_y = looks.judge_y();
                    let ahead = if looks.reverse {
                        (judge_y..720.0).contains(&y)
                    } else {
                        (0.0..judge_y).contains(&y)
                    };
                    if ahead
                        && let Some(text) = g.texts.get(
                            &format!("{} 周目", lap.in_set),
                            &Style {
                                px: 14.0,
                                color: Color32::WHITE,
                                bold: true,
                                shadow: None,
                                edge: Some((Color32::BLACK, 1.5)),
                                gradient: None,
                            },
                        )
                    {
                        text.draw(c, 300.0, y - text.h - 2.0, 1.0, 1.0);
                    }
                }
            }
        }
    }

    /// チップ（上流 `tUpdateAndDraw_Chip_PatternOnly_Drums` の模様を描いてから `tUpdateAndDraw_Chip_Drums`）
    fn draw_chips(
        &self,
        c: &Canvas,
        g: &Gfx,
        play: &Play,
        now_ms: f64,
        px_per_ms: f32,
        now: Instant,
    ) {
        let Some(t) = g.skin.tex("Graphics/7_chips_drums.png") else {
            return;
        };
        let looks = self.looks;
        let left_bass = matches!(looks.lbd, LbdLook::BdLane | LbdLook::OwnLane)
            .then(|| left_bass_tex(g))
            .flatten();
        let tex_of = |lane: DrumLane| match &left_bass {
            Some(lb) if looks.left_bass_tinted(lane) => lb,
            _ => &t,
        };
        let ahead = |ms: f64| ((ms - now_ms) as f32) * px_per_ms;
        let visible = |ms: f64| {
            let y = looks.ahead_y(ahead(ms));
            y > -64.0 && y < 720.0 + 64.0 && ahead(ms) < DRAW_AHEAD_PX + 64.0
        };
        // 模様は 70 ミリ秒ごとに 8 コマ
        let frame = count(self.entered, now, 70.0) as u32 % 8;
        let notes = play.judge.notes();
        let from = notes.partition_point(|n| n.ms < now_ms - 1000.0);
        let mut shown: Vec<(DrumLane, f32, f32)> = Vec::new();
        for (i, n) in notes.iter().enumerate().skip(from) {
            let y = looks.ahead_y(ahead(n.ms));
            if looks.beyond_edge(y) {
                break;
            }
            if play.judge.is_done(i) || !visible(n.ms) {
                continue;
            }
            if let Some(alpha) = looks.chip_alpha(ahead(n.ms)) {
                shown.push((n.lane, y, alpha));
            }
        }
        // 練習の助走など、判定しない音符は薄く
        if let Some(p) = &play.practice {
            for &(ms, lane) in &p.faint {
                if visible(ms)
                    && let Some(alpha) = looks.chip_alpha(ahead(ms))
                {
                    shown.push((lane, looks.ahead_y(ahead(ms)), 0.35 * alpha));
                }
            }
        }
        for &(lane, y, alpha) in &shown {
            let (x, sx, w, pattern) = chip(&looks, lane);
            if !pattern {
                continue;
            }
            c.draw_opt(
                tex_of(lane),
                x - 5.0,
                y - 32.0,
                Opt {
                    src: Some(src(sx, 128.0 + frame as f32 * 64.0, w, 64.0)),
                    alpha,
                    ..Default::default()
                },
            );
        }
        for &(lane, y, alpha) in &shown {
            let (x, sx, w, _) = chip(&looks, lane);
            c.draw_opt(
                tex_of(lane),
                x - 5.0,
                y - 32.0,
                Opt {
                    src: Some(src(sx, 0.0, w, 64.0)),
                    alpha,
                    ..Default::default()
                },
            );
        }
    }

    /// シャッター（上流の ShutterInPos・ShutterOutPos。7_shutter.png を上と下から 7.2 ドット × 値だけ出す）
    fn draw_shutters(&self, c: &Canvas, g: &Gfx) {
        // 出てくる側が ShutterIn（Reverse では下）
        let (top, bottom) = if self.looks.reverse {
            (self.looks.shutter_out, self.looks.shutter_in)
        } else {
            (self.looks.shutter_in, self.looks.shutter_out)
        };
        if top <= 0.0 && bottom <= 0.0 {
            return;
        }
        let Some(t) = g.skin.tex("Graphics/7_shutter.png") else {
            return;
        };
        // 左バスドラムのレーンを足したら、そのぶん横に伸ばす
        let o = Opt {
            sx: (t.w + self.looks.layout.extra_w()) / t.w,
            ..Default::default()
        };
        if top > 0.0 {
            c.draw_opt(&t, LANES_LEFT, (-720.0 + top * 7.2).floor(), o);
        }
        if bottom > 0.0 {
            c.draw_opt(&t, LANES_LEFT, (720.0 - bottom * 7.2).floor(), o);
        }
    }

    /// 進み具合のバー（上流 `CActPerfProgressBar`）
    fn draw_progress(&self, c: &Canvas, g: &Gfx, play: &Play, now_ms: f64) {
        // 左バスドラムのレーンを足したら、そのぶん右へ
        let x = PROGRESS_X + self.looks.layout.extra_w();
        const Y: f32 = 15.0;
        const W: f32 = 20.0;
        const H: f32 = 540.0;
        if let Some(t) = g.skin.tex("Graphics/7_Drum_Progress_bg.png") {
            c.draw(&t, x - 2.0, Y - 15.0);
        }
        // 5 ドットごとの縞
        c.fill(x, Y, W + 2.0, H, Color32::from_rgb(10, 10, 10));
        let mut y = 5.0;
        while y < H {
            c.fill(x, Y + y, W + 2.0, 5.0, Color32::from_rgb(14, 14, 14));
            y += 10.0;
        }
        c.fill(x + W, Y, 1.0, H, Color32::from_white_alpha(64));
        c.fill(x + W + 1.0, Y, 1.0, H, Color32::from_white_alpha(32));
        if play.practice.is_some() || play.last_chip_ms <= 0.0 {
            return;
        }
        let done = ((now_ms / play.last_chip_ms) as f32 * H).clamp(0.0, H);
        if done > 0.0 {
            c.fill(x, Y + H - done, W, done, Color32::from_white_alpha(48));
            c.fill(
                x,
                Y + H - done,
                W,
                done.min(8.0),
                Color32::from_white_alpha(128),
            );
        }
        // 1 区間の幅は音符の数で 1〜10 ブロック（上流の基準は 1600 音符）
        let per_block = 1600.0 / 10.0 / SECTIONS as f64;
        let mut below = H;
        for (i, s) in self.sections.iter().enumerate() {
            let top = (H - (i as f32 + 1.0) * H / SECTIONS as f32).round();
            let h = below - top;
            below = top;
            let blocks = ((f64::from(s.notes) / per_block) as u32 + 1).min(10);
            let w = blocks as f32 * 2.0;
            let color = if section_end_ms(i, play.last_chip_ms) > now_ms {
                Color32::from_white_alpha(64)
            } else if s.notes == 0 {
                continue;
            } else if s.hits >= s.notes {
                Color32::from_rgba_unmultiplied(255, 255, 0, 192)
            } else {
                Color32::from_rgb(0, 191, 255)
            };
            c.fill(x, Y + top, w, h, color);
        }
    }

    /// パッド（上流 `CActPerfDrumsPad`）
    fn draw_pads(&mut self, c: &Canvas, g: &Gfx, now: Instant) {
        for p in &mut self.pads {
            p.update(now);
        }
        let pads = g.skin.tex("Graphics/7_pads.png");
        let flush = g.skin.tex("Graphics/ScreenPlayDrums pads flush.png");
        for l in PAD_ORDER {
            if !self.looks.layout.shows(l) {
                continue;
            }
            let x = self.looks.layout.pad[l.i()];
            let (sx, sy) = PADS[l.i()];
            let st = self.pads[l.i()];
            let y = if self.looks.reverse { 70.0 } else { 570.0 } + st.dip as f32;
            let s = src(sx, sy, 96.0, 96.0);
            if let Some(t) = &pads {
                c.part(t, x, y, s);
            }
            if let Some(t) = &flush
                && st.bright > 0
            {
                c.draw_opt(
                    t,
                    x,
                    y,
                    Opt {
                        src: Some(s),
                        alpha: (st.bright * 50) as f32 / 255.0,
                        ..Default::default()
                    },
                );
            }
        }
    }

    /// スコア（上流 `CActPerfDrumsScore`）
    fn draw_score(&mut self, c: &Canvas, g: &Gfx, now: Instant, practice: bool) {
        // 練習ではスコアを出さない（その場所に練習の欄を出す）
        if practice {
            return;
        }
        let s = &mut self.score;
        const STEP: Duration = Duration::from_millis(10);
        while now.saturating_duration_since(s.at) >= STEP && (s.shown as f64) < s.truth {
            s.shown = (s.shown + s.step).min(s.truth as i64);
            s.at += STEP;
        }
        if let Some(t) = g.skin.tex("Graphics/7_score numbersGD.png") {
            draw_score_digits(c, &t, 40.0, 41.0, 13.0, s.shown);
        }
    }

    /// STAGE FAILED（上流 `CActPerfStageFailure`：7_stage_failed.jpg の左右の半分が真ん中へ閉じる）
    fn draw_failed(&self, c: &Canvas, g: &Gfx, now: Instant) {
        let Some((at, _)) = self.failed else {
            return;
        };
        let Some(t) = g.skin.tex("Graphics/7_stage_failed.jpg") else {
            c.cover(Color32::from_black_alpha(200));
            return;
        };
        let v = now.saturating_duration_since(at).as_secs_f64() * 1000.0 / FAIL_CLOSE;
        if v < 1.0 {
            let x = (640.0 * (std::f64::consts::FRAC_PI_2 * v).cos()) as f32;
            c.part(&t, 0.0, 0.0, src(x, 0.0, 640.0 - x, 720.0));
            c.part(&t, 640.0 + x, 0.0, src(640.0, 0.0, 640.0 - x, 720.0));
        } else {
            c.draw(&t, 0.0, 0.0);
        }
    }

    /// 左のパネル（上流 `CActPerfDrumsStatusPanel`）
    fn draw_status(&self, c: &Canvas, g: &mut Gfx, play: &Play, practice: bool) {
        draw_skill_panel(
            c,
            g,
            (22.0, 250.0),
            &PanelNumbers {
                tally: &self.tally,
                rate: self.rate(practice),
                level: play.level.unwrap_or(0.0),
                diff_panel: self.diff_panel,
            },
            &LARGE,
        );
    }

    /// ゲージ（上流 `CActPerfDrumsGauge`。Reverse では上の端）
    fn draw_gauge(&self, c: &Canvas, g: &Gfx, px_per_ms: f32) {
        const X: f32 = 294.0;
        let top = if self.looks.reverse { 28.0 } else { 626.0 };
        let Some(frame) = g.skin.tex("Graphics/7_Gauge.png") else {
            return;
        };
        c.part(&frame, X, top, src(0.0, 0.0, frame.w, 47.0));
        // 落ちる速さのアイコン（上流の速さの段 n は 0.089375 × (n + 1) ドット/ミリ秒）
        if let Some(t) = g.skin.tex("Graphics/7_panel_icons.jpg") {
            // 段は 21 まで。NX v2.0 の絵は 16 段しか無いので、絵にある一番下の段で止める
            let rows = (t.h / 48.0).floor().max(1.0);
            let n = (px_per_ms / 0.089375 - 1.0)
                .round()
                .clamp(0.0, (rows - 1.0).min(20.0));
            c.draw_opt(
                &t,
                -37.0 + X + frame.w,
                top + if self.looks.reverse { 7.0 } else { 8.0 },
                Opt {
                    src: Some(src(0.0, n * 48.0, 42.0, 48.0)),
                    sx: 0.761_904_8,
                    sy: 2.0 / 3.0,
                    ..Default::default()
                },
            );
        }
        let bar = src(0.0, 0.0, frame.w - 63.0, 31.0);
        if self.gauge.value >= 1.0
            && let Some(t) = g.skin.tex("Graphics/7_gauge_bar.jpg")
        {
            c.part(&t, X + 20.0, top + 9.0, bar);
        } else if let Some(t) = g.skin.tex("Graphics/7_gauge_bar.png") {
            c.draw_opt(
                &t,
                X + 20.0,
                top + 9.0,
                Opt {
                    src: Some(bar),
                    sx: self.gauge.value.max(0.0) as f32,
                    ..Default::default()
                },
            );
        }
        c.part(&frame, X, top, src(0.0, 47.0, frame.w, 47.0));
    }

    /// コンボ（上流 `CActPerfCommonCombo.tDrawCombo_Drums` を右上 (1245, 60) に。10 コンボから出す）
    fn draw_combo(&mut self, c: &Canvas, g: &Gfx, play: &Play, now: Instant) {
        // 上流 `bドラムコンボ文字の表示`。切ったら 100 コンボごとの爆発も出さない
        if !self.looks.combo {
            return;
        }
        let combo = self.tally.combo;
        // 左バスドラムのレーンを足して右へ寄った進み具合のバーに、コンボの数字がかからないように
        let shift = if self.looks.layout.left_bass_lane {
            COMBO_SHIFT
        } else {
            0.0
        };
        // 100 コンボごとの爆発（上流 `CActPerfDrumsComboDGB`）
        let hundred = combo / 100;
        if combo >= 100 && combo > hundred + 100 && !self.bombed.contains(&hundred) {
            self.bombed.push(hundred);
            self.bomb = Some((hundred, now));
        }
        if let Some((_, at)) = self.bomb {
            let f = count(at, now, 20.0);
            if f >= 14.0 || combo < 100 {
                self.bomb = None;
            } else if let Some(t) = g.skin.tex("Graphics/7_combobomb.png") {
                c.draw_opt(
                    &t,
                    845.0 + shift,
                    -130.0,
                    Opt {
                        src: Some(src(0.0, f as f32 * 340.0, 360.0, 340.0)),
                        sx: 1.5,
                        sy: 1.5,
                        add: true,
                        ..Default::default()
                    },
                );
            }
        }
        if combo < self.looks.min_combo.max(1) {
            return;
        }
        let Some(t) = g.skin.tex("Graphics/ScreenPlayDrums combo.png") else {
            return;
        };
        // 拍に合わせて少し沈む（上流 `ctComboTimer`：1 拍を 16 に分けた最初の 3 つは 8、ほかは 2）
        let sixteenth = 60_000.0 / play.bpm.max(1.0) / 16.0;
        let tick = count(self.entered, now, sixteenth) as u64 % 16 + 1;
        let pulse = if tick < 4 { 8.0 } else { 2.0 };
        // 跳ねる（2 ミリ秒ごとに 3 進み、180 までの sin）
        let jump_index = self
            .combo_jump
            .map_or(i64::MAX, |at| count(at, now, 2.0) as i64 * 3);
        let jump = |i: i64| {
            if (0..180).contains(&i) {
                (-15.0 * (std::f64::consts::PI * i as f64 / 180.0).sin()) as i32 as f32
            } else {
                0.0
            }
        };
        c.part(
            &t,
            985.0 + shift,
            222.0 + pulse,
            src(0.0, 320.0, 250.0, 60.0),
        );
        let digits: Vec<u32> = combo
            .to_string()
            .chars()
            .rev()
            .filter_map(|ch| ch.to_digit(10))
            .collect();
        let n = digits.len() as i64;
        let big = g.skin.tex("Graphics/ScreenPlayDrums combo_2.png");
        let mut x = 1245.0 + shift;
        for (i, &d) in digits.iter().enumerate() {
            let j = jump(jump_index - (n - i as i64 - 1));
            if n < 4 {
                x -= 120.0 - 6.0;
                c.part(
                    &t,
                    x,
                    60.0 + j + pulse,
                    src((d % 5) as f32 * 120.0, (d / 5) as f32 * 160.0, 120.0, 160.0),
                );
            } else if let Some(b) = &big {
                x -= 96.0 - 6.0;
                c.part(
                    b,
                    x,
                    80.0 + j + pulse,
                    src((d % 5) as f32 * 96.0, (d / 5) as f32 * 128.0, 96.0, 128.0),
                );
            }
        }
    }

    /// 判定の文字とずれ（上流 `CActPerfDrumsJudgementString` のコマ送り：24 コマを 14 ミリ秒ごと）
    fn draw_judges(&mut self, c: &Canvas, g: &Gfx, now: Instant) {
        // 上流 `JudgementStringPosition`：レーンの上は 348、判定ラインの下は 583（Reverse では上の 80）を基準に、
        // レーンごとの段をずらす。Reverse のレーンの上は段を逆に
        let reverse = self.looks.reverse;
        let (base, step) = match (self.looks.judge_position, reverse) {
            (JudgePosition::Lane, false) => (348, 32),
            (JudgePosition::Lane, true) => (348, -32),
            (JudgePosition::Below, false) => (583, 32),
            (JudgePosition::Below, true) => (80, 32),
            (JudgePosition::Off, _) => {
                self.judges = [None; LANES];
                return;
            }
        };
        let strings = g.skin.tex("Graphics/7_judge strings.png");
        let lag = g.skin.tex("Graphics/7_lag numbers.png");
        let digits = self.lag_digits;
        for (i, slot) in self.judges.iter_mut().enumerate() {
            let Some(j) = *slot else { continue };
            let frame = count(j.start, now, 14.0);
            if frame >= 23.0 {
                *slot = None;
                continue;
            }
            let (lx, lw) = self.looks.layout.judge[i];
            let row = JUDGE_ROW[i];
            let xc = lx + lw / 2;
            let x = (xc - 122) as f32;
            let y = (base + row * step - 87) as f32;
            if let Some(t) = &strings {
                c.part(
                    t,
                    x,
                    y,
                    src(
                        judge_column(j.judgement) * 250.0,
                        frame as f32 * 170.0,
                        250.0,
                        170.0,
                    ),
                );
            }
            if let (Some(t), Some(ms)) = (&lag, j.lag)
                && digits.shows(j.judgement)
            {
                let text = ms.to_string();
                let mut x = (xc - text.len() as i32 * 15 / 2) as f32;
                for ch in text.chars() {
                    let p = if ch == '-' {
                        11
                    } else {
                        ch.to_digit(10).unwrap_or(0) as i32
                    };
                    // 早い（マイナス）は左上の色、遅い（プラス）は右下の色の数字（入れ替えの設定なら逆）
                    let (ox, oy) = if (ms < 0) != digits.swapped {
                        (0.0, 0.0)
                    } else {
                        (64.0, 64.0)
                    };
                    let s = src(
                        ox + (p % 4) as f32 * 15.0,
                        oy + (p / 4) as f32 * 19.0,
                        15.0,
                        19.0,
                    );
                    c.part(t, x, y + 34.0, s);
                    x += 15.0;
                }
            }
        }
    }

    /// 火花・星・飛び散るチップ（上流 `CActPerfDrumsChipFireD`）
    fn draw_fire(&mut self, c: &Canvas, g: &Gfx, now: Instant) {
        // 星：7 ミリ秒ごとに 40 コマ
        self.stars.retain_mut(|s| {
            let v = count(s.start, now, 7.0).min(40.0) as u32;
            while s.steps < v {
                s.x += s.ax;
                s.y -= s.ay;
                s.ay *= 1.01;
                s.ay -= 0.0204;
                s.steps += 1;
            }
            if v >= 40 {
                return false;
            }
            let rel = format!(
                "Graphics/ScreenPlayDrums chip star_{}.png",
                FIRE[s.lane.i()]
            );
            let tex = if s.lane == Lane::LBD {
                left_bass_tinted_tex(g, &rel)
            } else {
                g.skin.tex(&rel)
            };
            if let Some(t) = tex {
                let f = s.radius * (std::f32::consts::FRAC_PI_2 * v as f32 / 100.0).cos();
                c.draw_rot(
                    &t,
                    s.x,
                    s.y,
                    0.0,
                    Opt {
                        sx: f,
                        sy: f,
                        add: true,
                        ..Default::default()
                    },
                );
            }
            true
        });
        // 飛び散るチップ：10 ミリ秒ごとに 44 コマ、半透明（120/255）
        let chips = g.skin.tex("Graphics/7_chips_drums.png");
        let left_bass = self
            .pieces
            .iter()
            .any(|p| p.left_bass)
            .then(|| left_bass_tex(g))
            .flatten();
        self.pieces.retain_mut(|p| {
            let v = count(p.start, now, 10.0).min(44.0) as u32;
            let gravity = (0.031_f32 * 2.0).exp() / 2.0;
            while p.steps < v {
                p.xl += p.ax * (120.0_f32).to_radians().cos() * 5.0;
                p.xr += p.ax * (60.0_f32).to_radians().cos() * 5.0;
                p.y += p.ay * (60.0_f32).to_radians().sin() * 10.0 - gravity;
                p.ax *= 0.995;
                p.ay += 0.031;
                p.steps += 1;
            }
            if v >= 44 {
                return false;
            }
            let tex = if p.left_bass { &left_bass } else { &chips };
            if let Some(t) = tex {
                let (left, w) = PIECES[p.lane.i()];
                let o = Opt {
                    src: Some(src(left, 640.0, ((w + 10.0) / 2.0).floor(), 64.0)),
                    alpha: 120.0 / 255.0,
                    ..Default::default()
                };
                let turn = 0.09 * v as f32;
                c.draw_rot(t, p.xl - 50.0, p.y, turn, o);
                c.draw_rot(t, p.xr - 50.0, p.y, -turn, o);
            }
            true
        });
        // 火花：3 ミリ秒ごとに 70 コマ、加算で 2 枚重ね
        for (i, slot) in self.fires.iter_mut().enumerate() {
            let Some(at) = *slot else { continue };
            let v = count(at, now, 3.0);
            if v > 70.0 {
                *slot = None;
                continue;
            }
            let rel = format!("Graphics/ScreenPlayDrums chip fire_{}.png", FIRE[i]);
            let tex = if i == Lane::LBD.i() {
                left_bass_tinted_tex(g, &rel)
            } else {
                g.skin.tex(&rel)
            };
            let Some(t) = tex else {
                continue;
            };
            let f = 0.4 + 0.8 * (v / 50.0 * std::f64::consts::FRAC_PI_2).cos() as f32;
            let o = Opt {
                sx: f,
                sy: f,
                add: true,
                ..Default::default()
            };
            let x = self.looks.layout.fire[i] + 320.0;
            let y = self.looks.fire_y() + 190.0;
            c.draw_rot(&t, x, y, 0.0, o);
            c.draw_rot(&t, x, y, 0.0, o);
        }
    }
}

/// ゲージがここまで減ったら STAGE FAILED（上流 `GAUGE_MIN`。0 を少し下回るまでは耐える）
const GAUGE_MIN: f64 = -0.1;
/// STAGE FAILED の画像が閉じきるまで（上流は 2 ミリ秒 × 100）
const FAIL_CLOSE: f64 = 200.0;
/// STAGE FAILED を出しておく時間（上流は 2 ミリ秒 × 1000）
const FAIL_FOR: Duration = Duration::from_millis(2000);

/// 7_SkillPanel.png に出す数字
pub struct PanelNumbers<'a> {
    pub tally: &'a Tally,
    /// 達成率（0〜100）
    pub rate: f64,
    pub level: f64,
    /// 7_Difficulty.png の切り出しの左上
    pub diff_panel: (f32, f32),
}

/// 7_SkillPanel.png と、判定の数と割合、達成率、スキル、難易度とレベル。
/// 演奏の画面（上流 `CActPerfDrumsStatusPanel`）と結果の画面（`CActResultParameterPanel`）は
/// 置く場所と大きい数字の送りだけが違う
pub fn draw_skill_panel(
    c: &Canvas,
    g: &mut Gfx,
    (x, y): (f32, f32),
    n: &PanelNumbers<'_>,
    large: &Glyphs<'_>,
) {
    if let Some(t) = g.skin.tex("Graphics/7_SkillPanel.png") {
        c.draw(&t, x, y);
    }
    // 名前（上流は Config.ini の名前、無ければ GUEST）
    if let Some(t) = g.texts.get("GUEST", &Style::plain(20.0, Color32::WHITE)) {
        t.draw(c, x - 2.0, y + 26.0, 1.0, 1.0);
    }
    let small = g.skin.tex("Graphics/7_Ratenumber_s.png");
    let large_tex = g.skin.tex("Graphics/7_Ratenumber_l.png");
    let t = n.tally;
    let judged = t.perfect + t.great + t.good + t.poor + t.miss;
    // 割合は判定した数に対して（練習でも、ここまでの周の割合になる）
    let percent = |k: u32| {
        if judged == 0 {
            0.0
        } else {
            (100.0 * f64::from(k) / f64::from(judged)).round()
        }
    };
    let rows = [t.perfect, t.great, t.good, t.poor, t.miss, t.max_combo];
    for (k, count) in rows.into_iter().enumerate() {
        let ry = y + 72.0 + k as f32 * 30.0;
        SMALL.draw(c, small.as_ref(), x + 80.0, ry, &format!("{count:>4}"));
        SMALL.draw(
            c,
            small.as_ref(),
            x + 167.0,
            ry,
            &format!("{:>3}%", percent(count)),
        );
    }
    if n.rate >= 100.0 {
        if let Some(t) = g.skin.tex("Graphics/7_skill max.png") {
            c.draw(&t, x + 127.0, y + 277.0);
        }
    } else {
        large.draw(
            c,
            large_tex.as_ref(),
            x + 58.0,
            y + 277.0,
            &format!("{:>6.2}", n.rate),
        );
        if let Some(t) = g.skin.tex("Graphics/7_RatePercent_l.png") {
            c.draw(&t, x + 217.0, y + 287.0);
        }
    }
    let skill = n.rate * n.level * 0.2;
    large.draw(
        c,
        large_tex.as_ref(),
        x + 88.0,
        y + 363.0,
        &format!("{skill:>6.2}"),
    );
    if let Some(t) = g.skin.tex("Graphics/7_Difficulty.png") {
        let (sx, sy) = n.diff_panel;
        c.part(&t, x + 14.0, y + 266.0, src(sx, sy, 60.0, 60.0));
    }
    let digits = g.skin.tex("Graphics/7_LevelNumber.png");
    LEVEL.draw(
        c,
        digits.as_ref(),
        x + 18.0,
        y + 290.0,
        &format!("{:.2}", n.level),
    );
}

/// 7_score numbersGD.png のスコア（7 桁の右寄せ）と「SCORE」の文字
pub fn draw_score_digits(c: &Canvas, t: &Tex, x: f32, y: f32, label_y: f32, score: i64) {
    for (i, ch) in format!("{score:>7}").chars().enumerate() {
        if let Some(d) = ch.to_digit(10) {
            c.part(
                t,
                x + i as f32 * 34.0,
                y,
                src(d as f32 * 36.0, 0.0, 36.0, 50.0),
            );
        }
    }
    c.part(t, x, label_y, src(0.0, 50.0, 86.0, 28.0));
}

/// XG のスコアの満点
const MAX_SCORE: f64 = 1_000_000.0;

/// XG のスコアの増え方（上流 `tProcessChipHit` の「100万/{1275+50×(総ノーツ数-50)}」）。
/// `tally` は今の判定を数えたあと、`truth` は今までの本当の点数。
/// 上流は音符が 50 より少ない曲で 100 万を超えるので、100 万で止める
fn score_delta(j: Judgement, tally: &Tally, notes: usize, truth: f64) -> f64 {
    let n = notes as f64;
    let base = MAX_SCORE / (1275.0 + 50.0 * (n - 50.0));
    let combo = f64::from(tally.combo);
    let all_perfect = f64::from(tally.perfect) >= n;
    let delta = match j {
        Judgement::Perfect if combo < n => base,
        // 最後まで全部 PERFECT なら 100 万に合わせる
        Judgement::Perfect if all_perfect => return (MAX_SCORE - truth).max(0.0),
        Judgement::Great => base * 0.5,
        Judgement::Good => base * 0.2,
        _ => 0.0,
    };
    let delta = if combo < 50.0 {
        delta * combo
    } else if combo != n && !all_perfect {
        delta * 50.0
    } else {
        delta
    };
    delta.trunc().min(MAX_SCORE - truth).max(0.0)
}

/// ゲージ（上流 `CActPerfCommonGauge`）
#[derive(Debug, Clone, Copy, PartialEq)]
struct Gauge {
    /// 0〜1（尽きるのは `GAUGE_MIN`）
    value: f64,
    rule: GaugeRule,
    /// Risky の残りの回数
    risky_left: u32,
}

impl Gauge {
    /// 上流 `Init`。ふつうは `GAUGE_INITIAL`（2/3）、Risky は満タン（1 回なら空）から
    fn new(rule: GaugeRule) -> Self {
        Self {
            value: match rule.risky {
                0 => 2.0 / 3.0,
                1 => 0.0,
                _ => 1.0,
            },
            rule,
            risky_left: rule.risky,
        }
    }

    /// 上流 `IsFailed`
    fn empty(&self) -> bool {
        if self.rule.risky > 0 {
            self.risky_left == 0
        } else {
            self.value <= GAUGE_MIN
        }
    }

    /// 判定でゲージを動かす（上流 `Damage`）
    fn apply(&mut self, j: Judgement) {
        let rule = self.rule;
        let hazard = rule.hazard && matches!(j, Judgement::Great | Judgement::Good);
        let bad = hazard || matches!(j, Judgement::Poor | Judgement::Miss);
        let delta = if rule.risky > 0 {
            // Risky はゲージが増えず、ミスのたびに同じだけ減る。最後の 1 回は減らさずに尽きる
            if bad {
                let d = if self.risky_left <= 1 {
                    0.0
                } else {
                    -1.0 / f64::from(rule.risky - 1)
                };
                self.risky_left = self.risky_left.saturating_sub(1);
                d
            } else {
                0.0
            }
        } else if hazard {
            // HAZARD の GREAT・GOOD は MISS の減り（ゲージの減り方はかけない）
            gauge_delta(Judgement::Miss)
        } else if j == Judgement::Miss {
            gauge_delta(j) * rule.miss_factor
        } else {
            gauge_delta(j)
        };
        self.value = (self.value + delta).clamp(GAUGE_MIN, 1.0);
    }
}

/// XG のゲージの増減（上流 `CActPerfCommonGauge.Damage`。MISS にはゲージの減り方の数をかけて使う）
fn gauge_delta(j: Judgement) -> f64 {
    match j {
        Judgement::Perfect => 0.005,
        Judgement::Great => 0.001,
        Judgement::Good => 0.0,
        Judgement::Poor => -0.017,
        Judgement::Miss => -0.041,
    }
}

/// 時刻が進み具合のバーのどの区間か
fn section_of(ms: f64, last_ms: f64) -> usize {
    if last_ms <= 0.0 {
        return 0;
    }
    ((ms * SECTIONS as f64 / last_ms).max(0.0) as usize).min(SECTIONS - 1)
}

/// 区間 `i` の終わりの時刻（上流は「(i + 1) × 最後のチップの時刻 / 64 − 1」を過ぎたらたどり着いたとする）
fn section_end_ms(i: usize, last_ms: f64) -> f64 {
    (i as f64 + 1.0) * last_ms / SECTIONS as f64 - 1.0
}

/// 進み具合のバーの文字列（上流 `GetScoreIniString`）。
/// '0' はたどり着いていない、'1' は取りこぼしあり、'2' は全部叩けた、'3' は音符なし
fn progress_text(sections: &[Section], last_ms: f64, now_ms: f64) -> String {
    sections
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if section_end_ms(i, last_ms) > now_ms {
                '0'
            } else if s.notes == 0 {
                '3'
            } else if s.hits >= s.notes {
                '2'
            } else {
                '1'
            }
        })
        .collect()
}

/// 画面の中心を原点に y を上にした座標 (x, y) を、原点のまわりに `angle` 回して、画面の座標にする（上流の `tDraw3D`）
pub fn rotate_about_center(x: f32, y: f32, angle: f32) -> (f32, f32) {
    let (sin, cos) = angle.sin_cos();
    let rx = x * cos - y * sin;
    let ry = x * sin + y * cos;
    (640.0 + rx, 360.0 - ry)
}

/// スキンの Script/difficult.dtxs から、場面 `scene` の難易度の名前 `label` の画像の切り出しの左上を引く
/// （上流 `tSetDifficultyLabelFromScript`。行は「場面,名前,x,y[,大文字小文字を区別するなら 1]」）
pub fn difficulty_cell(scene: &str, label: &str) -> Option<(f32, f32)> {
    let bytes = crate::skin::read("Script/difficult.dtxs")?;
    let (text, _) = dtx::decode(&bytes);
    parse_difficulty(&text, scene, label)
}

fn parse_difficulty(text: &str, scene: &str, label: &str) -> Option<(f32, f32)> {
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with("//") {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if !(4..=5).contains(&f.len()) || f[0] != scene {
            continue;
        }
        let same = if f.len() == 5 && f[4] == "1" {
            f[1] == label
        } else {
            f[1].eq_ignore_ascii_case(label)
        };
        if !same {
            continue;
        }
        return Some((f[2].trim().parse().ok()?, f[3].trim().parse().ok()?));
    }
    None
}

/// 7_Ratenumber_s.png の文字（上流 `st小文字位置`：20×26、送りは 20）
static SMALL: LazyLock<Glyphs<'static>> = LazyLock::new(|| Glyphs {
    table: Box::leak(
        glyph_table(20.0, 26.0, 20.0, &[('%', 200.0), ('.', 210.0)], 20.0, 20.0).into_boxed_slice(),
    ),
    missing: 20.0,
});
/// 7_Ratenumber_l.png の文字（上流 `st大文字位置`：28×42、送りは 29、点は幅 10 で送り 12）
static LARGE: LazyLock<Glyphs<'static>> = LazyLock::new(|| Glyphs {
    table: Box::leak(glyph_table(28.0, 42.0, 29.0, &[('.', 280.0)], 10.0, 12.0).into_boxed_slice()),
    missing: 29.0,
});
/// 結果の画面の 7_Ratenumber_l.png の文字（上流 `CActResultParameterPanel.tDrawStringLarge`：送りは 28）
pub static LARGE_RESULT: LazyLock<Glyphs<'static>> = LazyLock::new(|| Glyphs {
    table: Box::leak(glyph_table(28.0, 42.0, 28.0, &[('.', 280.0)], 10.0, 12.0).into_boxed_slice()),
    missing: 28.0,
});
/// 7_LevelNumber.png の文字（上流 `st難易度数字位置`：16×32、点は幅 5 で送り 5）
static LEVEL: LazyLock<Glyphs<'static>> = LazyLock::new(|| Glyphs {
    table: Box::leak(glyph_table(16.0, 32.0, 16.0, &[('.', 160.0)], 5.0, 5.0).into_boxed_slice()),
    missing: 16.0,
});

/// 0〜9 が `w` ずつ並んだ数字の表を作る。`extra` はほかの文字とその x（幅 `extra_w`、送り `extra_adv`）
fn glyph_table(
    w: f32,
    h: f32,
    adv: f32,
    extra: &[(char, f32)],
    extra_w: f32,
    extra_adv: f32,
) -> Vec<(char, Src, f32)> {
    let mut out: Vec<(char, Src, f32)> = (0..10u8)
        .map(|d| (char::from(b'0' + d), src(f32::from(d) * w, 0.0, w, h), adv))
        .collect();
    for &(ch, x) in extra {
        // 上流の小さい数字は % と点も 20 の幅で切り出す
        let ew = if ch == '.' { extra_w } else { w };
        out.push((ch, src(x, 0.0, ew, h), extra_adv));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jacket_center_matches_upstream_transform() {
        // 上流：Translation(400, -227) のあと RotationZ(0.3)
        let (x, y) = rotate_about_center(400.0, -227.0, 0.3);
        assert!((x - 1089.2).abs() < 0.5, "{x}");
        assert!((y - 458.7).abs() < 0.5, "{y}");
    }

    #[test]
    fn difficulty_script_lookup() {
        let s = "// comment\n7,BASIC,0,360\n7,Master,0,300,1\n6,BASIC,0,50\n";
        assert_eq!(parse_difficulty(s, "7", "basic"), Some((0.0, 360.0)));
        assert_eq!(parse_difficulty(s, "6", "BASIC"), Some((0.0, 50.0)));
        // 5 つ目が 1 なら大文字小文字を区別する
        assert_eq!(parse_difficulty(s, "7", "MASTER"), None);
        assert_eq!(parse_difficulty(s, "7", "Master"), Some((0.0, 300.0)));
        assert_eq!(parse_difficulty(s, "7", "EXTREME"), None);
    }

    #[test]
    fn pad_dips_and_comes_back() {
        let t0 = Instant::now();
        let mut p = PadState::new(t0);
        p.hit(t0);
        p.update(t0 + Duration::from_millis(48));
        // 8 ミリ秒ごとに 2 ずつ沈み、11 で戻り始める
        assert_eq!(p.dip, 11);
        assert_eq!(p.dip_speed, -1);
        p.update(t0 + Duration::from_millis(48 + 8 * 12));
        assert_eq!((p.dip, p.dip_speed), (0, 0));
        // 明るさは 18 ミリ秒ごとに 1 下がる
        assert_eq!(p.bright, 0);
    }

    #[test]
    fn sections_split_the_song_into_64() {
        assert_eq!(section_of(0.0, 64_000.0), 0);
        assert_eq!(section_of(1_000.0, 64_000.0), 1);
        assert_eq!(section_of(64_000.0, 64_000.0), 63);
        assert_eq!(section_of(-5.0, 64_000.0), 0);
    }

    #[test]
    fn lag_digits_follow_the_config() {
        let mut c = crate::config::Config::default();
        let on = LagDigits::from_config(&c);
        assert_eq!(on.show, LagShow::On);
        assert!(!on.swapped);
        c.show_lag_time = 0;
        assert_eq!(LagDigits::from_config(&c).show, LagShow::Off);
        c.show_lag_time = 2;
        c.show_lag_time_color = 1;
        let d = LagDigits::from_config(&c);
        assert_eq!(d.show, LagShow::NotPerfect);
        assert!(d.swapped);
        assert!(!d.shows(Judgement::Perfect));
        assert!(d.shows(Judgement::Great));
    }

    /// 全部 PERFECT で叩いたときのスコア
    fn all_perfect_score(notes: usize) -> f64 {
        let mut t = Tally::default();
        let mut truth = 0.0;
        for _ in 0..notes {
            t.add(Judgement::Perfect);
            truth = (truth + score_delta(Judgement::Perfect, &t, notes, truth)).min(MAX_SCORE);
        }
        truth
    }

    #[test]
    fn hidden_and_sudden_fade_chips_like_upstream() {
        let looks = |hid_sud| Looks {
            hid_sud,
            ..Looks::default()
        };
        let off = looks(HidSud::Off);
        assert_eq!(off.chip_alpha(500.0), Some(1.0));
        assert_eq!(off.chip_alpha(-30.0), Some(1.0));
        let sudden = looks(HidSud::Sudden);
        assert_eq!(sudden.chip_alpha(260.0), None);
        assert_eq!(sudden.chip_alpha(225.0), Some(0.5));
        assert_eq!(sudden.chip_alpha(50.0), Some(1.0));
        let hidden = looks(HidSud::Hidden);
        assert_eq!(hidden.chip_alpha(500.0), Some(1.0));
        assert_eq!(hidden.chip_alpha(125.0), Some(0.5));
        assert_eq!(hidden.chip_alpha(90.0), None);
        let both = looks(HidSud::Both);
        assert_eq!(both.chip_alpha(300.0), None);
        assert_eq!(both.chip_alpha(175.0), Some(1.0));
        assert_eq!(both.chip_alpha(50.0), None);
        assert_eq!(looks(HidSud::Stealth).chip_alpha(175.0), None);
    }

    #[test]
    fn looks_follow_the_settings() {
        use crate::settings as s;
        let mut v = s::Values::default();
        let d = Looks::from_settings(&v);
        assert!(d.bar_lines && !d.dark_lanes && d.judge_line && d.lane_flush && d.progress);
        assert_eq!(d.judge_y(), JUDGE_LINE_Y);
        assert_eq!(d.min_combo, 10);
        v.set_raw(s::DARK, 2);
        v.set_raw(s::JUDGE_LINE_POS, 40);
        let full = Looks::from_settings(&v);
        assert!(!full.bar_lines && full.dark_lanes && !full.judge_line && !full.progress);
        assert_eq!(full.judge_y(), JUDGE_LINE_Y - 40.0);
        v.set_raw(s::LANE_DISP, 2);
        let line_off = Looks::from_settings(&v);
        assert!(!line_off.bar_lines && !line_off.dark_lanes);
    }

    #[test]
    fn left_bass_goes_to_the_bd_lane_by_default() {
        use crate::settings as s;
        let mut v = s::Values::default();
        let d = Looks::from_settings(&v);
        assert_eq!(d.lbd, LbdLook::BdLane);
        assert!(!d.layout.left_bass_lane);
        assert_eq!(d.lane_of(DrumLane::LBD), Lane::BD);
        assert_eq!(d.lane_of(DrumLane::LP), Lane::LP);
        // BD と同じ x と切り出し（絵は色を変えたもの）
        assert_eq!(chip(&d, DrumLane::LBD), chip(&d, DrumLane::BD));
        assert!(d.left_bass_tinted(DrumLane::LBD) && !d.left_bass_tinted(DrumLane::BD));
        // LP の右に足したレーン。絵は水色の絵の LP の列から
        v.set_raw(s::LBD_GRAPHICS, 1);
        let own = Looks::from_settings(&v);
        assert_eq!(own.lbd, LbdLook::OwnLane);
        assert!(own.layout.left_bass_lane);
        assert_eq!(own.lane_of(DrumLane::LBD), Lane::LBD);
        let (x, cut, w, _) = chip(&own, DrumLane::LBD);
        assert_eq!(
            (x, cut, w),
            (own.layout.chip[Lane::LBD.i()], LEFT_BASS_LP_SX, 58.0)
        );
        assert!(x > own.layout.chip[Lane::LP.i()]);
        assert!(own.left_bass_tinted(DrumLane::LBD));
        // NX の A と B は LP のレーン
        for (raw, look, sx) in [(2, LbdLook::SameAsLp, 660.0), (3, LbdLook::Own, 480.0)] {
            v.set_raw(s::LBD_GRAPHICS, raw);
            let nx = Looks::from_settings(&v);
            assert_eq!(nx.lbd, look);
            assert!(!nx.layout.left_bass_lane);
            assert_eq!(nx.lane_of(DrumLane::LBD), Lane::LP);
            let (x, cut, _, _) = chip(&nx, DrumLane::LBD);
            assert_eq!((x, cut), (nx.layout.chip[Lane::LP.i()], sx));
            assert!(!nx.left_bass_tinted(DrumLane::LBD));
        }
    }

    #[test]
    fn left_bass_chips_are_the_bd_chips_flipped_and_recolored() {
        let mut src = image::RgbaImage::new(718, 4);
        // BD の列の左端に淡い青紫、LP の列の左端にピンク。ほかの列は使わない
        src.put_pixel(0, 1, image::Rgba([150, 130, 255, 200]));
        src.put_pixel(100, 1, image::Rgba([255, 0, 0, 255]));
        src.put_pixel(660, 1, image::Rgba([255, 80, 170, 255]));
        let out = left_bass_chips(src);
        assert_eq!((out.width(), out.height()), (70 + 58, 4));
        // BD の列は左右に返したので右端へ。透明さはそのまま、色相は LEFT_BASS_HUE、明るさは元のまま
        let p = out.get_pixel(69, 1);
        assert_eq!(p[3], 200);
        let (h, s, v) = rgb_to_hsv(p[0], p[1], p[2]);
        assert!((h - LEFT_BASS_HUE).abs() < 2.0, "{h}");
        assert!(s > 0.4 && (v - 1.0).abs() < 0.01, "{s} {v}");
        assert_eq!(out.get_pixel(0, 1)[3], 0);
        // LP の列は返さずにその後ろへ
        let p = out.get_pixel(70, 1);
        assert_eq!(p[3], 255);
        let (h, _, _) = rgb_to_hsv(p[0], p[1], p[2]);
        assert!((h - LEFT_BASS_HUE).abs() < 2.0, "{h}");
    }

    #[test]
    fn hsv_round_trips() {
        for (r, g, b) in [
            (0, 0, 0),
            (255, 255, 255),
            (255, 0, 0),
            (12, 200, 90),
            (150, 130, 255),
        ] {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            assert_eq!(hsv_to_rgb(h, s, v), (r, g, b));
        }
    }

    #[test]
    fn reverse_puts_the_judge_line_on_top() {
        use crate::settings as s;
        let mut v = s::Values::default();
        let normal = Looks::from_settings(&v);
        assert!(!normal.reverse);
        assert_eq!(normal.ahead_y(100.0), JUDGE_LINE_Y - 100.0);
        assert_eq!(normal.fire_y() + 190.0, JUDGE_LINE_Y + 4.0);
        assert!(normal.beyond_edge(-70.0) && !normal.beyond_edge(790.0));
        v.set_raw(s::REVERSE, 1);
        v.set_raw(s::JUDGE_LINE_POS, 20);
        let rev = Looks::from_settings(&v);
        // 上流：判定ラインは 159 + JudgeLinePos、チップは判定ラインより下、火花は判定ライン - 183 が基準
        assert_eq!(rev.judge_y(), 179.0);
        assert_eq!(rev.ahead_y(100.0), 279.0);
        assert_eq!(rev.fire_y(), 179.0 - 183.0);
        assert!(rev.beyond_edge(790.0) && !rev.beyond_edge(-70.0));
    }

    #[test]
    fn gauge_follows_the_damage_level() {
        let mut g = Gauge::new(GaugeRule::default());
        g.apply(Judgement::Miss);
        assert!((g.value - (2.0 / 3.0 - 0.0205)).abs() < 1e-9);
        let mut small = Gauge::new(GaugeRule {
            miss_factor: 0.25,
            ..GaugeRule::default()
        });
        small.apply(Judgement::Miss);
        assert!((small.value - (2.0 / 3.0 - 0.01025)).abs() < 1e-9);
        // POOR には減り方をかけない
        small.apply(Judgement::Poor);
        assert!((small.value - (2.0 / 3.0 - 0.01025 - 0.017)).abs() < 1e-9);
    }

    #[test]
    fn hazard_drains_on_great_and_good() {
        let mut g = Gauge::new(GaugeRule {
            hazard: true,
            ..GaugeRule::default()
        });
        g.apply(Judgement::Perfect);
        g.apply(Judgement::Great);
        g.apply(Judgement::Good);
        assert!((g.value - (2.0 / 3.0 + 0.005 - 0.082)).abs() < 1e-9);
    }

    #[test]
    fn risky_fails_after_n_misses() {
        let mut g = Gauge::new(GaugeRule {
            risky: 3,
            ..GaugeRule::default()
        });
        assert_eq!(g.value, 1.0);
        g.apply(Judgement::Perfect);
        assert_eq!(g.value, 1.0);
        g.apply(Judgement::Miss);
        assert!((g.value - 0.5).abs() < 1e-9 && !g.empty());
        g.apply(Judgement::Poor);
        assert!(g.value.abs() < 1e-9 && !g.empty());
        g.apply(Judgement::Miss);
        assert!(g.empty());
        // 1 回なら空から始まり、1 回で尽きる
        let mut one = Gauge::new(GaugeRule {
            risky: 1,
            ..GaugeRule::default()
        });
        assert_eq!(one.value, 0.0);
        one.apply(Judgement::Great);
        assert!(!one.empty());
        one.apply(Judgement::Miss);
        assert!(one.empty());
        one.apply(Judgement::Miss);
        assert_eq!(one.value, 0.0);
    }

    #[test]
    fn all_perfect_is_exactly_a_million() {
        assert_eq!(all_perfect_score(37), MAX_SCORE);
        assert_eq!(all_perfect_score(50), MAX_SCORE);
        assert_eq!(all_perfect_score(400), MAX_SCORE);
        assert_eq!(all_perfect_score(1234), MAX_SCORE);
    }

    #[test]
    fn score_grows_with_the_combo_up_to_50() {
        let mut t = Tally::default();
        t.add(Judgement::Perfect);
        t.add(Judgement::Perfect);
        // 400 音符：100 万 / (1275 + 50 × 350) = 53.28…、コンボ 2 なら 2 倍
        assert_eq!(score_delta(Judgement::Perfect, &t, 400, 0.0), 106.0);
        assert_eq!(score_delta(Judgement::Great, &t, 400, 0.0), 53.0);
        assert_eq!(score_delta(Judgement::Miss, &t, 400, 0.0), 0.0);
    }

    #[test]
    fn progress_text_marks_each_section() {
        let mut sections = [Section::default(); SECTIONS];
        sections[0] = Section { notes: 4, hits: 4 };
        sections[1] = Section { notes: 4, hits: 3 };
        sections[2] = Section { notes: 4, hits: 0 };
        // 区間 3 は音符なし
        sections[4] = Section { notes: 1, hits: 1 };
        // 64 秒の曲の 4.5 秒まで：区間 0〜3 にたどり着いた
        let t = progress_text(&sections, 64_000.0, 4_500.0);
        assert_eq!(t.len(), SECTIONS);
        assert_eq!(&t[..6], "211300");
        assert!(t[4..].chars().all(|c| c == '0'));
        // 最後まで
        let t = progress_text(&sections, 64_000.0, 66_000.0);
        assert_eq!(&t[..5], "21132");
        assert!(t[5..].chars().all(|c| c == '3'));
    }

    #[test]
    fn small_digits_skip_spaces_but_advance() {
        let g = &*SMALL;
        assert!(g.table.iter().any(|(c, _, _)| *c == '%'));
        assert_eq!(g.missing, 20.0);
    }
}
