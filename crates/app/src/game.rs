//! 1 曲を遊ぶ。譜面・音・判定・入力をまとめ、曲の時刻で動かす。
//! 練習（区間ループ）も同じ形で動かす。練習の時計は「始めてからの実時間」で、周ごとに音符と BGM を後ろへ足す。
//! 基礎練習（ドリル）は型から作った譜面を、ふつうの演奏と同じ時計と判定で動かす。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dtx::{Chart, DrumLane, TempoMap};
use engine::analysis::{NoteResult, Tendency, WeakPhrase, mean_and_sd};
use engine::clock::MidiTimestampMapper;
use engine::input::{Articulation, InputEvent, MidiMessage, MidiProfile, Normalizer, Pad};
use engine::judge::{Hit, HitWindows, Judge, JudgeEvent, Judgement, LaneGroups, Tally};
use engine::practice::{Attempt, SpeedPlan, place};
use engine::stroke::{Stroke, StrokeTally};
use i18n::tf;
use records::{Lap, NoteHit, SectionClear, Session, SessionKind, StrayHit};

use crate::audio::{AudioOut, Bus, HitTiming, Sound, click, drum_voice, load_sound};
use crate::chart_sound::{AutoMix, ChartSounds, Hat};
use crate::drill::{DrillOutcome, DrillPlay};
use crate::history::OvercomeCheck;
use crate::midi::RawMidi;
use crate::practice::{
    CLEAR_HOLD_MS, LapSummary, Practice, PracticeOptions, StrayHint, count_per_measure,
};
use crate::replay::{Feed, Take};
use crate::stretch::{SectionAudio, mix_bgm};
use crate::time::host_us;

/// 曲が始まる前の空白（ミリ秒）
const LEAD_IN_MS: f64 = 2000.0;

/// 練習で、助走の前に置く空白（ミリ秒）
const PRACTICE_LEAD_IN_MS: f64 = 1000.0;

/// カウントのクリックの音量
const CLICK_GAIN: f32 = 0.8;

/// メトロノーム（周の中の拍のクリック）の音量。BGM の邪魔をしないよう、カウントより小さく
const METRONOME_GAIN: f32 = 0.45;

/// 練習でない演奏のメトロノームの音量（小節の頭、拍。上流の Metronome と同じく拍は小節の頭の 4 割）
const METRONOME_BAR_GAIN: f32 = 1.0;
const METRONOME_BEAT_GAIN: f32 = 0.4;
/// 基礎練習のクリックの拍の音量（クリックに合わせて叩くので、曲のときより大きく）
const DRILL_BEAT_GAIN: f32 = 0.7;

/// 基礎練習で、最後の小節が終わってから結果へ移るまで（ミリ秒）
const DRILL_TAIL_MS: f64 = 1000.0;
/// 限界チャレンジで 3 つ落としてから結果へ移るまで（ミリ秒）
const DRILL_STOP_MS: f64 = 1500.0;
/// 聞いてまねるのお手本の音の番号（譜面の `#WAVzz` は 36 × 36 未満なので重ならない）
const DEMO_WAV: u16 = 0xF000;

/// メトロノームの音に付ける目印（一時停止の板で切ったとき、鳴らす予定の音を取りやめる）
const METRONOME_TAG: u8 = 2;

/// 練習の区間の BGM をまとめるとき、助走の前と着地のあとに足す余白（曲のミリ秒）。伸ばした音の端のにじみを周に入れないため
const SECTION_MARGIN_MS: f64 = 1000.0;

/// 落ちてくる音符が見え始めるより、これだけ前に周を並べる（ミリ秒）
const LAY_MARGIN_MS: f64 = 500.0;

/// 自動で鳴らす音（BGM・効果音など）を、今よりこれだけ先の分まで出力に渡しておく（ミリ秒）
const AUTO_AHEAD_MS: f64 = 1500.0;

/// 自動で叩いたときのレーンの光の強さ（ベロシティ）
const AUTO_VELOCITY: u8 = 100;

/// 最後まで遊ばなかったプレイのリプレイを、最後の判定からどれだけあとで終えるか（ミリ秒）
const REPLAY_TAIL_MS: f64 = 1500.0;

/// 練習の出来事。演奏の画面が取り出して音を鳴らし、終わったら結果へ進む
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PracticeEvent {
    /// 段を越えた（次の段へ）
    StepUp,
    /// 目標の段を越えた
    Cleared,
    /// クリアを見せ終えた。この時刻（曲の時刻）までで結果へ
    Finished { at_ms: f64 },
    /// 1 周叩かなかったので休みにした（一時停止の板を出す）
    Rest,
}

/// 画面の効果に使う出来事。演奏画面が毎フレーム取り出す
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fx {
    /// 叩いた（判定に関わらず。レーンの光とパッドの動き）。`lane` は叩いたパッドのレーン。
    /// `rim` はスネアのリム（リムショットかクロススティック）で叩いた（自動ならリムの音符を叩いた）
    Hit {
        lane: DrumLane,
        velocity: u8,
        rim: bool,
    },
    /// スネアの音符を、違う所で叩いた（判定はそのまま）。`want` は叩くべきだった方
    WrongStroke { want: Stroke },
    /// 音符を判定した（見逃しも）。`lane` と `note_ms` は音符のもの
    Judged {
        lane: DrumLane,
        judgement: Judgement,
        offset_ms: Option<f64>,
        note_ms: f64,
    },
    /// 音符の無いところを叩いて MISS にした（上流の Tight）。`lane` は叩いたパッドのレーン
    StrayMiss { lane: DrumLane },
}

/// 判定の決まり（設定から）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    /// 自動で叩くレーン（上流の AutoPlay）
    pub auto: Vec<DrumLane>,
    /// 判定の幅（ペダル以外）
    pub windows: HitWindows,
    /// バスドラム・左ペダル・左バスドラムの判定の幅
    pub pedal_windows: HitWindows,
    /// 音符の無いところを叩くと MISS（上流の Tight。練習では使わない）
    pub tight: bool,
    pub gauge: GaugeRule,
    /// 叩き分け（譜面に無いレーンは共通に寄せてから使う）
    pub groups: LaneGroups,
    pub hit_sounds: HitSoundRule,
    /// 自動で鳴らす音の音量・BGM の有無・ずらし
    pub auto_mix: AutoMix,
    /// メトロノームを鳴らす
    pub metronome: bool,
}

/// 叩いた音の鳴らし方（上流の HH / FT / CY Priority と Muting LP）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitSoundRule {
    /// 共通にしたレーンで別のレーンの音符を取ったとき、叩いたパッドのレーンの音を鳴らす
    /// （ハイハットと左シンバル、ロータムとフロアタム、シンバルとライドの順）
    pub pad_first: [bool; 3],
    /// 閉じたハイハットか左ペダルの音を鳴らしたら、鳴っているハイハットの音（叩いた音と自動で鳴らす音）を止める
    pub muting_lp: bool,
}

impl Default for HitSoundRule {
    fn default() -> Self {
        Self {
            pad_first: [false; 3],
            muting_lp: true,
        }
    }
}

/// ハイハットの音に付ける目印（閉じたハイハットと左ペダルで止める。目印はビットの組）
const HH_TAG: u8 = 1;
/// 叩いたシンバルの音に付ける目印（叩いたパッドを握ったら止める。チョーク）。メトロノームの目印とも重ねない
const CY_TAG: u8 = 4;
const LC_TAG: u8 = 8;
const RD_TAG: u8 = 16;

/// 叩いたパッドのレーン `lane` の、握って止めるための目印（シンバルでなければ 0）
fn choke_tag(lane: DrumLane) -> u8 {
    match lane {
        DrumLane::CY => CY_TAG,
        DrumLane::LC => LC_TAG,
        DrumLane::RD => RD_TAG,
        _ => 0,
    }
}

/// パッド `pad` を握ったら止める音の目印（[`engine::input`] のパッドとレーンの対応と同じ）
fn grip_tag(pad: Pad) -> u8 {
    match pad {
        Pad::Crash1 => LC_TAG,
        Pad::Crash2 => CY_TAG,
        Pad::Ride => RD_TAG,
        Pad::HiHat => HH_TAG,
        _ => 0,
    }
}

/// ハイハットの音なら [`HH_TAG`]
fn hat_tag(hat: Option<Hat>) -> u8 {
    if hat.is_some_and(Hat::is_hat) {
        HH_TAG
    } else {
        0
    }
}

/// 叩いた音の止め方。鳴らす音のレーン `sound_lane` と叩いたパッドのレーン `pad_lane` から、
/// 鳴らす前に鳴っているハイハットの音を止めるか（上流の Muting LP が `muting_lp`）と、鳴らす音に付ける目印
fn hit_sound_tags(muting_lp: bool, sound_lane: DrumLane, pad_lane: DrumLane) -> (bool, u8) {
    let hat = Hat::of_lane(sound_lane);
    (
        muting_lp && hat.is_some_and(Hat::mutes),
        hat_tag(hat) | choke_tag(pad_lane),
    )
}

/// ゲージの増え方と減り方（上流 `CActPerfCommonGauge`）。演奏の画面がこれでゲージを動かす
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GaugeRule {
    /// MISS の減りにかける数（上流 `fDamageLevelFactor`。ふつうは 0.5）
    pub miss_factor: f64,
    /// ミスの許容数（上流の Risky。0 なら使わない）
    pub risky: u32,
    /// GREAT と GOOD でも MISS と同じだけ減る（上流の HAZARD）
    pub hazard: bool,
}

impl Default for GaugeRule {
    fn default() -> Self {
        Self {
            miss_factor: 0.5,
            risky: 0,
            hazard: false,
        }
    }
}

/// 取り出されないまま溜められる効果の数（画面を描いていないあいだに増え続けないように）
const MAX_FX: usize = 1024;

/// 小節線と拍線（曲の時刻）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatLine {
    pub ms: f64,
    /// 小節の頭（false なら拍の線）
    pub bar: bool,
}

/// 最後に届いた入力（電子ドラムの確認用に画面に出す）
#[derive(Debug, Clone)]
pub struct LastInput {
    pub source: String,
    pub pad: Option<Pad>,
    pub articulation: Option<Articulation>,
    pub lane: Option<DrumLane>,
    pub velocity: u8,
    pub hh_pedal: Option<u8>,
}

/// 1 曲を遊び終えたときの成績
#[derive(Debug, Clone)]
pub struct Outcome {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub level: Option<f64>,
    pub notes: usize,
    pub tally: Tally,
    /// 叩けた音符のずれ（ミリ秒、マイナスは早い）の平均と標準偏差
    pub offset_mean_ms: Option<f64>,
    pub offset_sd_ms: Option<f64>,
    /// 叩けた音符のずれ（結果の画面の散らばりに使う）と、判定の幅（ペダル以外）
    pub offsets: Vec<f64>,
    pub windows: HitWindows,
    /// 結果の画面の「ずれ」の板（普通のプレイだけ。記録を読んでから入れる）
    pub lag: Option<crate::lag_view::LagResult>,
    /// 最後まで遊んだ（途中でやめたら false）
    pub cleared: bool,
    /// 小節ごとの音符の数（長さが譜面の小節の数）
    pub notes_per_measure: Vec<usize>,
    /// 今回の、音符ごとの判定（苦手の帯に使う。記録が読めないときもこれで出す）
    pub results: Vec<NoteResult>,
    /// 苦手な小節（記録を読んでから入れる）
    pub weak: Option<WeakMap>,
    /// 練習でクリアした区間を、今回のプレイで克服したか（記録を読んでから入れる）
    pub overcome: Vec<OvercomeCheck>,
    /// すべての曲を通した叩き方のくせ（記録を読んでから入れる）
    pub tendencies: Vec<Tendency>,
    /// 始めた時刻（UNIX 秒）
    pub started_at: i64,
    /// 練習なら、その決めごとと周ごとの成績
    pub practice: Option<PracticeOutcome>,
    /// 基礎練習なら、段ごとの出来と手ごとのずれ
    pub drill: Option<DrillOutcome>,
    /// スネアの打ち分け（譜面にリムの音符があり、電子ドラムで叩いたときだけ）
    pub strokes: Option<StrokeTally>,
    /// 記録に残したセッションの番号（普通のプレイなら、結果の画面からリプレイを見られる）
    pub session_id: Option<i64>,
}

/// 練習 1 回の成績
#[derive(Debug, Clone)]
pub struct PracticeOutcome {
    pub options: PracticeOptions,
    pub laps: Vec<LapSummary>,
    /// 段の挑戦ごとのまとめ（途中でやめた挑戦も、最後まで叩いた周があれば入れる）
    pub attempts: Vec<Attempt>,
    /// 練習した時間（最初の助走から、やめるまで。ミリ秒）
    pub elapsed_ms: f64,
    /// 目標の段を越えた
    pub cleared: bool,
    /// 空打ちが偏っているパッド（拾いすぎの疑い）
    pub strays: Option<StrayHint>,
    /// 最後まで叩いた周のずれの平均（ミリ秒）と打った数
    pub offset: Option<(f64, usize)>,
    /// 譜面の絵（ミスした所に印を付けて出す）
    pub picture: crate::chart_picture::ChartPicture,
}

/// 苦手な小節の地図（結果の画面に出す）
#[derive(Debug, Clone)]
pub struct WeakMap {
    /// 小節ごとの苦手度（0〜1）。記録の無い小節は None
    pub weakness: Vec<Option<f64>>,
    pub phrases: Vec<WeakPhrase>,
    /// 何回ぶんの普通のプレイの記録から出したか
    pub plays: usize,
    /// 何周ぶんの練習の記録から出したか
    pub practice_laps: usize,
}

pub struct Play {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub level: Option<f64>,
    pub judge: Judge,
    pub audio: Option<AudioOut>,
    pub end_ms: f64,
    /// 入力の遅れの補正（全体）。叩いた時刻からこの分を引く
    pub input_offset_ms: f64,
    /// この曲の補正。判定と譜面の流れを一緒にこの分だけ遅らせる（譜面と BGM が合っていない曲など）
    pub song_offset_ms: f64,
    /// 叩いたときに譜面の音を鳴らすか（モジュールの音を使うなら切る）
    pub drum_sounds: bool,
    /// スネアの音符の音のうち、リムの音らしいもの（その音のスネアの音符がリムの音符）
    rim_wavs: HashSet<u16>,
    /// 画面の効果（演奏画面が取り出す）
    fx: Vec<Fx>,
    /// 練習の出来事（演奏画面が取り出す）
    practice_events: Vec<PracticeEvent>,
    /// 練習で、終わった段の挑戦の数と、クリアを見せ終えたと知らせたか
    seen_attempts: usize,
    finish_sent: bool,
    /// 小節線と拍線（時刻の順）
    pub lines: Vec<BeatLine>,
    /// メトロノームを鳴らす時刻（時刻の順）。ふつうは拍線と同じで、基礎練習はドリルの拍子とクリックの型に合わせる
    click_lines: Vec<BeatLine>,
    /// 曲の頭のテンポ
    pub bpm: f64,
    /// 譜面の最後のチップの時刻（進み具合のバーに使う）
    pub last_chip_ms: f64,
    /// ジャケットの画像（`#PREIMAGE`。無ければ None）
    pub preimage: Option<PathBuf>,
    /// 譜面の動画（`#AVI`）を流し始める曲の時刻とファイル（時刻の順。ファイルが無いものは入れない）
    pub movies: Vec<(f64, PathBuf)>,
    pub last_input: Option<LastInput>,
    pub hh_pedal: Option<u8>,
    /// 叩けた音符のずれ（ミリ秒）
    hit_offsets: Vec<f64>,
    /// 譜面のファイルの中身のハッシュ（記録で譜面を見分ける）
    pub chart_hash: u64,
    /// 始めた時刻（UNIX 秒）
    pub started_at: i64,
    /// 遊んだ時間を残すときの種類（[`records::time_kind`]。曲か区間練習。基礎練習などはここを変える）
    pub time_kind: &'static str,
    notes_per_measure: Vec<usize>,
    /// 小節の頭の拍（曲頭から）。音符の拍の中の位置を出すのに使う
    measure_beats: Vec<f64>,
    /// 練習なら周の並びと集計
    pub practice: Option<Practice>,
    /// 基礎練習なら、段と手順とクリックの消える小節
    pub drill: Option<DrillPlay>,
    /// 練習の区間の BGM（速度ごとに伸ばしたもの）。音が出ないときは None
    section_audio: Option<SectionAudio>,
    /// 判定の線より上に見えている時間（ミリ秒）。練習の周をこれより前に並べる。画面の高さと落ちる速さから決める
    pub view_ahead_ms: f64,
    /// カウントのクリック（1 拍目、ほか）
    clicks: Option<(Sound, Sound)>,
    /// メトロノームを鳴らす（練習でなければ設定のメトロノーム、練習なら練習の決めごとのメトロノーム）。
    /// 一時停止の板から切り替える（[`Self::set_metronome`]）
    metronome: bool,
    /// 練習でなければ、メトロノームで次に出力へ渡す拍の線（`lines` の添字）
    click_next: usize,
    /// 音符ごとの結果（判定したものだけ埋まる）
    note_hits: Vec<Option<NoteHit>>,
    strays: Vec<StrayHit>,
    sounds: HashMap<u16, Sound>,
    /// 譜面の音の鳴らし方（自動で鳴らす音、音ごとの音量と左右、空打ちの音）
    chart_sounds: ChartSounds,
    /// 自動で鳴らす音のうち、次に出力へ渡すもの（練習では使わない）
    auto_next: usize,
    /// MIDI のポートごとの入力の読み手と、ポートの名前（[`crate::midi::MidiIn::ports`] と同じ並び）
    inputs: Vec<(String, Normalizer)>,
    mappers: HashMap<usize, MidiTimestampMapper>,
    /// 叩いた音を鳴らさなかった打撃の時刻（FPS と遅延の表示へ。鳴らしたものは出力から返ってくる）
    silent_timings: Vec<HitTiming>,
    /// 音が出ないときの時計の起点（ホスト時刻）。この時刻が時間軸の `start_ms`
    fallback_origin_us: i64,
    start_ms: f64,
    /// 一時停止しているなら、止めた曲の時刻
    paused_ms: Option<f64>,
    /// 早送りか巻き戻しをした（上流の練習扱い。記録に残さず、STAGE FAILED にもしない）
    skipped: bool,
    /// 空打ちを MISS にする（練習では使わない）
    tight: bool,
    /// ゲージの決まり（演奏の画面が使う）
    pub gauge: GaugeRule,
    hit_sounds: HitSoundRule,
    /// リプレイなら、組み立て直した打撃（叩いても判定しない。記録にも残さない）
    replay: Option<Feed>,
    /// リプレイで飛んだ先まで一度に判定しているところ（画面の効果を捨てずに全部ためる）
    catching_up: bool,
}

impl Play {
    /// 譜面を読んで始める。`input_offset_ms` と `drum_sounds` は設定から
    /// MIDI のポートごとの割り当て（ポートの名前と割り当て。[`crate::midi::MidiIn::ports`] と同じ並び）
    pub fn set_inputs(&mut self, inputs: Vec<(String, MidiProfile)>) {
        self.inputs = inputs
            .into_iter()
            .map(|(name, p)| (name, Normalizer::new(p)))
            .collect();
    }

    /// `rules` は自動で叩くレーンや判定の幅
    pub fn load(
        path: &Path,
        input_offset_ms: f64,
        drum_sounds: bool,
        rules: &Rules,
    ) -> Result<Self> {
        Self::open(path, input_offset_ms, drum_sounds, rules, None)
    }

    /// 基礎練習を始める。譜面は型から作る（ファイルは読まない）。空打ちの MISS は使わず、クリックはいつも鳴らす
    pub fn load_drill(
        run: engine::drill::Run,
        input_offset_ms: f64,
        drum_sounds: bool,
        rules: &Rules,
    ) -> Result<Self> {
        let drill = run.drill;
        let path = crate::drill::chart_path(drill);
        let text = run.to_dtx(crate::drill::name(drill));
        let rules = Rules {
            tight: false,
            metronome: true,
            ..rules.clone()
        };
        let mut play = Self::build(
            &path,
            text.as_bytes(),
            input_offset_ms,
            drum_sounds,
            &rules,
            None,
            Some(run),
        )?;
        // 記録では、テンポとやり方が違ってもドリルごとに同じ譜面として扱う
        play.chart_hash = records::hash_bytes(drill.id.as_bytes());
        play.artist = crate::drill::category_name(drill.category).to_owned();
        play.time_kind = records::time_kind::DRILL;
        Ok(play)
    }

    /// 区間を練習する
    pub fn load_practice(
        path: &Path,
        input_offset_ms: f64,
        drum_sounds: bool,
        rules: &Rules,
        options: PracticeOptions,
    ) -> Result<Self> {
        Self::open(path, input_offset_ms, drum_sounds, rules, Some(options))
    }

    /// 記録したプレイ `rec` を再生する。`rules` は遊んだときの決まりにしたもの（[`crate::replay::RecordedRules`]）。
    /// 判定の時刻は記録のまま使うので、入力の補正は 0 にし、この曲の補正は遊んだときの値にする
    pub fn load_replay(
        path: &Path,
        rules: &Rules,
        song_offset_ms: f64,
        rec: &records::PlayTakes,
    ) -> Result<Self> {
        let mut play = Self::open(path, 0.0, true, rules, None)?;
        play.song_offset_ms = song_offset_ms;
        play.started_at = rec.entry.started_at;
        let fresh = play.judge.clone();
        let (takes, missing) = crate::replay::takes(play.judge.notes(), &rec.hits, &rec.strays);
        let r = crate::replay::rehearse(fresh.clone(), takes);
        let differ = crate::replay::differences(play.judge.notes(), &rec.hits, &r.judged);
        let (was, now) = (&rec.entry.tally, &r.tally);
        log::info!(
            "replay of {}: {} hits, {missing} notes not in the chart, {differ} judged differently \
             (PERFECT {} → {}, GREAT {} → {}, MISS {} → {})",
            rec.entry.session_id,
            r.takes.len(),
            was.perfect,
            now.perfect,
            was.great,
            now.great,
            was.miss,
            now.miss,
        );
        let sounds = r
            .takes
            .iter()
            .zip(&r.matched)
            .map(|(t, m)| match m {
                Some(i) => {
                    let note = play.judge.notes()[*i].clone();
                    Some(play.hit_wav(t.lane, &note, t.ms, t.articulation))
                }
                None => play
                    .stray_wav(t.lane, t.ms, t.articulation)
                    .map(|w| (w, t.lane)),
            })
            .collect();
        // 最後まで遊ばなかったプレイは、最後に判定した音符か空打ちの少しあとで終える
        let end_ms = if rec.entry.completed {
            play.end_ms - song_offset_ms
        } else {
            let index: HashMap<(dtx::NotePos, DrumLane), f64> = play
                .judge
                .notes()
                .iter()
                .map(|n| ((n.at, n.lane), n.ms))
                .collect();
            rec.hits
                .iter()
                .filter_map(|h| index.get(&(h.at, h.lane)).copied())
                .chain(rec.strays.iter().map(|s| s.at_ms))
                .fold(0.0, f64::max)
                + REPLAY_TAIL_MS
        };
        play.replay = Some(Feed {
            takes: r.takes,
            matched: r.matched,
            sounds,
            next_hit: 0,
            next_sound: 0,
            end_ms,
            fresh,
            judged: r.judged,
        });
        Ok(play)
    }

    /// リプレイなら、その打撃
    pub fn replay(&self) -> Option<&Feed> {
        self.replay.as_ref()
    }

    fn open(
        path: &Path,
        input_offset_ms: f64,
        drum_sounds: bool,
        rules: &Rules,
        practice: Option<PracticeOptions>,
    ) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| tf!("Cannot read {}", "{} を読めません", path.display()))?;
        Self::build(
            path,
            &bytes,
            input_offset_ms,
            drum_sounds,
            rules,
            practice,
            None,
        )
    }

    /// 譜面の中身 `bytes` から作る。`path` は記録と音のファイルを探すのに使う
    fn build(
        path: &Path,
        bytes: &[u8],
        input_offset_ms: f64,
        drum_sounds: bool,
        rules: &Rules,
        practice: Option<PracticeOptions>,
        drill: Option<engine::drill::Run>,
    ) -> Result<Self> {
        let auto = rules.auto.as_slice();
        let chart = dtx::load(bytes);
        for w in &chart.warnings {
            log::warn!("{}:{}: {}", path.display(), w.line, w.message);
        }
        let tempo = TempoMap::build(&chart);
        let notes = tempo.drum_notes(&chart);
        let rim_wavs = chart.rim_wavs(&notes);
        let notes_per_measure = count_per_measure(&notes, tempo.measure_count());
        let last_chip_ms = chart
            .chips
            .iter()
            .map(|c| tempo.chip_ms(c))
            .fold(0.0, f64::max);
        let end_ms = match &drill {
            Some(run) => tempo.measure_start_ms(run.measure_count() as usize) + DRILL_TAIL_MS,
            None => last_chip_ms + 2000.0,
        };
        let mut chart_sounds = ChartSounds::mixed(&chart, &tempo, rules.auto_mix);
        // 自動で叩くレーンの音は、BGM と同じく時刻を決めて鳴らす（叩いた音の設定に関わらず）
        chart_sounds.add_auto_notes(
            notes
                .iter()
                .filter(|n| auto.contains(&n.lane))
                .map(|n| (n.ms, n.wav, n.lane)),
        );
        // 上流の Muting LP: 自動で鳴らすハイハットの音は、後の閉じたハイハットか左ペダルの音で止める
        if rules.hit_sounds.muting_lp {
            chart_sounds.mute_hats();
        }
        // メトロノームは演奏中に切り替えられるよう、自動で鳴らす音にまとめず拍の線から鳴らす（`lay_auto`）。
        // 練習は区間の音を周の速度に伸ばしてまとめるので、周ごとに鳴らす（`lay_laps`）
        let lines = beat_lines(&tempo);
        // 叩き分けの寄せ方は曲全体の音符で決める（練習の区間によって判定が変わらないように）
        let groups = rules.groups.effective_for(&notes);
        let (mut judge, practice) = match practice {
            None => (Judge::with_groups(notes, rules.windows, groups), None),
            Some(options) => (
                Judge::with_groups(Vec::new(), rules.windows, groups),
                Some({
                    let mut p = Practice::new(options, &tempo, notes);
                    p.set_auto_lanes(auto);
                    p
                }),
            ),
        };
        judge.set_auto(auto);
        judge.set_pedal_windows(rules.pedal_windows);
        let drill = drill.map(|run| DrillPlay::new(run, &tempo, judge.notes()));
        // 聞いてまねるのお手本は、作った音を自動で鳴らす音として時刻を決めて鳴らす（判定はしない）
        let demo = drill.as_ref().map_or_else(Vec::new, DrillPlay::demo_hits);
        chart_sounds.add_auto_notes(demo.iter().map(|&(ms, lane)| (ms, demo_wav(lane), lane)));
        let click_lines = match &drill {
            Some(d) => d
                .clicks()
                .into_iter()
                .map(|(ms, bar)| BeatLine { ms, bar })
                .collect(),
            None => lines.clone(),
        };
        let tight = rules.tight && practice.is_none();
        let start_ms = practice
            .as_ref()
            .map_or(-LEAD_IN_MS, |p| p.first_audio_ms() - PRACTICE_LEAD_IN_MS);

        let audio = match AudioOut::open(start_ms) {
            Ok(a) => Some(a),
            Err(e) => {
                log::warn!("audio is off: {e:#}");
                None
            }
        };
        let sounds = match &audio {
            Some(a) => {
                let mut sounds = load_sounds(
                    &chart,
                    &chart_sounds,
                    path.parent().unwrap_or(Path::new(".")),
                    a.rate(),
                );
                for &(_, lane) in &demo {
                    sounds
                        .entry(demo_wav(lane))
                        .or_insert_with(|| drum_voice(a.rate(), lane));
                }
                sounds
            }
            None => HashMap::new(),
        };
        let mut section_audio = None;
        // 練習でなければ、自動で鳴らす音（BGM・効果音など）は時刻が近づいたものから出力に渡す（`lay_auto`）。
        // 練習は区間の自動で鳴らす音を 1 本にまとめ、周の速度に伸ばして鳴らす
        if let (Some(a), Some(p)) = (audio.as_ref(), &practice) {
            let t = p.times();
            let from = t.run_up_ms - SECTION_MARGIN_MS;
            let to = t.tail_ms.max(t.end_ms) + SECTION_MARGIN_MS;
            let started = std::time::Instant::now();
            let first = p.speed();
            section_audio = Some(SectionAudio::new(
                mix_bgm(&chart_sounds.auto, &sounds, a.rate(), from, to),
                from,
                a.rate(),
                p.options.pitch,
                first,
            ));
            log::info!(
                "section audio at {first}%: {:.0}ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        // メトロノームの音は NX のスキンに無いので、数え始めと同じ作った音を使う
        let clicks = audio
            .as_ref()
            .map(|a| (click(a.rate(), true), click(a.rate(), false)));
        let metronome = practice
            .as_ref()
            .map_or(rules.metronome, |p| p.options.metronome);

        log::info!(
            "{}: {} notes, {:.1}s{}",
            path.display(),
            judge.notes().len(),
            end_ms / 1000.0,
            practice.as_ref().map_or(String::new(), |p| format!(
                ", practice measures {}-{} from {start_ms:.0}ms",
                p.options.section.first, p.options.section.last
            ))
        );
        let note_count = judge.notes().len();
        let mut play = Self {
            path: path.to_owned(),
            title: chart.title().to_owned(),
            artist: chart.artist().to_owned(),
            level: chart.drum_level(),
            judge,
            audio,
            end_ms,
            input_offset_ms,
            song_offset_ms: 0.0,
            drum_sounds,
            rim_wavs,
            fx: Vec::new(),
            practice_events: Vec::new(),
            seen_attempts: 0,
            finish_sent: false,
            lines,
            click_lines,
            bpm: 60_000.0 / (tempo.beat_to_ms(1.0) - tempo.beat_to_ms(0.0)).max(1.0),
            last_chip_ms,
            preimage: preimage_path(&chart, path),
            movies: movie_cues(&chart, &tempo, path),
            last_input: None,
            hh_pedal: None,
            hit_offsets: Vec::new(),
            chart_hash: records::hash_bytes(bytes),
            started_at: unix_now(),
            time_kind: if practice.is_some() {
                records::time_kind::PRACTICE
            } else {
                records::time_kind::PLAY
            },
            notes_per_measure,
            measure_beats: (0..=tempo.measure_count())
                .map(|m| tempo.measure_start_beat(m))
                .collect(),
            practice,
            drill,
            section_audio,
            view_ahead_ms: 2000.0,
            clicks,
            metronome,
            click_next: 0,
            note_hits: vec![None; note_count],
            strays: Vec::new(),
            sounds,
            chart_sounds,
            auto_next: 0,
            inputs: Vec::new(),
            mappers: HashMap::new(),
            silent_timings: Vec::new(),
            fallback_origin_us: host_us(),
            start_ms,
            paused_ms: None,
            skipped: false,
            tight,
            gauge: rules.gauge,
            hit_sounds: rules.hit_sounds,
            replay: None,
            catching_up: false,
        };
        play.lay_laps(start_ms);
        play.lay_auto(start_ms);
        Ok(play)
    }

    /// 鳴らし始める（曲の時計を動かす）。読み込んでから画面を切り替えるときに呼ぶ
    pub fn begin(&mut self) {
        if let Some(a) = &self.audio
            && let Err(e) = a.start()
        {
            log::warn!("audio is off: {e:#}");
            self.audio = None;
            self.section_audio = None;
        }
        self.fallback_origin_us = host_us();
    }

    /// たまった画面の効果を取り出す
    pub fn take_fx(&mut self) -> Vec<Fx> {
        std::mem::take(&mut self.fx)
    }

    /// たまった練習の出来事を取り出す
    pub fn take_practice_events(&mut self) -> Vec<PracticeEvent> {
        std::mem::take(&mut self.practice_events)
    }

    /// 終わった段の挑戦とクリアを、練習の出来事にする
    fn note_practice(&mut self, now: f64) {
        let Some(p) = self.practice.as_ref() else {
            return;
        };
        for a in &p.done[self.seen_attempts.min(p.done.len())..] {
            if a.passed == Some(true) {
                self.practice_events
                    .push(if p.cleared() && a.index + 1 == p.done.len() {
                        PracticeEvent::Cleared
                    } else {
                        PracticeEvent::StepUp
                    });
            }
        }
        self.seen_attempts = p.done.len();
        if let Some(at) = p.cleared_at_ms
            && !self.finish_sent
            && now - at >= CLEAR_HOLD_MS
        {
            self.finish_sent = true;
            self.practice_events
                .push(PracticeEvent::Finished { at_ms: now });
        }
    }

    fn push_fx(&mut self, fx: Fx) {
        if self.fx.len() >= MAX_FX && !self.catching_up {
            self.fx.drain(..MAX_FX / 2);
        }
        self.fx.push(fx);
    }

    /// 練習なら、終わった周の合否で速度を決め、`now_ms` から少し先までの周を並べる（判定の音符・BGM・クリック）
    fn lay_laps(&mut self, now_ms: f64) {
        let Some(p) = self.practice.as_mut() else {
            return;
        };
        if let Some(sa) = self.section_audio.as_mut() {
            sa.poll();
        }
        p.settle(now_ms);
        if p.check_rest(now_ms) {
            self.practice_events.push(PracticeEvent::Rest);
        }
        // 速度が変わったら、まだ始まっていない次の周を取り下げ、新しい速度の助走から並べ直す。
        // 新しい速度の音ができていなければ取り下げない（前の速度で回し、できてから変える）
        let ready = self
            .section_audio
            .as_ref()
            .is_none_or(|sa| sa.get(p.speed()).is_some());
        if ready {
            while let Some(w) = p.withdraw_stale(now_ms) {
                if !self.judge.truncate(w.notes_from) {
                    log::warn!("withdrawn lap had judged notes");
                }
                self.note_hits.truncate(w.notes_from);
                if let Some(a) = self.audio.as_mut() {
                    a.cancel_from(w.audio_from_ms);
                }
            }
        }
        let ahead = self.view_ahead_ms + LAY_MARGIN_MS;
        while p.wants_lap(now_ms, ahead) {
            // 決めた速度の音がまだできていなければ、できるまで前の周の速度で回す
            let want = p.speed();
            // セットの始めは、選んだ速度の音ができるまで並べない（前の速度で始めると知らせと合わない）
            if p.resume_pending()
                && self
                    .section_audio
                    .as_ref()
                    .is_some_and(|sa| sa.get(want).is_none())
            {
                break;
            }
            let speed = match &self.section_audio {
                Some(sa) if sa.get(want).is_none() => p.last_speed().unwrap_or(want),
                _ => want,
            };
            let (lap, notes) = p.next_lap(speed, now_ms + ahead);
            log::info!(
                "lap {} at {speed}%{}: {:.0}-{:.0}ms",
                lap.index + 1,
                if lap.has_run_up() { " with run-up" } else { "" },
                lap.start_ms,
                lap.end_ms
            );
            // 自動で叩く閉じたハイハットと左ペダルは、その時刻に叩いて鳴っているハイハットの音を止める
            // （自動で叩く音は区間の音にまとめてあり、まとめるときに止めてある）
            if self.hit_sounds.muting_lp
                && let Some(a) = self.audio.as_mut()
            {
                for n in &notes {
                    if self.judge.is_auto(n.lane) && Hat::of_lane(n.lane).is_some_and(Hat::mutes) {
                        a.stop_at(HH_TAG, n.ms);
                    }
                }
            }
            let range = self.judge.extend(notes);
            self.note_hits.resize(self.judge.notes().len(), None);
            if let Some(a) = self.audio.as_mut() {
                if let Some(sa) = &self.section_audio
                    && let Some(s) = sa.get(speed)
                    && let Some(at) = place(sa.from_ms, sa.song_len_ms, lap.audio, lap.speed)
                {
                    a.play_placed(s, &at, 1.0, Bus::Song);
                }
                if let Some((accent, plain)) = &self.clicks {
                    for (i, &c) in lap.clicks.iter().enumerate() {
                        let click = if i == 0 { accent } else { plain };
                        a.play_at(click, c, CLICK_GAIN, Bus::Click);
                    }
                    if self.metronome {
                        for &(c, bar) in &lap.beats {
                            let click = if bar { accent } else { plain };
                            a.play_at_tagged(click, c, METRONOME_GAIN, METRONOME_TAG, Bus::Click);
                        }
                    }
                }
            }
            p.push_lap(lap, range);
        }
        if let Some(sa) = self.section_audio.as_mut() {
            for s in p.speeds_to_prepare() {
                sa.request(s);
            }
        }
        p.forget_before(now_ms - 2000.0);
    }

    /// 練習でなければ、自動で鳴らす音（BGM・効果音・ギターとベースなど）と、鳴らしているならメトロノームの拍のうち、
    /// `now_ms` から少し先までのものを出力に渡す。何千もある曲でも、出力へ渡す口があふれないように少しずつ渡す
    fn lay_auto(&mut self, now_ms: f64) {
        if self.practice.is_some() {
            return;
        }
        let Some(a) = self.audio.as_mut() else {
            return;
        };
        let until = now_ms + AUTO_AHEAD_MS;
        while let Some(v) = self.chart_sounds.auto.get(self.auto_next) {
            if v.ms >= until {
                break;
            }
            self.auto_next += 1;
            // 上流の Muting LP: 閉じたハイハットと左ペダルは、その時刻に鳴っているハイハットの音
            // （叩いた音も）を止める。自動で鳴らすハイハットの音どうしは `mute_hats` で止めてある
            if self.hit_sounds.muting_lp && v.hat.is_some_and(Hat::mutes) {
                a.stop_at(HH_TAG, v.ms);
            }
            if let Some(s) = self.sounds.get(&v.wav) {
                let route = (Bus::Song, hat_tag(v.hat));
                a.play_chip(s, v.ms, v.len_ms, (v.gain, v.pan), route);
            }
        }
        if !self.metronome {
            return;
        }
        let Some((accent, plain)) = &self.clicks else {
            return;
        };
        while let Some(l) = self.click_lines.get(self.click_next) {
            if l.ms >= until {
                break;
            }
            self.click_next += 1;
            // 基礎練習は、クリックの消える小節と終わったあとは鳴らさない
            if self.drill.as_ref().is_some_and(|d| !d.clicks_at(l.ms)) {
                continue;
            }
            let (click, gain) = match (l.bar, self.drill.is_some()) {
                (true, _) => (accent, METRONOME_BAR_GAIN),
                (false, false) => (plain, METRONOME_BEAT_GAIN),
                (false, true) => (plain, DRILL_BEAT_GAIN),
            };
            a.play_at_tagged(click, l.ms, gain, METRONOME_TAG, Bus::Click);
        }
    }

    /// メトロノームを鳴らしているか
    pub fn metronome(&self) -> bool {
        self.metronome
    }

    /// メトロノームを鳴らす・止める（一時停止の板から。すぐ効く）。練習では練習の決めごとのメトロノームを変える
    /// （やり直しても続く）。止めたら鳴らす予定の拍を取りやめ、鳴らすならこれから鳴る拍を出力に渡す
    pub fn set_metronome(&mut self, on: bool) {
        if self.metronome == on {
            return;
        }
        self.metronome = on;
        if let Some(p) = self.practice.as_mut() {
            p.options.metronome = on;
        }
        let now = self.now_ms();
        if !on {
            if let Some(a) = self.audio.as_mut() {
                a.stop(METRONOME_TAG);
            }
            return;
        }
        match &self.practice {
            None => {
                self.click_next = self.click_lines.partition_point(|l| l.ms < now);
                self.lay_auto(now);
            }
            Some(p) => {
                // 並べてある周の拍のうち、これから鳴るもの（周はこの先 `lay_laps` が並べるときに鳴らす）
                let (Some(a), Some((accent, plain))) = (self.audio.as_mut(), &self.clicks) else {
                    return;
                };
                for lap in &p.laps {
                    for &(c, bar) in lap.plan.beats.iter().filter(|b| b.0 >= now) {
                        let click = if bar { accent } else { plain };
                        a.play_at_tagged(click, c, METRONOME_GAIN, METRONOME_TAG, Bus::Click);
                    }
                }
            }
        }
    }

    /// 練習の速度を手で `steps` 刻み動かす（次に並べる周から）
    pub fn nudge_speed(&mut self, steps: i64) {
        if let Some(p) = self.practice.as_mut() {
            p.nudge_speed(steps);
        }
    }

    /// 小節ごとの音符の数（曲全体）
    pub fn notes_per_measure(&self) -> &[usize] {
        &self.notes_per_measure
    }

    /// ホスト時刻 `host_us` の曲の時刻（一時停止しているあいだは止めた時刻）
    pub fn song_ms_at(&self, at_us: i64) -> f64 {
        if let Some(ms) = self.paused_ms {
            return ms;
        }
        self.audio
            .as_ref()
            .and_then(|a| a.song_ms_at(at_us))
            .unwrap_or_else(|| (at_us - self.fallback_origin_us) as f64 / 1000.0 + self.start_ms)
    }

    pub fn now_ms(&self) -> f64 {
        self.song_ms_at(host_us())
    }

    /// 画面の時刻 `now_ms` の、譜面の中の時刻。練習は周の速さと並びをほどく（ふつうはそのまま）
    pub fn chart_ms(&self, now_ms: f64) -> f64 {
        self.practice
            .as_ref()
            .map_or(now_ms, |p| p.chart_ms(now_ms))
    }

    /// 曲の時刻 `song_ms` を判定の時刻にする（入力の補正とこの曲の補正を引く）
    fn judge_ms(&self, song_ms: f64) -> f64 {
        song_ms - self.input_offset_ms - self.song_offset_ms
    }

    /// 譜面を描く時刻。`visual_offset_ms` は表示の補正。この曲の補正のぶん譜面を遅らせる
    pub fn view_ms(&self, visual_offset_ms: f64) -> f64 {
        self.now_ms() + visual_offset_ms - self.song_offset_ms
    }

    /// 曲が終わった（練習は自分でやめるまで終わらない）
    pub fn finished(&self) -> bool {
        if let Some(f) = &self.replay {
            return self.judge_ms(self.now_ms()) > f.end_ms;
        }
        self.practice.is_none() && self.now_ms() > self.end_ms
    }

    /// 一時停止する・つづける（上流 Pause）。止めているあいだは音も曲の時刻も止まり、叩いても数えない。
    /// 止めたら true
    pub fn toggle_pause(&mut self) -> bool {
        match self.paused_ms.take() {
            Some(ms) => {
                if let Some(a) = &self.audio {
                    a.set_paused(false);
                }
                // 音が出ないときの時計は、止めていたぶん起点を後ろへずらす
                self.fallback_origin_us = host_us() - ((ms - self.start_ms) * 1000.0) as i64;
                // 基礎練習は、続けたときから叩かない時間を数え直す
                let at = self.judge_ms(ms);
                if let Some(d) = self.drill.as_mut() {
                    d.resume_at(at);
                }
                false
            }
            None => {
                let ms = self.now_ms();
                if let Some(a) = &self.audio {
                    a.set_paused(true);
                }
                self.paused_ms = Some(ms);
                true
            }
        }
    }

    pub fn paused(&self) -> bool {
        self.paused_ms.is_some()
    }

    /// 早送りか巻き戻しをした
    pub fn skipped(&self) -> bool {
        self.skipped
    }

    /// 曲の時刻を `delta_ms` だけ先へ（マイナスなら前へ）飛ぶ（上流 SkipForward / SkipBackward）。
    /// 練習では使わない（None）。飛んだプレイは上流と同じく練習扱いで、記録に残さない。飛んだ先の時刻を返す
    pub fn seek(&mut self, delta_ms: f64) -> Option<f64> {
        if self.practice.is_some() || self.drill.is_some() {
            return None;
        }
        let now = self.now_ms();
        let to = (now + delta_ms).clamp(self.start_ms, self.end_ms.max(self.start_ms));
        self.skipped = true;
        if to == now {
            return Some(to);
        }
        self.judge.jump_to(self.judge_ms(to));
        self.move_clock(now, to);
        Some(to)
    }

    /// リプレイで、曲の時刻 `to` へ飛ぶ。判定を最初からやり直して `to` までの打撃を一度にかけ、
    /// 画面の効果をすべてためる（演奏の画面は作り直してから受け取る）。止めていれば止めたまま
    pub fn replay_jump(&mut self, to: f64) -> f64 {
        let Some(f) = self.replay.as_mut() else {
            return self.now_ms();
        };
        let to = to.clamp(
            self.start_ms,
            (f.end_ms + self.song_offset_ms).max(self.start_ms),
        );
        self.judge = f.fresh.clone();
        f.next_hit = 0;
        let song_offset = self.song_offset_ms;
        f.next_sound = f.takes.partition_point(|t| t.ms + song_offset < to);
        self.note_hits = vec![None; self.judge.notes().len()];
        self.strays.clear();
        self.hit_offsets.clear();
        self.fx.clear();
        self.catching_up = true;
        self.feed_replay(to);
        self.advance_to(self.judge_ms(to));
        self.catching_up = false;
        // 同じ所へ飛んでも、出力へ渡してある音は止めて渡し直す（次に渡す音を `to` に戻したので）
        let now = self.now_ms();
        self.move_clock(now, to);
        to
    }

    /// リプレイの今の判定の時刻（譜面の音符と同じ時刻）
    pub fn replay_ms(&self) -> f64 {
        self.judge_ms(self.now_ms())
    }

    /// 曲の時計を `now` から `to` へ動かし、飛んだ先で鳴らす音を出力に渡し直す
    fn move_clock(&mut self, now: f64, to: f64) {
        let d = to - now;
        if let Some(a) = self.audio.as_mut() {
            a.seek(d);
        }
        self.fallback_origin_us -= (d * 1000.0) as i64;
        if self.paused_ms.is_some() {
            self.paused_ms = Some(to);
        }
        // 飛んだ先でまだ鳴っているはずの自動の音（長い BGM など）は途中から鳴らし直し、その先は `lay_auto` に任せる
        self.auto_next = self.chart_sounds.auto.partition_point(|v| v.ms < to);
        self.click_next = self.click_lines.partition_point(|l| l.ms < to);
        if let Some(a) = self.audio.as_mut() {
            let rate = f64::from(a.rate());
            for v in &self.chart_sounds.auto[..self.auto_next] {
                let Some(s) = self.sounds.get(&v.wav) else {
                    continue;
                };
                let len =
                    (s.frames() as f64 / rate * 1000.0).min(v.len_ms.unwrap_or(f64::INFINITY));
                if v.ms + len > to {
                    let route = (Bus::Song, hat_tag(v.hat));
                    a.play_chip(s, v.ms, v.len_ms, (v.gain, v.pan), route);
                }
            }
        }
        self.lay_auto(to);
        self.lay_replay(to);
        log::info!("jumped from {now:.0}ms to {to:.0}ms");
    }

    /// 時間を進め、見逃しを判定する
    pub fn update(&mut self) {
        if self.paused() {
            return;
        }
        let now = self.now_ms();
        self.lay_laps(now);
        self.note_practice(now);
        self.lay_auto(now);
        self.lay_replay(now);
        self.feed_replay(now);
        // 見逃しは判定の時刻で決める（遅めに叩いても POOR の幅の中なら、見逃しより先に届いた打撃で取れるように）
        self.advance_to(self.judge_ms(now));
    }

    /// 判定の時刻 `ms` までに来た自動の音符を取り、判定の幅を過ぎた音符を見逃しにする
    fn advance_to(&mut self, ms: f64) {
        let events = self.judge.advance(ms);
        // 基礎練習で叩くのをやめたら、休み（一時停止の板を出す）
        if let Some(d) = self.drill.as_mut()
            && d.check_rest(ms)
        {
            self.practice_events.push(PracticeEvent::Rest);
        }
        // 自動で叩いた音符は、叩いたときと同じくレーンを光らせる（音は時刻を決めて鳴らしてある）
        for e in &events {
            let note = &self.judge.notes()[e.note];
            let lane = note.lane;
            if e.offset_ms.is_none() && self.judge.is_auto(lane) {
                if let Some(d) = self.drill.as_mut() {
                    d.on_hit(note.ms);
                }
                let rim = self.is_rim_note(note);
                self.push_fx(Fx::Hit {
                    lane,
                    velocity: AUTO_VELOCITY,
                    rim,
                });
            }
        }
        self.record(&events, None, None);
        self.show(&events);
    }

    /// リプレイなら、曲の時刻 `now` までの打撃を判定にかける（組み立てたときと同じく、打撃ごとにその時刻まで見逃しを進める）
    fn feed_replay(&mut self, now: f64) {
        let until = self.judge_ms(now);
        loop {
            let Some(f) = self.replay.as_mut() else {
                return;
            };
            let Some(&t) = f.takes.get(f.next_hit) else {
                return;
            };
            if t.ms > until {
                return;
            }
            f.next_hit += 1;
            self.advance_to(t.ms);
            self.strike(t, None);
        }
    }

    /// リプレイなら、叩いた音のうち `now`（曲の時刻）から少し先までのものを、叩いた時刻どおりに出力へ渡す
    fn lay_replay(&mut self, now: f64) {
        let (Some(f), Some(a)) = (self.replay.as_mut(), self.audio.as_mut()) else {
            return;
        };
        let until = now + AUTO_AHEAD_MS;
        while let Some(t) = f.takes.get(f.next_sound) {
            let at = t.ms + self.song_offset_ms;
            if at >= until {
                break;
            }
            let sound = f.sounds[f.next_sound];
            f.next_sound += 1;
            if !self.drum_sounds {
                continue;
            }
            let Some((wav, sound_lane)) = sound else {
                continue;
            };
            let (muting, tag) = hit_sound_tags(self.hit_sounds.muting_lp, sound_lane, t.lane);
            if muting {
                a.stop_at(HH_TAG, at);
            }
            if let Some(s) = self.sounds.get(&wav) {
                let (gain, pan) = self.chart_sounds.gain_pan(wav);
                let gain = gain * f32::from(t.velocity) / 127.0;
                a.play_chip(s, at, None, (gain, pan), (Bus::Hits, tag));
            }
        }
    }

    /// ポートから届いた MIDI（`msgs` は `raw.bytes` を読んだもの）
    pub fn on_midi(&mut self, raw: &RawMidi, msgs: &[MidiMessage]) {
        let Some((name, normalizer)) = self.inputs.get_mut(raw.port) else {
            return;
        };
        let mapper = self.mappers.entry(raw.port).or_default();
        let at_us = mapper.map(Some(raw.device_us), raw.arrival_us);
        let mut hits = Vec::new();
        let mut chokes = Vec::new();
        for &msg in msgs {
            match normalizer.feed(msg, at_us) {
                Some(InputEvent::Hit {
                    host_us,
                    lane,
                    pad,
                    articulation,
                    velocity,
                    hh_pedal,
                }) => {
                    self.last_input = Some(LastInput {
                        source: name.clone(),
                        pad: Some(pad),
                        articulation: Some(articulation),
                        lane,
                        velocity,
                        hh_pedal,
                    });
                    if let Some(lane) = lane {
                        hits.push((lane, velocity, host_us, articulation, hh_pedal));
                    }
                }
                Some(InputEvent::HiHatPedal { value, .. }) => self.hh_pedal = Some(value),
                Some(InputEvent::Choke { pad, .. }) => chokes.push(pad),
                None => {}
            }
        }
        for (lane, velocity, host_us, articulation, hh_pedal) in hits {
            let at = (host_us, raw.arrival_us);
            self.hit(lane, velocity, at, Some(articulation), hh_pedal);
        }
        for pad in chokes {
            self.choke(pad);
        }
    }

    /// シンバルのパッド `pad` を握った（Roland のチョーク）。そのパッドで叩いて鳴っている音を止める
    fn choke(&mut self, pad: Pad) {
        let tag = grip_tag(pad);
        if tag == 0 || self.paused() {
            return;
        }
        if let Some(a) = self.audio.as_mut() {
            a.stop_ringing(tag);
        }
    }

    /// キーボードで叩いた（電子ドラムが無いときの確認用）
    /// 音を止める（STAGE FAILED）。時計はシステム時刻になるが、もう譜面は進めない
    pub fn silence(&mut self) {
        self.audio = None;
        self.section_audio = None;
    }

    pub fn on_key(&mut self, lane: DrumLane, at_us: i64) {
        self.last_input = Some(LastInput {
            source: crate::inputs::keyboard().into(),
            pad: None,
            articulation: None,
            lane: Some(lane),
            velocity: 100,
            hh_pedal: None,
        });
        self.hit(lane, 100, (at_us, at_us), None, None);
    }

    /// `at_us` は叩いた時刻と、アプリに届いた時刻（ホスト時刻）
    fn hit(
        &mut self,
        lane: DrumLane,
        velocity: u8,
        (at_us, arrival_us): (i64, i64),
        articulation: Option<Articulation>,
        hh_pedal: Option<u8>,
    ) {
        // 上流と同じく、止めているあいだは叩いても何もしない。リプレイでは叩いても数えない
        if self.paused() || self.replay.is_some() {
            return;
        }
        let ms = self.judge_ms(self.song_ms_at(at_us));
        let timing = HitTiming {
            hit_us: at_us,
            arrival_us,
            handled_us: host_us(),
            sound_us: None,
        };
        self.strike(
            Take {
                ms,
                lane,
                velocity,
                articulation,
                hh_pedal,
                target: None,
            },
            Some(timing),
        );
    }

    /// 判定の時刻 `t.ms` に叩いた。`live` は叩いた時刻（遊んでいるとき）。リプレイなら None で、音は先に出力へ渡してある
    fn strike(&mut self, t: Take, live: Option<HitTiming>) {
        let Take {
            ms,
            lane,
            velocity,
            articulation,
            hh_pedal,
            ..
        } = t;
        if let Some(p) = self.practice.as_mut() {
            p.on_hit(ms);
        }
        if let Some(d) = self.drill.as_mut() {
            d.on_hit(ms);
        }
        let rim = lane == DrumLane::SD
            && matches!(
                articulation,
                Some(Articulation::Rim | Articulation::CrossStick)
            );
        self.push_fx(Fx::Hit {
            lane,
            velocity,
            rim,
        });
        let events = self.judge.hit(Hit { ms, lane, velocity });
        // 自動で叩くレーンを叩いても空打ちには数えない
        if events.is_empty() && !self.judge.is_auto(lane) {
            let stray = StrayHit {
                at_ms: ms,
                lane,
                velocity,
                articulation,
                hh_pedal,
            };
            match self.practice.as_mut() {
                Some(p) => p.on_stray(stray, self.judge.widest_poor()),
                None => self.strays.push(stray),
            }
            // 上流の Tight: MISS を出し、コンボを切り、ゲージを減らす（MISS の数には入れない）
            if self.tight {
                self.judge.break_combo();
                self.push_fx(Fx::StrayMiss { lane });
            }
        }
        self.record(&events, articulation, hh_pedal);
        let mut timing = live;
        if self.drum_sounds && live.is_some() {
            // 取った音符の音を鳴らす。空打ちなら空打ちの音、無ければそのレーンの一番近い音符の音。
            // `sound_lane` は鳴らす音のレーン（上流が音を止めるかを決めるチップのチャンネル）
            let (wav, sound_lane) = match events.first() {
                Some(e) => {
                    let (w, l) = self.hit_wav(lane, &self.judge.notes()[e.note], ms, articulation);
                    (Some(w), l)
                }
                None => (self.stray_wav(lane, ms, articulation), lane),
            };
            // 上流の Muting LP: 閉じたハイハットか左ペダルで、鳴っているハイハットの音を止める
            let (muting, tag) = hit_sound_tags(self.hit_sounds.muting_lp, sound_lane, lane);
            if let Some(a) = self.audio.as_mut() {
                if muting {
                    a.stop_ringing(HH_TAG);
                }
                if let Some(w) = wav
                    && let Some(s) = self.sounds.get(&w)
                {
                    let (gain, pan) = self.chart_sounds.gain_pan(w);
                    let gain = gain * f32::from(velocity) / 127.0;
                    a.play_hit(s, (gain, pan), tag, timing.take());
                }
            }
        }
        if let Some(t) = timing
            && self.silent_timings.len() < 256
        {
            self.silent_timings.push(t);
        }
        self.show(&events);
        // スネアの音符を違う所で叩いたら知らせる（判定はそのまま）
        if let (Some(a), Some(e)) = (articulation, events.first()) {
            let note = &self.judge.notes()[e.note];
            if note.lane == DrumLane::SD
                && let Some(want) = Stroke::of_note(self.is_rim_note(note)).wrong_for(a)
            {
                self.push_fx(Fx::WrongStroke { want });
            }
        }
    }

    /// 叩いた時刻と音が鳴り始めた時刻（FPS と遅延の表示へ）。取り出したものは消える
    pub fn take_hit_timings(&mut self) -> Vec<HitTiming> {
        let mut out = std::mem::take(&mut self.silent_timings);
        if let Some(a) = self.audio.as_mut() {
            out.extend(a.take_timings());
        }
        out
    }

    /// スネアのリムの音符か（音がリムの音らしい）
    pub fn is_rim_note(&self, note: &dtx::DrumNote) -> bool {
        self.is_rim(note.lane, note.wav)
    }

    /// レーン `lane` の音 `wav` の音符が、スネアのリムの音符か
    pub fn is_rim(&self, lane: DrumLane, wav: u16) -> bool {
        lane == DrumLane::SD && self.rim_wavs.contains(&wav)
    }

    /// 譜面にスネアのリムの音符がある
    pub fn has_rim_notes(&self) -> bool {
        !self.rim_wavs.is_empty()
    }

    /// 音符の無い所でレーン `lane` を叩いたときの音。上流と同じく、空打ちの音（0xB1〜）を決めてあればそれ、
    /// 無ければそのレーンの一番近い音符（見えない音符 0x31〜 も含む）の音。
    /// 練習は周ごとに時刻を並べ直すので、並べた音符のうち一番近いものの音にする
    /// スネアは、ヘッドならヘッドの音符の、クロススティックならリムの音符の一番近いものの音を先にする
    fn stray_wav(
        &self,
        lane: DrumLane,
        ms: f64,
        articulation: Option<Articulation>,
    ) -> Option<u16> {
        if self.practice.is_none()
            && let Some(w) = self.chart_sounds.empty_hit(lane, ms)
        {
            return Some(w);
        }
        let want = match (lane, articulation) {
            (DrumLane::SD, Some(Articulation::CrossStick)) if self.has_rim_notes() => {
                Some(Stroke::Rim)
            }
            (DrumLane::SD, Some(Articulation::Head)) if self.has_rim_notes() => Some(Stroke::Head),
            _ => None,
        };
        want.and_then(|w| self.snare_wav(w, ms))
            .or_else(|| self.lane_wav(lane, ms))
    }

    /// スネアの音符のうち、種類が `stroke` で曲の時刻 `ms` に一番近いものの音
    fn snare_wav(&self, stroke: Stroke, ms: f64) -> Option<u16> {
        self.judge
            .notes()
            .iter()
            .filter(|n| n.lane == DrumLane::SD && Stroke::of_note(self.is_rim_note(n)) == stroke)
            .min_by(|a, b| (a.ms - ms).abs().total_cmp(&(b.ms - ms).abs()))
            .map(|n| n.wav)
    }

    /// 叩いて音符 `note` を取ったときの音。叩いたパッドのレーン `lane` と音符のレーンが違い（叩き分けを共通にした）、
    /// パッドの音を先にする設定なら、そのパッドのレーンの一番近い音符の音（上流 `tProcessDrumHit` の P>C）。
    /// 譜面に無いレーンは、上流と同じく開いたハイハットと左シンバルを閉じたハイハット、ライドを右シンバルとして探す。
    /// スネアは叩いた所に合わせる（リムの音符をヘッドで叩いたら、一番近いヘッドの音符の音。[`Stroke::sound_for`]）。
    /// 返すのは音と、その音のレーン
    fn hit_wav(
        &self,
        lane: DrumLane,
        note: &dtx::DrumNote,
        ms: f64,
        articulation: Option<Articulation>,
    ) -> (u16, DrumLane) {
        use DrumLane::*;
        if note.lane == SD
            && let Some(a) = articulation
        {
            let stroke = Stroke::of_note(self.is_rim_note(note));
            let want = stroke.sound_for(a);
            if want != stroke {
                return (self.snare_wav(want, ms).unwrap_or(note.wav), SD);
            }
            return (note.wav, SD);
        }
        let own = (note.wav, note.lane);
        let family = match lane {
            HH | HHO | LC => 0,
            LT | FT => 1,
            CY | RD => 2,
            _ => return own,
        };
        if note.lane == lane || !self.hit_sounds.pad_first[family] {
            return own;
        }
        let has = |l: DrumLane| self.judge.notes().iter().any(|n| n.lane == l);
        let pad = match lane {
            HHO | LC if !has(lane) => HH,
            RD if !has(RD) => CY,
            l => l,
        };
        self.lane_wav(pad, ms).map_or(own, |w| (w, pad))
    }

    /// レーン `lane` の音符（見えない音符 0x31〜 も含む）のうち、曲の時刻 `ms` に一番近いものの音。
    /// 練習は周ごとに時刻を並べ直すので、並べた音符のうち一番近いものの音にする
    fn lane_wav(&self, lane: DrumLane, ms: f64) -> Option<u16> {
        let nearest = self
            .judge
            .notes()
            .iter()
            .filter(|n| n.lane == lane)
            .min_by(|a, b| (a.ms - ms).abs().total_cmp(&(b.ms - ms).abs()))
            .map(|n| (n.ms, n.wav));
        if self.practice.is_some() {
            return nearest.map(|(_, w)| w);
        }
        match (nearest, self.chart_sounds.nearest_hidden(lane, ms)) {
            (Some(n), Some(h)) => Some(if (h.0 - ms).abs() < (n.0 - ms).abs() {
                h.1
            } else {
                n.1
            }),
            (n, h) => n.or(h).map(|(_, w)| w),
        }
    }

    /// 自動で叩くレーンがある（ベストの記録には入れない）
    pub fn has_auto(&self) -> bool {
        self.judge.has_auto()
    }

    /// 叩いていた時間（ミリ秒）。曲の時刻 `end_ms` でやめたとして、始めから数える。
    /// 一時停止のあいだは曲の時刻が止まるので入らない。練習で叩かずに休んだ周も入れない。
    /// 全部自動のときと、音符を 1 つも叩かなかったときは None（練習した時間に数えない）
    pub fn played_ms(&self, end_ms: f64) -> Option<i64> {
        let touched = self
            .note_hits
            .iter()
            .flatten()
            .any(|h| h.offset_ms.is_some());
        if self.all_auto() || !touched {
            return None;
        }
        let ms = match &self.practice {
            None => end_ms.min(self.end_ms) - self.start_ms,
            Some(p) => {
                let rest: f64 = p
                    .laps
                    .iter()
                    .filter(|l| l.rest)
                    .map(|l| l.plan.end_ms - l.plan.start_ms)
                    .sum();
                end_ms - self.start_ms - rest
            }
        };
        Some(ms.max(0.0).round() as i64)
    }

    /// 譜面の音符をすべて自動で叩く
    fn all_auto(&self) -> bool {
        self.judge.has_auto()
            && self
                .judge
                .notes()
                .iter()
                .all(|n| self.judge.is_auto(n.lane))
    }

    /// 今までの成績
    pub fn outcome(&self) -> Outcome {
        let (mean, sd) = mean_and_sd(&self.hit_offsets);
        // 練習で叩かずに休んだ周は、結果の数に入れない
        let (tally, notes) = match &self.practice {
            Some(p) => p.without_rest(self.judge.tally(), self.judge.notes().len()),
            None => (self.judge.tally().clone(), self.judge.notes().len()),
        };
        Outcome {
            path: self.path.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            level: self.level,
            notes,
            tally,
            offset_mean_ms: mean,
            offset_sd_ms: sd,
            offsets: self.hit_offsets.clone(),
            windows: self.judge.windows(),
            lag: None,
            cleared: self.finished(),
            notes_per_measure: self.notes_per_measure.clone(),
            results: self
                .note_hits
                .iter()
                .flatten()
                .map(|h| NoteResult {
                    measure: h.at.measure,
                    judgement: h.judgement,
                })
                .collect(),
            weak: None,
            overcome: Vec::new(),
            tendencies: Vec::new(),
            started_at: self.started_at,
            strokes: self.stroke_tally(),
            session_id: None,
            practice: self.practice.as_ref().map(|p| PracticeOutcome {
                options: p.options,
                laps: p.summaries(self.now_ms()),
                attempts: p.attempts(),
                elapsed_ms: p
                    .laps
                    .first()
                    .map_or(0.0, |l| (self.now_ms() - l.plan.audio.at_ms).max(0.0)),
                cleared: p.cleared(),
                strays: p.stray_hint(),
                offset: p.offset_mean(),
                picture: p.picture.clone(),
            }),
            drill: self.drill_outcome(),
        }
    }

    /// 基礎練習の今までの成績（基礎練習でなければ None）
    pub fn drill_outcome(&self) -> Option<DrillOutcome> {
        let d = self.drill.as_ref()?;
        let now = self.judge_ms(self.now_ms());
        let results: Vec<engine::drill::NoteOutcome> = self
            .note_hits
            .iter()
            .flatten()
            .map(|h| engine::drill::NoteOutcome {
                at: h.at,
                lane: h.lane,
                judgement: h.judgement,
                offset_ms: h.offset_ms,
            })
            .collect();
        // 限界チャレンジで終えた段は、最後まで叩いていない段として数える（出来は 3 つ落とした × に決まる）
        let now_measure = match d.stopped {
            Some((step, _)) => d.measure_at(now).min(d.run.step_range(step).start),
            None => d.measure_at(now),
        };
        let summary = engine::drill::summarize(&d.run, &results, now_measure);
        Some(DrillOutcome {
            run: d.run.clone(),
            summary,
            elapsed_ms: (now - d.measure_start_ms(0)).max(0.0),
            best_before: None,
        })
    }

    /// 判定の結果を音符ごとの記録に書く
    fn record(
        &mut self,
        events: &[JudgeEvent],
        articulation: Option<Articulation>,
        hh_pedal: Option<u8>,
    ) {
        if let Some(p) = self.practice.as_mut() {
            p.on_events(events);
        }
        for e in events {
            let note = &self.judge.notes()[e.note];
            // 自動で叩いた音符は、苦手やくせを出す記録に入れない
            if self.judge.is_auto(note.lane) {
                continue;
            }
            let beat_pos = self
                .measure_beats
                .get(note.at.measure as usize)
                .map(|start| (note.beat - start).rem_euclid(1.0));
            self.note_hits[e.note] = Some(NoteHit {
                at: note.at,
                lane: note.lane,
                judgement: e.judgement,
                offset_ms: e.offset_ms,
                velocity: e.velocity,
                articulation,
                hh_pedal,
                beat_pos,
            });
        }
        // 限界チャレンジは、同じ段で 3 つ落としたらそこで終える
        let mut stop = false;
        if let Some(d) = self.drill.as_mut() {
            for e in events {
                let note = &self.judge.notes()[e.note];
                if !self.judge.is_auto(note.lane)
                    && d.on_judged(note.at.measure, e.judgement, note.ms)
                {
                    stop = true;
                }
            }
        }
        if stop {
            self.stop_drill();
        }
    }

    /// 限界チャレンジを終える。残りの音符は判定せず、クリックを止め、少し置いて結果へ
    fn stop_drill(&mut self) {
        let now = self.now_ms();
        self.end_ms = self.end_ms.min(now + DRILL_STOP_MS);
        self.judge.jump_to(f64::INFINITY);
        if let Some(a) = self.audio.as_mut() {
            a.stop(METRONOME_TAG);
        }
    }

    /// 記録に残す形にする。何も叩かず何も過ぎていなければ None（すぐやめたときは残さない）。
    /// 音符をすべて自動で叩いたときと、早送りか巻き戻しをしたときも残さない（上流と同じ）
    pub fn session(&self) -> Option<Session> {
        if self.all_auto() || self.skipped || self.replay.is_some() {
            return None;
        }
        let w = self.judge.windows();
        // 設定は JSON で残す（数は有限なので Display のままで JSON の数になる）
        let mut settings = format!(
            "{{\"input_offset_ms\":{},\"windows_ms\":[{},{},{},{}],\"drum_sounds\":{}",
            self.input_offset_ms, w.perfect, w.great, w.good, w.poor, self.drum_sounds
        );
        if self.song_offset_ms != 0.0 {
            settings += &format!(",\"song_offset_ms\":{}", self.song_offset_ms);
        }
        let pw = self.judge.pedal_windows();
        if pw != w {
            settings += &format!(
                ",\"pedal_windows_ms\":[{},{},{},{}]",
                pw.perfect, pw.great, pw.good, pw.poor
            );
        }
        if self.tight {
            settings += ",\"tight\":true";
        }
        let groups = self.judge.groups();
        if groups != LaneGroups::default() {
            settings += &format!(",\"groups\":\"{groups:?}\"");
        }
        let g = self.gauge;
        if g != GaugeRule::default() {
            settings += &format!(
                ",\"gauge\":{{\"miss_factor\":{},\"risky\":{},\"hazard\":{}}}",
                g.miss_factor, g.risky, g.hazard
            );
        }
        let auto: Vec<String> = DrumLane::ALL
            .into_iter()
            .filter(|&l| self.judge.is_auto(l))
            .map(|l| format!("\"{}\"", l.name()))
            .collect();
        if !auto.is_empty() {
            settings += &format!(",\"auto\":[{}]", auto.join(","));
        }
        if let Some(p) = &self.practice {
            let o = p.options;
            let speed = match o.speed {
                SpeedPlan::Fixed(s) => format!("{{\"plan\":\"fixed\",\"speed\":{s}}}"),
                SpeedPlan::Ladder { start, target } => {
                    format!("{{\"plan\":\"ladder\",\"start\":{start},\"target\":{target}}}")
                }
            };
            settings += &format!(
                ",\"section\":[{},{}],\"style\":\"{}\",\"rule\":\"{}\",\"speed\":{},\"pitch\":\"{}\",\"metronome\":{},\"step_laps\":{},\"attempts\":{},\"cleared\":{}",
                o.section.first,
                o.section.last,
                o.style.name(),
                o.rule.name(),
                speed,
                o.pitch.name(),
                o.metronome,
                o.step_laps,
                p.attempts().len(),
                p.cleared()
            );
        }
        if let Some(d) = &self.drill {
            let run = &d.run;
            let bpms: Vec<String> = run.bpms.iter().map(u32::to_string).collect();
            settings += &format!(
                ",\"drill\":\"{}\",\"way\":\"{}\",\"lefty\":{},\"bpms\":[{}],\"target_bpm\":{}",
                run.drill.id,
                run.way.key(),
                run.lefty,
                bpms.join(","),
                run.target()
            );
            if let Some(best) = self.drill_outcome().and_then(|o| o.best_now()) {
                settings += &format!(",\"best_bpm\":{best}");
            }
        }
        settings.push('}');
        if self.drill.is_some() {
            let laps = self.drill_laps();
            if laps.is_empty() {
                return None;
            }
            return Some(Session {
                kind: SessionKind::Drill,
                started_at: self.started_at,
                settings,
                laps,
                clear: None,
                score: None,
            });
        }
        let laps = match &self.practice {
            None => {
                let hits: Vec<NoteHit> = self.note_hits.iter().flatten().cloned().collect();
                if hits.is_empty() && self.strays.is_empty() {
                    return None;
                }
                let o = self.outcome();
                vec![Lap {
                    speed: 1.0,
                    completed: o.cleared,
                    passed: None,
                    tally: o.tally,
                    offset_mean_ms: o.offset_mean_ms,
                    offset_sd_ms: o.offset_sd_ms,
                    hits,
                    strays: self.strays.clone(),
                }]
            }
            // 練習は最後まで叩いた周だけを残す（途中でやめた周の見逃しは苦手のせいではないので）
            Some(p) => p
                .laps
                .iter()
                .filter(|l| l.complete() && !l.notes.is_empty() && !l.rest)
                .map(|l| {
                    let s = l.summary();
                    Lap {
                        speed: s.speed,
                        completed: s.completed,
                        passed: s.passed,
                        tally: s.tally,
                        offset_mean_ms: s.offset_mean_ms,
                        offset_sd_ms: s.offset_sd_ms,
                        hits: self.note_hits[l.notes.clone()]
                            .iter()
                            .flatten()
                            .cloned()
                            .collect(),
                        strays: l.strays.clone(),
                    }
                })
                .collect(),
        };
        if laps.is_empty() {
            return None;
        }
        Some(Session {
            kind: if self.practice.is_some() {
                SessionKind::Practice
            } else {
                SessionKind::Play
            },
            started_at: self.started_at,
            settings,
            laps,
            // 等速以上で区間全体を苦手なしで叩き終えたら、その区間をクリアとして残す（普通のプレイで克服したかを見る）
            clear: self.practice.as_ref().and_then(|p| {
                p.clear_speed().map(|speed| SectionClear {
                    first: p.options.section.first,
                    last: p.options.section.last,
                    speed: f64::from(speed) / 100.0,
                })
            }),
            // スコアは演奏の画面が数えている（History::save で入れる）
            score: None,
        })
    }

    /// 基礎練習の段ごとの周（音符を 1 つでも判定した段だけ）。速度は目標のテンポに対する段のテンポ
    fn drill_laps(&self) -> Vec<Lap> {
        let (Some(d), Some(o)) = (&self.drill, self.drill_outcome()) else {
            return Vec::new();
        };
        let target = f64::from(d.run.target().max(1));
        o.summary
            .steps
            .iter()
            .enumerate()
            .filter(|(_, s)| s.notes > 0)
            .map(|(i, s)| {
                let measures = d.step_measures(i);
                Lap {
                    speed: f64::from(s.bpm) / target,
                    completed: s.completed,
                    passed: s.grade().map(|g| g == engine::drill::Grade::Clean),
                    tally: s.tally.clone(),
                    offset_mean_ms: s.offset_mean_ms,
                    offset_sd_ms: s.offset_sd_ms,
                    hits: self
                        .note_hits
                        .iter()
                        .flatten()
                        .filter(|h| measures.contains(&h.at.measure))
                        .cloned()
                        .collect(),
                    strays: self
                        .strays
                        .iter()
                        .filter(|x| measures.contains(&d.measure_at(x.at_ms)))
                        .cloned()
                        .collect(),
                }
            })
            .collect()
    }

    /// スネアの打ち分けの数。譜面にリムの音符が無いか、電子ドラムでスネアを叩いていなければ `None`
    fn stroke_tally(&self) -> Option<StrokeTally> {
        if !self.has_rim_notes() {
            return None;
        }
        let mut t = StrokeTally::default();
        for (note, hit) in self.judge.notes().iter().zip(&self.note_hits) {
            let Some(h) = hit else { continue };
            if note.lane != DrumLane::SD || h.offset_ms.is_none() {
                continue;
            }
            if let Some(a) = h.articulation {
                t.add(Stroke::of_note(self.is_rim_note(note)), a, note.at.measure);
            }
        }
        (t.rim_hit + t.head_hit > 0).then_some(t)
    }

    fn show(&mut self, events: &[JudgeEvent]) {
        for e in events {
            if let Some(o) = e.offset_ms {
                self.hit_offsets.push(o);
            }
            let note = &self.judge.notes()[e.note];
            let fx = Fx::Judged {
                lane: note.lane,
                judgement: e.judgement,
                offset_ms: e.offset_ms,
                note_ms: note.ms,
            };
            self.push_fx(fx);
        }
    }
}

/// 小節線と拍線を並べる（上流 `CDTX` の「拍子_拍線の挿入」と同じく、曲の終わりの小節まで小節線を引き、
/// 小節の中は 1 拍ごとに拍線を引く。拍線のずらし（C1）と表示の切り替え（C2）はまだ読まない）
fn beat_lines(tempo: &TempoMap) -> Vec<BeatLine> {
    let measures = tempo.measure_count();
    let mut out = Vec::new();
    for m in 0..=measures {
        let start = tempo.measure_start_beat(m);
        out.push(BeatLine {
            ms: tempo.beat_to_ms(start),
            bar: true,
        });
        if m == measures {
            break;
        }
        let len = tempo.measure_start_beat(m + 1) - start;
        let mut i = 1.0;
        while i < len - 1e-9 {
            out.push(BeatLine {
                ms: tempo.beat_to_ms(start + i),
                bar: false,
            });
            i += 1.0;
        }
    }
    out
}

/// 譜面の `#PREIMAGE` の画像のファイル（大文字小文字の違いは許す）
fn preimage_path(chart: &Chart, path: &Path) -> Option<PathBuf> {
    let name = chart.field("PREIMAGE")?.trim();
    if name.is_empty() {
        return None;
    }
    nx::find_file(path.parent()?, name).filter(|p| p.is_file())
}

/// 動画を流し始めるチップ（0x54 と 0x5A。上流 `EChannel.Movie`・`MovieFull`）の時刻と、`#AVIzz` のファイル。
/// ファイルが無いチップは飛ばす（前の動画が流れたまま）
fn movie_cues(chart: &Chart, tempo: &TempoMap, path: &Path) -> Vec<(f64, PathBuf)> {
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let mut found: HashMap<u16, Option<PathBuf>> = HashMap::new();
    let mut cues: Vec<(f64, PathBuf)> = chart
        .chips
        .iter()
        .filter(|c| matches!(c.channel, dtx::ch::MOVIE | dtx::ch::MOVIE_FULL))
        .filter_map(|c| {
            let file = found.entry(c.value).or_insert_with(|| {
                let name = chart.avi(c.value)?.trim();
                let file = nx::find_file(dir, name).filter(|p| !name.is_empty() && p.is_file());
                if file.is_none() {
                    log::warn!("{}: 動画 {name} がありません", path.display());
                }
                file
            });
            Some((tempo.chip_ms(c), file.clone()?))
        })
        .collect();
    cues.sort_by(|a, b| a.0.total_cmp(&b.0));
    cues
}

/// 今の UNIX 秒
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// お手本の音（作った音）の番号
fn demo_wav(lane: DrumLane) -> u16 {
    DEMO_WAV + u16::from(lane.channel())
}

/// 譜面が使う音（叩く音符、自動で鳴らす音、空打ちの音、見えない音符）をすべて読む。読めない音は飛ばす
pub(crate) fn load_sounds(
    chart: &Chart,
    chart_sounds: &ChartSounds,
    dir: &Path,
    rate: u32,
) -> HashMap<u16, Sound> {
    let used: BTreeSet<u16> = chart
        .chips
        .iter()
        .filter(|c| DrumLane::from_channel(c.channel).is_some())
        .map(|c| c.value)
        .chain(chart_sounds.wavs())
        .collect();
    let mut out = HashMap::new();
    for id in used {
        let Some(name) = chart.wav(id) else { continue };
        let path = dir.join(name.replace('\\', "/"));
        match load_sound(&path, rate) {
            Ok(s) => {
                out.insert(id, s);
            }
            Err(e) => log::warn!("sound {}: {e:#}", path.display()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movie_cues_find_the_files() {
        let dir = std::env::temp_dir().join(format!("nxt-movies-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Movie.MP4"), b"").unwrap();
        let path = dir.join("chart.dtx");
        // BPM 120 で 1 小節 2 秒。2 小節目の頭と半ばで流し直し、無いファイルのチップは飛ばす
        let text = "#BPM 120\n#AVI01: movie.mp4\n#AVI02: missing.avi\n#00154: 0102\n#00254: 01\n#0025A: 0001\n";
        std::fs::write(&path, text).unwrap();
        let play = Play::load(&path, 0.0, false, &Rules::default()).unwrap();
        // 大文字小文字を区別しないファイルシステム（Windows・macOS）では、譜面に書いた綴りのまま見つかる
        let found = play
            .movies
            .iter()
            .all(|(_, p)| p.parent() == Some(&*dir) && p.is_file());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(found, "{:?}", play.movies);
        let times: Vec<f64> = play.movies.iter().map(|m| m.0).collect();
        assert_eq!(times, [2000.0, 4000.0, 5000.0]);
        assert!(
            play.movies
                .iter()
                .all(|(_, p)| p.file_name().unwrap().eq_ignore_ascii_case("movie.mp4"))
        );
    }

    /// 譜面 `text` を一時ファイルに書いて読む（音は鳴らさない）
    fn load_text(name: &str, text: &str, rules: &Rules) -> Play {
        let path = std::env::temp_dir().join(format!("nxt-{name}-{}.dtx", std::process::id()));
        std::fs::write(&path, text).unwrap();
        let play = Play::load(&path, 0.0, false, rules);
        std::fs::remove_file(&path).unwrap();
        play.unwrap()
    }

    #[test]
    fn a_saved_play_replays_with_the_same_judgements() {
        // BPM 120 で 1 小節 2000ms。HH は 8 分、SD は 2・4 拍
        let text = "#BPM 120\n#WAV01 hh.ogg\n#WAV02 sd.ogg\n\
                    #00111: 0101010101010101\n#00112: 00020002\n\
                    #00211: 0101010101010101\n#00212: 00020002\n";
        let path = std::env::temp_dir().join(format!("nxt-replay-{}.dtx", std::process::id()));
        std::fs::write(&path, text).unwrap();
        let rules = Rules {
            tight: true,
            ..Rules::default()
        };
        let mut p = Play::load(&path, 0.0, false, &rules).unwrap();
        let notes = p.judge.notes().to_vec();
        let take = |ms: f64, lane: DrumLane| Take {
            ms,
            lane,
            velocity: 90,
            articulation: None,
            hh_pedal: None,
            target: None,
        };
        // 遊んだつもりで叩く（5 つに 1 つは抜かし、早め・遅めを混ぜ、空打ちも入れる）
        let mut inputs: Vec<Take> = notes
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 5 != 0)
            .map(|(i, n)| take(n.ms + [-20.0, 8.0, 45.0, -70.0][i % 4], n.lane))
            .collect();
        inputs.push(take(4100.0, DrumLane::FT));
        inputs.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        for t in inputs {
            p.advance_to(t.ms);
            p.strike(t, None);
        }
        p.advance_to(f64::INFINITY);
        let tally = p.judge.tally().clone();
        assert!(tally.miss > 0 && tally.stray == 1 && tally.great > 0);
        let session = p.session().unwrap();
        let lap = &session.laps[0];
        let rec = records::PlayTakes {
            entry: records::PlayEntry {
                session_id: 1,
                started_at: 0,
                chart_hash: Some(p.chart_hash),
                tally: lap.tally.clone(),
                completed: true,
                score: None,
            },
            chart_path: path.to_string_lossy().into_owned(),
            settings: session.settings.clone(),
            hits: lap.hits.clone(),
            strays: lap.strays.clone(),
        };
        // 今の設定がどうでも、遊んだときの決まりで判定し直す
        let mut now_rules = Rules {
            auto: vec![DrumLane::HH],
            ..Rules::default()
        };
        crate::replay::RecordedRules::parse(&session.settings).apply(&mut now_rules);
        let mut r = Play::load_replay(&path, &now_rules, 0.0, &rec).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(r.session().is_none());
        assert_eq!(r.replay().unwrap().takes.len(), notes.len() - 4 + 1);
        let end = r.replay_jump(f64::MAX);
        assert!(end > notes.last().unwrap().ms);
        assert_eq!(r.judge.tally(), &tally);
        let judged = r
            .take_fx()
            .iter()
            .filter(|f| matches!(f, Fx::Judged { .. }))
            .count();
        assert_eq!(judged, notes.len());
        // 戻ればまた最初から判定する
        r.replay_jump(0.0);
        assert_eq!(r.judge.tally().perfect + r.judge.tally().great, 0);
        // 叩いた音は、取った音符の音（空打ちは一番近い音符の音、FT には無い）
        let f = r.replay().unwrap();
        assert!(f.sounds.iter().filter(|s| s.is_some()).count() == notes.len() - 4);
    }

    #[test]
    fn pad_sound_first_plays_the_pad_lane() {
        // BPM 60 で 1 小節 4000ms。HH（wav 01）は 4000ms と 12000ms、LC（wav 02）は 7500ms
        let text = "#BPM 60\n#00111: 01\n#0011A: 0000000000000002\n#00311: 01\n";
        let common = LaneGroups {
            hh: engine::judge::HhGroup::Common,
            ..Default::default()
        };
        let chip_first = load_text(
            "chip-first",
            text,
            &Rules {
                groups: common,
                ..Default::default()
            },
        );
        let hh = chip_first.judge.notes()[0].clone();
        assert_eq!(hh.lane, DrumLane::HH);
        // 左シンバルで閉じたハイハットの音符を取っても、ふだん（C>P）は譜面の音（音のレーンも譜面の音符）
        assert_eq!(
            chip_first.hit_wav(DrumLane::LC, &hh, 4000.0, None),
            (1, DrumLane::HH)
        );
        let pad_first = load_text(
            "pad-first",
            text,
            &Rules {
                groups: common,
                hit_sounds: HitSoundRule {
                    pad_first: [true, false, false],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        // P>C なら叩いた左シンバルのレーンの音
        assert_eq!(
            pad_first.hit_wav(DrumLane::LC, &hh, 4000.0, None),
            (2, DrumLane::LC)
        );
        // 譜面に開いたハイハットが無ければ、閉じたハイハットの音
        assert_eq!(
            pad_first.hit_wav(DrumLane::HHO, &hh, 4000.0, None),
            (1, DrumLane::HH)
        );
        // 同じレーンなら譜面の音のまま
        assert_eq!(pad_first.hit_wav(DrumLane::HH, &hh, 4000.0, None).0, 1);
    }

    #[test]
    fn closed_hats_and_the_left_pedal_stop_the_ringing_hats() {
        use DrumLane::*;
        // 閉じたハイハットと左ペダルの音は、鳴らす前に鳴っているハイハットの音を止める（上流の Muting LP）
        assert_eq!(hit_sound_tags(true, HH, HH), (true, HH_TAG));
        assert_eq!(hit_sound_tags(true, LP, LP), (true, 0));
        // 開いたハイハットは止めず、自分は後で止められる
        assert_eq!(hit_sound_tags(true, HHO, HHO), (false, HH_TAG));
        assert_eq!(hit_sound_tags(true, SD, SD), (false, 0));
        // 決めるのは鳴らす音のレーン（開いたハイハットの音符を閉じたハイハットで取ったら止めない）
        assert_eq!(hit_sound_tags(true, HHO, HH), (false, HH_TAG));
        assert_eq!(hit_sound_tags(true, HH, HHO), (true, HH_TAG));
        // 左シンバルで閉じたハイハットの音を鳴らしたら、ハイハットとして止め、左シンバルを握っても止まる
        assert_eq!(hit_sound_tags(true, HH, LC), (true, HH_TAG | LC_TAG));
        // Muting LP が OFF なら止めない
        assert_eq!(hit_sound_tags(false, HH, HH), (false, HH_TAG));
        assert_eq!(hit_sound_tags(false, LP, LP), (false, 0));
    }

    #[test]
    fn gripping_a_cymbal_stops_what_that_pad_hit() {
        // Roland の工場出荷時のノートで、握ったパッドの目印と、そのパッドで叩いた音の目印が同じ
        let p = MidiProfile::roland_vdrums();
        for note in [49, 55, 57, 52, 51, 59, 53] {
            let lane = p.note_lane(note).unwrap();
            assert_ne!(choke_tag(lane), 0, "{note}");
            assert_eq!(grip_tag(p.notes[&note].pad), choke_tag(lane), "{note}");
        }
        // ハイハットを握ったらハイハットの音、シンバルでないパッドは何もしない
        assert_eq!(grip_tag(Pad::HiHat), HH_TAG);
        assert_eq!(grip_tag(Pad::Snare), 0);
        // 目印はビットの組なので重ならない
        let tags = [HH_TAG, METRONOME_TAG, CY_TAG, LC_TAG, RD_TAG];
        for (i, a) in tags.iter().enumerate() {
            for b in &tags[i + 1..] {
                assert_eq!(a & b, 0);
            }
        }
    }

    #[test]
    fn snare_sounds_and_strokes_follow_where_you_hit() {
        // BPM 60 で 1 小節 4000ms。スネアは 4000ms（ヘッド 01）、5000ms（リム 02）、6000ms（ヘッド 01）、
        // 8000ms（リム 03、注釈で見分ける）
        let text = "#BPM 60\n#WAV01 snare.ogg\n#WAV02 Rim_37.ogg\n#WAV03 sn.ogg ;Snare Rim\n\
                    #00112: 01020100\n#00212: 03\n";
        let mut p = load_text("snare-rim", text, &Rules::default());
        assert!(p.has_rim_notes());
        let notes = p.judge.notes().to_vec();
        let rims: Vec<bool> = notes.iter().map(|n| p.is_rim_note(n)).collect();
        assert_eq!(rims, [false, true, false, true]);
        use Articulation::*;
        // リムの音符をリムで叩いたらリムの音、ヘッドで叩いたら一番近いヘッドの音符の音
        assert_eq!(p.hit_wav(DrumLane::SD, &notes[1], 5000.0, Some(Rim)).0, 2);
        assert_eq!(p.hit_wav(DrumLane::SD, &notes[1], 5000.0, Some(Head)).0, 1);
        // 普通の音符のリムショットはそのまま、クロススティックなら一番近いリムの音符の音
        assert_eq!(p.hit_wav(DrumLane::SD, &notes[2], 6000.0, Some(Rim)).0, 1);
        assert_eq!(
            p.hit_wav(DrumLane::SD, &notes[2], 6000.0, Some(CrossStick)),
            (2, DrumLane::SD)
        );
        assert_eq!(
            p.hit_wav(DrumLane::SD, &notes[2], 7900.0, Some(CrossStick)),
            (3, DrumLane::SD)
        );
        // キーボードは譜面の音のまま
        assert_eq!(p.hit_wav(DrumLane::SD, &notes[1], 5000.0, None).0, 2);
        // 空打ちも叩いた所に合わせる
        assert_eq!(p.stray_wav(DrumLane::SD, 5100.0, Some(Head)), Some(1));
        assert_eq!(p.stray_wav(DrumLane::SD, 5900.0, Some(CrossStick)), Some(2));
        assert_eq!(p.stray_wav(DrumLane::SD, 5900.0, Some(Rim)), Some(1));

        // 電子ドラムで叩いた音符だけを数える
        assert_eq!(p.stroke_tally(), None);
        let hit = |n: &dtx::DrumNote, a: Option<Articulation>| {
            Some(NoteHit {
                at: n.at,
                lane: n.lane,
                judgement: Judgement::Perfect,
                offset_ms: Some(0.0),
                velocity: Some(100),
                articulation: a,
                hh_pedal: None,
                beat_pos: None,
            })
        };
        p.note_hits[0] = hit(&notes[0], Some(Head));
        p.note_hits[1] = hit(&notes[1], Some(Head));
        p.note_hits[2] = hit(&notes[2], None);
        p.note_hits[3] = hit(&notes[3], Some(CrossStick));
        let t = p.stroke_tally().unwrap();
        assert_eq!((t.rim_hit, t.rim_ok, t.head_hit, t.head_ok), (2, 1, 1, 1));
        assert_eq!(t.rim_missed_measures, vec![1]);

        // リムの音が無い譜面では数えない
        let plain = load_text(
            "snare-plain",
            "#BPM 60\n#WAV01 snare.ogg\n#00112: 01\n",
            &Rules::default(),
        );
        assert!(!plain.has_rim_notes());
    }

    #[test]
    fn the_song_offset_adds_to_the_input_offset() {
        let mut p = load_text("song-offset", "#BPM 60\n#00111: 01\n", &Rules::default());
        p.input_offset_ms = 3.0;
        p.song_offset_ms = 10.0;
        // 合わせて 13 ms 遅く叩くのがちょうど
        assert_eq!(p.judge_ms(4013.0), 4000.0);
    }

    #[test]
    fn drills_play_a_made_chart_with_the_click_on() {
        use engine::drill::{Drill, Run};
        let rules = Rules {
            metronome: false,
            tight: true,
            ..Default::default()
        };
        let run = Run::new(Drill::find("paradiddle").unwrap(), 100, 10);
        let mut play = Play::load_drill(run.clone(), 0.0, false, &rules).unwrap();
        play.audio = None;
        // 1 小節 16 打 × 4 段 × 4 小節。クリックはいつも鳴らし、空打ちの MISS は使わない
        assert_eq!(play.judge.notes().len(), 16 * 16);
        assert!(play.metronome());
        assert!(!play.tight);
        assert_eq!(play.path, crate::drill::chart_path(run.drill));
        assert_eq!(play.seek(1000.0), None);
        // 記録ではテンポが違っても同じ譜面
        let faster = Play::load_drill(Run::new(run.drill, 120, 10), 0.0, false, &rules).unwrap();
        assert_eq!(faster.chart_hash, play.chart_hash);
        // 何も叩いていなければ残さない
        assert!(play.session().is_none());
        // 最初の段（70）をミスなしで叩き、最後まで時間を進める
        let first: Vec<(f64, DrumLane)> = play
            .judge
            .notes()
            .iter()
            .filter(|n| (1..5).contains(&n.at.measure))
            .map(|n| (n.ms, n.lane))
            .collect();
        for (ms, lane) in first {
            let events = play.judge.hit(Hit {
                ms: ms + 4.0,
                lane,
                velocity: 100,
            });
            play.record(&events, None, None);
        }
        play.fallback_origin_us = host_us() - 120_000_000;
        play.update();
        assert!(play.finished());
        let o = play.drill_outcome().unwrap();
        assert_eq!(o.best_now(), Some(70));
        let session = play.session().unwrap();
        assert_eq!(session.kind, SessionKind::Drill);
        assert!(session.score.is_none());
        assert!(session.settings.contains("\"drill\":\"paradiddle\""));
        assert!(session.settings.contains("\"best_bpm\":70"));
        // 段ごとの周。叩いていない段は見逃しだけなので入る（16 打の MISS）
        assert_eq!(session.laps.len(), 4);
        assert_eq!(session.laps[0].speed, 0.7);
        assert_eq!(session.laps[0].passed, Some(true));
        assert_eq!(session.laps[0].hits.len(), 64);
        assert_eq!(session.laps[1].passed, Some(false));
    }

    /// 時計を曲の時刻 `ms` に合わせる（音が出ないときの時計）
    fn set_clock(play: &mut Play, ms: f64) {
        play.fallback_origin_us = host_us() - ((ms - play.start_ms) * 1000.0) as i64;
    }

    #[test]
    fn the_limit_test_stops_after_three_misses_in_one_step() {
        use engine::drill::{Drill, Grade, Run, Way};
        let run = Run::with(Drill::find("quarter").unwrap(), Way::Limit, 100, 10, false);
        let mut play = Play::load_drill(run, 0.0, false, &Rules::default()).unwrap();
        play.audio = None;
        let full_end = play.end_ms;
        // 1 段目（小節 1・2、100 BPM）はミスなし
        let first: Vec<(f64, DrumLane)> = play
            .judge
            .notes()
            .iter()
            .filter(|n| (1..3).contains(&n.at.measure))
            .map(|n| (n.ms, n.lane))
            .collect();
        assert_eq!(first.len(), 8);
        for (ms, lane) in first {
            let events = play.judge.hit(Hit {
                ms: ms + 3.0,
                lane,
                velocity: 100,
            });
            play.record(&events, None, None);
        }
        // 2 段目（小節 3・4、105 BPM）は叩かない。3 つ目の見逃しで終わる
        let m3 = play.drill.as_ref().unwrap().measure_start_ms(3);
        let beat = 60_000.0 / 105.0;
        set_clock(&mut play, m3 + beat);
        play.update();
        assert!(play.drill.as_ref().unwrap().stopped.is_none());
        set_clock(&mut play, m3 + 2.0 * beat + 200.0);
        play.update();
        let d = play.drill.as_ref().unwrap();
        assert_eq!(d.stopped.map(|(s, _)| s), Some(1));
        assert!(play.end_ms < full_end);
        // 残りの音符は判定しない（見逃しを数えない）
        set_clock(&mut play, m3 + 6.0 * beat);
        play.update();
        let judged = play.note_hits.iter().flatten().count();
        assert_eq!(judged, 11);
        let o = play.drill_outcome().unwrap();
        assert_eq!(o.best_now(), Some(100));
        assert_eq!(o.summary.steps[0].grade(), Some(Grade::Clean));
        assert_eq!(o.summary.steps[1].grade(), Some(Grade::Many));
        assert_eq!(o.summary.steps[2].notes, 0);
        let session = play.session().unwrap();
        assert!(session.settings.contains("\"way\":\"limit\""));
        assert!(session.settings.contains("\"lefty\":false"));
        assert_eq!(session.laps.len(), 2);
        assert!(play.finished());
    }

    #[test]
    fn call_and_response_plays_the_heard_bars_without_notes() {
        use engine::drill::{Drill, Run, Way};
        let run = Run::with(Drill::find("eight_beat").unwrap(), Way::Echo, 100, 10, true);
        let heard = run.demo_notes().len();
        let mut play = Play::load_drill(run, 0.0, false, &Rules::default()).unwrap();
        play.audio = None;
        // お手本の音は自動で鳴らす音として並び、譜面の音符は自分の小節だけ
        let demo: Vec<_> = play
            .chart_sounds
            .auto
            .iter()
            .filter(|v| v.wav >= DEMO_WAV)
            .collect();
        assert_eq!(demo.len(), heard);
        let d = play.drill.as_ref().unwrap();
        assert!(
            play.judge
                .notes()
                .iter()
                .all(|n| !d.run.is_demo(n.at.measure))
        );
        // 自分の小節の頭の音符も、ふつうに叩ける
        let first = play.judge.notes()[0].clone();
        let events = play.judge.hit(Hit {
            ms: first.ms + 2.0,
            lane: first.lane,
            velocity: 100,
        });
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].judgement, Judgement::Perfect);
        let d = play.drill.as_ref().unwrap();
        // お手本の小節（2 小節目）は叩かなくても休みにしない
        let m1 = d.measure_start_ms(1);
        let m2 = d.measure_start_ms(2);
        let d = play.drill.as_mut().unwrap();
        assert!(!d.check_rest(m2 - 10.0));
        assert!(!d.check_rest(m1 + 3000.0));
    }

    #[test]
    fn metronome_and_bgm_follow_the_rules() {
        // BPM 120（1 小節 2000ms）。BGM（wav 01）は小節 1 の頭、HH（wav 02）は小節 2 の頭
        let text = "#BPM 120\n#00101: 01\n#00211: 02\n";
        let wavs = |p: &Play| -> Vec<(f64, u16)> {
            p.chart_sounds.auto.iter().map(|v| (v.ms, v.wav)).collect()
        };
        let plain = load_text("metronome-off", text, &Rules::default());
        assert_eq!(wavs(&plain), vec![(2000.0, 1)]);
        assert!(!plain.metronome());
        let mut on = load_text(
            "metronome-on",
            text,
            &Rules {
                metronome: true,
                auto_mix: AutoMix {
                    bgm: false,
                    shift_ms: 30.0,
                },
                ..Default::default()
            },
        );
        // BGM は鳴らさない。メトロノームは演奏中に切り替えられるよう、譜面の音にまとめず拍の線から鳴らす
        assert!(on.metronome());
        assert_eq!(wavs(&on), vec![]);
        on.set_metronome(false);
        assert!(!on.metronome());
        on.set_metronome(true);
        assert!(on.metronome());
    }

    #[test]
    fn practice_uses_its_own_metronome_and_keeps_the_switch_on_retry() {
        // 練習は練習の決めごとのメトロノームで鳴らす（設定のメトロノームは練習でない演奏のもの）
        let text = "#BPM 120\n#00101: 01\n#00211: 02\n";
        let path =
            std::env::temp_dir().join(format!("nxt-metronome-practice-{}.dtx", std::process::id()));
        std::fs::write(&path, text).unwrap();
        let rules = Rules {
            metronome: true,
            ..Default::default()
        };
        let options = PracticeOptions {
            section: engine::practice::Section::new(1, 1),
            style: Default::default(),
            rule: Default::default(),
            speed: SpeedPlan::Fixed(100),
            pitch: Default::default(),
            metronome: false,
            step_laps: 2,
        };
        let play = Play::load_practice(&path, 0.0, false, &rules, options);
        std::fs::remove_file(&path).unwrap();
        let mut play = play.unwrap();
        assert!(!play.metronome());
        // 一時停止の板で入れると、やり直しても入ったまま
        play.set_metronome(true);
        assert!(play.metronome());
        let p = play.practice.as_ref().unwrap();
        assert!(p.options.metronome && p.restart_options().metronome);
    }

    #[test]
    fn beat_lines_follow_bar_lengths() {
        // BPM 120（1 拍 500ms）。小節の長さは次に変えるまで続くので、小節 2 で 4/4 に戻す
        let chart = dtx::parse("#BPM 120\n#00102: 0.75\n#00202: 1\n#00211: 01\n");
        let lines = beat_lines(&TempoMap::build(&chart));
        let bars: Vec<f64> = lines.iter().filter(|l| l.bar).map(|l| l.ms).collect();
        let beats: Vec<f64> = lines.iter().filter(|l| !l.bar).map(|l| l.ms).collect();
        // 小節 0（4 拍）、小節 1（3 拍）、小節 2（4 拍）、曲の終わり
        assert_eq!(bars, vec![0.0, 2000.0, 3500.0, 5500.0]);
        assert_eq!(
            beats,
            vec![
                500.0, 1000.0, 1500.0, 2500.0, 3000.0, 4000.0, 4500.0, 5000.0
            ]
        );
    }
}
