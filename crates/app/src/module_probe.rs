//! 音源（Roland V71・V51・V31）の測る道具（開発者モードから動かす）。
//!
//! 音源の値（TOM1 HEAD の音量と音色、今のキット）を書き換えてから叩いた音が変わるまでの時間などを、
//! 音源の USB の音を録って測る。叩くのは MIDI のノートオンで、人は何もしなくてよい。
//! 書き換える前の値は必ず記録に残し（[`crate::module_link`]）、終わったら（途中でやめても、失敗しても）
//! 書き戻して読み直す。結果は issue に貼る Markdown（[`Finding`]）で返す（題の印と本文の長さの上限は開発者モードの側で扱う）。
//!
//! 録った音はこの中で調べ、数字だけを返す（音そのものは送らない）。

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use drum_module::listen::{self, FrameClock, Shape};
use drum_module::roland::{self, Address};
use drum_module::vseries::{self, Block, Layer, Unit};
use i18n::{t, tf, tr};

use crate::dev_report::{Finding, Tool};
use crate::module_link::{self, ModuleLink};
use crate::time::host_us;

/// 開発者メニューに並べる道具（[`crate::dev_report::TOOLS`]）
pub const TOOL: Tool = Tool {
    key: "module-probe",
    name: tr("Drum Module Sound Switch Test", "音源の音の切り替えテスト"),
    desc: tr(
        "For a Roland V71, V51 or V31. Briefly changes the module's TOM1 volume, TOM1 sound and kit, plays notes over MIDI, and records its USB audio to see how fast each change takes effect. Everything is put back at the end.",
        "Roland V71・V51・V31 用。音源の TOM1 の音量と音色、キットを一時的に変えて MIDI で鳴らし、音源の USB の音を録って、変えてから効くまでの時間を測ります。終わったら元に戻します。",
    ),
    confirm: Some(tr(
        "The drum module will play drum sounds for about a minute. Turn the volume down if you're on headphones, and don't touch the module until it's done.",
        "音源からドラムの音が 1 分ほど鳴ります。ヘッドホンの方は音量を下げ、終わるまで音源に触らないでください。",
    )),
    labels: &["drum-module"],
    run: run_module_probe,
};

/// 鳴らす強さ
const VELOCITY: u8 = 100;
/// パッドのチャンネルが GLOBAL のときに送るチャンネル（本体の既定の 10 ch）
const GLOBAL_CHANNEL_GUESS: u8 = vseries::GLOBAL_CHANNEL_DEFAULT;
/// 叩く間
const HIT_GAP_US: i64 = 700_000;

/// -INF にしてから（戻してから）叩くまでの間（ms）
const MUTE_DELAYS: [i64; 8] = [0, 1, 2, 5, 10, 20, 50, 100];
/// 音色を変えてから叩くまでの間（ms）
const VOICE_DELAYS: [i64; 8] = [0, 2, 5, 10, 20, 50, 100, 200];
/// キットを切り替えてから叩くまでの間（ms）
const KIT_DELAYS: [i64; 6] = [0, 20, 50, 100, 200, 500];
/// 鳴っている途中で -INF にするのは、叩いてから何 ms 後か
const TAIL_AT_US: i64 = 120_000;

/// 途中でやめた
#[derive(Debug)]
struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// 開発者モードから呼ぶ。`progress` には今していることを 1 行ずつ渡す。`cancel` が立ったらやめる（書き戻しはする）
pub fn run_module_probe(cancel: &AtomicBool, progress: &mut dyn FnMut(String)) -> Finding {
    let started = Instant::now();
    let mut r = Report::default();
    let result = probe(cancel, progress, &mut r);
    let secs = started.elapsed().as_secs_f64();
    let m = r.model.unwrap_or("音源");
    let (title, status) = match &result {
        Ok(()) => (
            format!("{m} の測定結果"),
            format!("最後まで測った（{secs:.0} 秒）"),
        ),
        Err(e) if e.is::<Cancelled>() => (
            format!("{m} の測定（途中でやめた）"),
            format!("{secs:.0} 秒でやめた。そこまでの結果"),
        ),
        Err(e) => (
            format!("{m} の測定（途中で止まった）"),
            format!("{secs:.0} 秒で止まった: {e:#}"),
        ),
    };
    let mut body = format!("## {m} の測定\n\n{status}\n\n");
    body.push_str(&r.out);
    Finding { title, body }
}

fn probe(cancel: &AtomicBool, progress: &mut dyn FnMut(String), r: &mut Report) -> Result<()> {
    progress(t("Looking for the drum module…", "音源を探しています…").to_owned());
    let mut notes = Vec::new();
    let mut link = match ModuleLink::find(&mut |s| notes.push(s)) {
        Ok(l) => l,
        Err(e) => {
            r.line(format!("- V71・V51・V31 が見つからない: {e:#}"));
            for n in &notes {
                r.line(format!("  - {n}"));
            }
            return Err(e);
        }
    };
    let id = link.identity;
    r.model = Some(link.model.name);
    r.h(link.model.name);
    r.line(format!(
        "- MIDI 出力 `{}`、Device ID {:02X}H、Identity の版 `{}`",
        link.port,
        id.device,
        roland::hex(&id.revision)
    ));
    for n in &notes {
        r.line(format!("- メモ: {n}"));
    }
    if link.pending() > 0 {
        progress(
            t(
                "Restoring values left from last time…",
                "前回の書き換えを戻しています…",
            )
            .to_owned(),
        );
        let res = link.restore()?;
        r.line(format!(
            "- 前回の書き戻しが残っていた: {} か所を戻し、{} か所が元の値と合った",
            res.written, res.verified
        ));
        if !res.failed.is_empty() {
            r.line(format!("- 戻せなかった: {}", res.failed.join("; ")));
            bail!("前回の書き換えを戻せない");
        }
    }
    let result = Measure::new(&mut link, cancel).run(progress, r);
    progress(t("Putting the drum module back…", "音源を元に戻しています…").to_owned());
    r.h("書き戻し");
    match link.restore() {
        Ok(res) if res.failed.is_empty() => r.line(format!(
            "- {} か所を書き戻し、読み直して {} か所とも元の値と合った（書き換えは {} 回）",
            res.written, res.verified, link.writes
        )),
        Ok(res) => {
            r.line(format!(
                "- {} か所を書き戻したが、合わない所がある: {}（記録に残したので、次に起動したときにもう一度戻す）",
                res.written,
                res.failed.join("; ")
            ));
        }
        Err(e) => r.line(format!(
            "- 書き戻せなかった: {e:#}（記録に残したので、次に起動したときにもう一度戻す）"
        )),
    }
    result
}

/// issue の本文
#[derive(Default)]
struct Report {
    out: String,
    /// 見つけた音源の機種
    model: Option<&'static str>,
}

impl Report {
    fn h(&mut self, title: &str) {
        let _ = write!(self.out, "\n### {title}\n\n");
    }

    fn line(&mut self, s: impl AsRef<str>) {
        self.out.push_str(s.as_ref());
        self.out.push('\n');
    }
}

/// パッドを MIDI で鳴らすためのノートとチャンネル（[KitMidi] から）
#[derive(Debug, Clone, Copy)]
struct Pad {
    note: Option<u8>,
    channel: u8,
    /// チャンネルが GLOBAL だったので 10 ch と見なした
    global: bool,
}

impl Pad {
    fn read(midi: &[u8], unit: Unit) -> Self {
        let note = unit
            .note_field()
            .read(midi)
            .filter(|n| (0..128).contains(n))
            .map(|n| n as u8);
        let ch = unit
            .channel_field()
            .read(midi)
            .unwrap_or(vseries::CHANNEL_GLOBAL);
        let (channel, global) = if (0..16).contains(&ch) {
            (ch as u8, false)
        } else {
            (GLOBAL_CHANNEL_GUESS, true)
        };
        Self {
            note,
            channel,
            global,
        }
    }

    fn label(self) -> String {
        let note = self
            .note
            .map_or_else(|| "OFF".to_owned(), |n| n.to_string());
        let ch = if self.global {
            "GLOBAL（10 ch で送る）".to_owned()
        } else {
            format!("{} ch", self.channel + 1)
        };
        format!("ノート {note}、{ch}")
    }
}

/// 叩いた 1 回
#[derive(Debug, Clone)]
struct Hit {
    group: Group,
    /// 値を書き換えてから叩くまでの狙いの間（ms）
    target_ms: Option<i64>,
    /// ノートオンを送ったホスト時刻
    sent_us: i64,
    /// 叩く前に値を書き換えた（または鳴っている途中で書き換えた）ホスト時刻
    change_us: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    /// 何も変えずに叩いた基準
    Base,
    /// -INF にしてから叩く
    Mute,
    /// -INF から戻してから叩く
    Unmute,
    /// 叩いて鳴っている途中で -INF にする
    Tail,
    /// 変えた音色で十分に待ってから叩いた基準
    VoiceRef,
    /// 音色を変えてから叩く
    Voice,
    /// 切り替えたキットで十分に待ってから叩いた基準
    KitRef,
    /// キットを切り替えてから叩く
    Kit,
    /// AUX4 HEAD を鳴らす
    Aux,
}

impl Group {
    fn label(self) -> &'static str {
        match self {
            Group::Base => "基準",
            Group::Mute => "-INF→叩く",
            Group::Unmute => "戻す→叩く",
            Group::Tail => "鳴っている途中で -INF",
            Group::VoiceRef => "変えた音色の基準",
            Group::Voice => "音色→叩く",
            Group::KitRef => "切り替えたキットの基準",
            Group::Kit => "キット→叩く",
            Group::Aux => "AUX4 HEAD",
        }
    }
}

/// 測るあいだの持ち物
struct Measure<'a> {
    link: &'a mut ModuleLink,
    cancel: &'a AtomicBool,
    hits: Vec<Hit>,
}

/// 読んだ値（書き換える所と、戻す値）
struct Values {
    kit: u16,
    current: Vec<u8>,
    tom: Pad,
    aux: Pad,
    tom_common: (Block, Vec<u8>),
    tom_layer_a: (Block, Vec<u8>),
    snare_layer_a: Vec<u8>,
}

impl<'a> Measure<'a> {
    fn new(link: &'a mut ModuleLink, cancel: &'a AtomicBool) -> Self {
        Self {
            link,
            cancel,
            hits: Vec::new(),
        }
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(anyhow!(Cancelled));
        }
        Ok(())
    }

    /// `target_us`（ホスト時刻）まで待つ。終わり際は細かく回して 1 ms より細かく合わせる
    fn wait_until(&self, target_us: i64) -> Result<()> {
        loop {
            self.check_cancel()?;
            let left = target_us - host_us();
            if left <= 0 {
                return Ok(());
            }
            if left > 3_000 {
                std::thread::sleep(Duration::from_micros((left - 2_000).min(20_000) as u64));
            } else {
                std::thread::yield_now();
            }
        }
    }

    fn wait_ms(&self, ms: i64) -> Result<()> {
        self.wait_until(host_us() + ms * 1000)
    }

    /// 書き換え、書いた時刻を返す
    fn write(&mut self, addr: Address, data: &[u8]) -> Result<i64> {
        let at = host_us();
        self.link.write(addr, data)?;
        Ok(at)
    }

    /// 鳴らし、送った時刻を返す
    fn hit(&mut self, pad: Pad) -> Result<i64> {
        let note = pad.note.ok_or_else(|| anyhow!("note is OFF"))?;
        let at = host_us();
        self.link.note_on(pad.channel, note, VELOCITY)?;
        Ok(at)
    }

    fn push(&mut self, group: Group, target_ms: Option<i64>, sent_us: i64, change_us: Option<i64>) {
        self.hits.push(Hit {
            group,
            target_ms,
            sent_us,
            change_us,
        });
    }

    fn run(mut self, progress: &mut dyn FnMut(String), r: &mut Report) -> Result<()> {
        progress(
            t(
                "Reading the drum module's settings…",
                "音源の設定を読んでいます…",
            )
            .to_owned(),
        );
        let v = self.read_values(r)?;
        self.check_cancel()?;

        progress(t("Checking writes…", "書き込みを確かめています…").to_owned());
        let volume_ok = self.check_writes(&v, r)?;

        let mut notes = Vec::new();
        let rec = match Recorder::open(&mut |s| notes.push(s)) {
            Ok(rec) => rec,
            Err(e) => {
                notes.push(format!("{e:#}"));
                None
            }
        };
        r.h("音の録音");
        let Some(rec) = rec else {
            r.line("- 音源の録音の入力が見つからないので、音を使う測定はしなかった");
            for n in &notes {
                r.line(format!("  - {n}"));
            }
            return Ok(());
        };
        r.line(format!(
            "- 入力 `{}`、{} Hz、{} ch、{}",
            rec.name, rec.rate, rec.channels, rec.format
        ));
        if v.tom.note.is_none() {
            r.line(
                "- TOM1 HEAD のノートが OFF なので、MIDI で鳴らせない。音を使う測定はしなかった",
            );
            return Ok(());
        }
        // 録音が落ち着くのを待ち、雑音を測る
        self.wait_ms(600)?;
        let noise_from = host_us();
        self.wait_ms(400)?;
        let noise_to = host_us();

        let result = self.sound_trials(&v, volume_ok, progress);
        progress(t("Analyzing the recording…", "録った音を調べています…").to_owned());
        let snap = rec.snapshot();
        drop(rec);
        match snap {
            Some(snap) => analyze(&snap, (noise_from, noise_to), &self.hits, r),
            None => r.line("- 録音が空だった（音源の USB の音が届いていない）"),
        }
        result
    }

    fn read_values(&mut self, r: &mut Report) -> Result<Values> {
        let current = self.link.read(vseries::current())?;
        let kit_index = vseries::KIT_NUM
            .read(&current)
            .filter(|k| (0..i32::from(vseries::KIT_COUNT)).contains(k))
            .ok_or_else(|| anyhow!("current kit is {}", roland::hex(&current)))?;
        let kit = (kit_index + 1) as u16;
        let common = self.link.read(vseries::kit_common(kit))?;
        let midi_block = vseries::kit_midi(kit);
        let midi = self.link.read(midi_block)?;
        let tom = Pad::read(&midi, Unit::Tom1Head);
        let aux = Pad::read(&midi, Unit::Aux4Head);
        let tom_common_block = vseries::unit_common(kit, Unit::Tom1Head);
        let tom_common = self.link.read(tom_common_block)?;
        let mut tom_layers = Vec::new();
        for layer in [Layer::A, Layer::B, Layer::C] {
            let b = vseries::unit_layer(kit, Unit::Tom1Head, layer);
            tom_layers.push((layer, b, self.link.read(b)?));
        }
        let snare_a_block = vseries::unit_layer(kit, Unit::SnareHead, Layer::A);
        let snare_a = self.link.read(snare_a_block)?;
        let aux_common_block = vseries::unit_common(kit, Unit::Aux4Head);
        let aux_common = self.link.read(aux_common_block)?;
        let aux_a_block = vseries::unit_layer(kit, Unit::Aux4Head, Layer::A);
        let aux_a = self.link.read(aux_a_block)?;

        r.h(&format!(
            "読んだ値（キット {kit}「{}」）",
            vseries::kit_name(&common).unwrap_or_default()
        ));
        r.line("| 所 | 番地 | 生の値 | 読み |");
        r.line("|---|---|---|---|");
        let mut row =
            |label: &str, block: Block, field: vseries::Field, data: &[u8], read: String| {
                r.line(format!(
                    "| {label} | `{}` | `{}` | {read} |",
                    block.at(field.offset).hex(),
                    field.raw(data).map(roland::hex).unwrap_or_default()
                ));
            };
        row(
            "今のキット",
            vseries::current(),
            vseries::KIT_NUM,
            &current,
            format!("キット {kit}"),
        );
        row(
            "TOM1 HEAD のノート",
            midi_block,
            Unit::Tom1Head.note_field(),
            &midi,
            tom.label(),
        );
        row(
            "TOM1 HEAD のチャンネル",
            midi_block,
            Unit::Tom1Head.channel_field(),
            &midi,
            String::new(),
        );
        row(
            "AUX4 HEAD のノート",
            midi_block,
            Unit::Aux4Head.note_field(),
            &midi,
            aux.label(),
        );
        row(
            "AUX4 HEAD のチャンネル",
            midi_block,
            Unit::Aux4Head.channel_field(),
            &midi,
            String::new(),
        );
        let vol = |data: &[u8], field: vseries::Field| {
            field.read(data).map_or_else(String::new, |x| {
                let range = if vseries::volume_in_range(x) {
                    ""
                } else {
                    "（範囲の外）"
                };
                format!("{x} = {}{range}", vseries::volume_label(x))
            })
        };
        let num = |data: &[u8], field: vseries::Field| {
            field.read(data).map_or_else(String::new, |x| x.to_string())
        };
        row(
            "TOM1 HEAD の音量",
            tom_common_block,
            vseries::UNIT_VOLUME,
            &tom_common,
            vol(&tom_common, vseries::UNIT_VOLUME),
        );
        for (layer, b, data) in &tom_layers {
            let l = layer.label();
            row(
                &format!("TOM1 HEAD レイヤー {l} の Inst Switch"),
                *b,
                vseries::LAYER_SWITCH,
                data,
                num(data, vseries::LAYER_SWITCH),
            );
            row(
                &format!("TOM1 HEAD レイヤー {l} の音色"),
                *b,
                vseries::LAYER_INSTRUMENT,
                data,
                num(data, vseries::LAYER_INSTRUMENT),
            );
            row(
                &format!("TOM1 HEAD レイヤー {l} の Inst Bank"),
                *b,
                vseries::LAYER_BANK,
                data,
                num(data, vseries::LAYER_BANK),
            );
            row(
                &format!("TOM1 HEAD レイヤー {l} の音量"),
                *b,
                vseries::LAYER_VOLUME,
                data,
                vol(data, vseries::LAYER_VOLUME),
            );
        }
        row(
            "SNARE HEAD レイヤー A の音色",
            snare_a_block,
            vseries::LAYER_INSTRUMENT,
            &snare_a,
            num(&snare_a, vseries::LAYER_INSTRUMENT),
        );
        row(
            "SNARE HEAD レイヤー A の Inst Bank",
            snare_a_block,
            vseries::LAYER_BANK,
            &snare_a,
            num(&snare_a, vseries::LAYER_BANK),
        );
        row(
            "AUX4 HEAD の音量",
            aux_common_block,
            vseries::UNIT_VOLUME,
            &aux_common,
            vol(&aux_common, vseries::UNIT_VOLUME),
        );
        row(
            "AUX4 HEAD レイヤー A の Inst Switch",
            aux_a_block,
            vseries::LAYER_SWITCH,
            &aux_a,
            num(&aux_a, vseries::LAYER_SWITCH),
        );
        row(
            "AUX4 HEAD レイヤー A の音色",
            aux_a_block,
            vseries::LAYER_INSTRUMENT,
            &aux_a,
            num(&aux_a, vseries::LAYER_INSTRUMENT),
        );
        let rt: Vec<f64> = self
            .link
            .round_trips
            .iter()
            .map(|&us| us as f64 / 1000.0)
            .collect();
        if let Some(med) = listen::median(&rt) {
            let max = rt.iter().copied().fold(0.0, f64::max);
            let min = rt.iter().copied().fold(f64::MAX, f64::min);
            r.line(String::new());
            r.line(format!(
                "- RQ1 を送ってから返事がそろうまで: 中央値 {med:.1} ms（最小 {min:.1}、最大 {max:.1}、{} 回）",
                rt.len()
            ));
        }
        Ok(Values {
            kit,
            current,
            tom,
            aux,
            tom_common: (tom_common_block, tom_common),
            tom_layer_a: (tom_layers[0].1, tom_layers[0].2.clone()),
            snare_layer_a: snare_a,
        })
    }

    /// TOM1 HEAD の音量に -INF を書いて読み直し、元に戻して読み直す（音が無くても確かめられる所）。
    /// 音量の読みが範囲に入っていれば true（音量を書き換える測定をしてよい）
    fn check_writes(&mut self, v: &Values, r: &mut Report) -> Result<bool> {
        let (block, data) = &v.tom_common;
        r.h("書き込みの確かめ");
        let Some(original) = vseries::UNIT_VOLUME
            .read(data)
            .filter(|&x| vseries::volume_in_range(x))
        else {
            r.line("- TOM1 HEAD の音量の読みが範囲の外なので、負の数の読み方が違う。音量は書き換えない");
            return Ok(false);
        };
        self.link.remember(
            *block,
            data,
            vseries::UNIT_VOLUME.offset,
            vseries::UNIT_VOLUME.len,
            "TOM1 HEAD の音量（測定）",
        )?;
        let addr = block.at(vseries::UNIT_VOLUME.offset);
        let off = vseries::UNIT_VOLUME.encode(vseries::VOLUME_OFF);
        self.write(addr, &off)?;
        let after = self.link.read(*block)?;
        let raw_after = vseries::UNIT_VOLUME
            .raw(&after)
            .map(roland::hex)
            .unwrap_or_default();
        let read_after = vseries::UNIT_VOLUME.read(&after);
        self.write(addr, &vseries::UNIT_VOLUME.encode(original))?;
        let back = self.link.read(*block)?;
        let read_back = vseries::UNIT_VOLUME.read(&back);
        r.line(format!(
            "- -INF（`{}`）を書いて読み直すと `{raw_after}`（{}）",
            roland::hex(&off),
            read_after.map_or_else(|| "?".to_owned(), vseries::volume_label)
        ));
        r.line(format!(
            "- 元の値（{}）に戻して読み直すと {}",
            vseries::volume_label(original),
            if read_back == Some(original) {
                "合った"
            } else {
                "合わない"
            }
        ));
        if read_after != Some(vseries::VOLUME_OFF) {
            r.line("- 書いた -INF がそのまま読めないので、音量を書き換える測定はしない");
            return Ok(false);
        }
        Ok(true)
    }

    fn sound_trials(
        &mut self,
        v: &Values,
        volume_ok: bool,
        progress: &mut dyn FnMut(String),
    ) -> Result<()> {
        progress(t("Recording reference hits…", "基準の音を録っています…").to_owned());
        for _ in 0..4 {
            let at = self.hit(v.tom)?;
            self.push(Group::Base, None, at, None);
            self.wait_until(at + HIT_GAP_US)?;
        }

        if volume_ok {
            let (block, data) = &v.tom_common;
            let addr = block.at(vseries::UNIT_VOLUME.offset);
            let off = vseries::UNIT_VOLUME.encode(vseries::VOLUME_OFF);
            let on = vseries::UNIT_VOLUME.raw(data).unwrap_or_default().to_vec();
            let n = MUTE_DELAYS.len();
            for (i, d) in MUTE_DELAYS.into_iter().enumerate() {
                let k = i + 1;
                progress(tf!(
                    "Testing how fast a mute works ({k}/{n})…",
                    "ミュートが効くまでを測っています（{k}/{n}）…"
                ));
                let change = self.write(addr, &off)?;
                self.wait_until(change + d * 1000)?;
                let at = self.hit(v.tom)?;
                self.push(Group::Mute, Some(d), at, Some(change));
                self.wait_ms(500)?;
                self.write(addr, &on)?;
                self.wait_ms(450)?;
            }
            for (i, d) in MUTE_DELAYS.into_iter().enumerate() {
                let k = i + 1;
                progress(tf!(
                    "Testing how fast an unmute works ({k}/{n})…",
                    "ミュートを解除して効くまでを測っています（{k}/{n}）…"
                ));
                self.write(addr, &off)?;
                self.wait_ms(450)?;
                let change = self.write(addr, &on)?;
                self.wait_until(change + d * 1000)?;
                let at = self.hit(v.tom)?;
                self.push(Group::Unmute, Some(d), at, Some(change));
                self.wait_ms(500)?;
            }
            progress(
                t(
                    "Muting while a hit is ringing…",
                    "鳴っている途中でミュートしています…",
                )
                .to_owned(),
            );
            for _ in 0..3 {
                let at = self.hit(v.tom)?;
                self.wait_until(at + TAIL_AT_US)?;
                let change = self.write(addr, &off)?;
                self.push(Group::Tail, None, at, Some(change));
                self.wait_ms(600)?;
                self.write(addr, &on)?;
                self.wait_ms(450)?;
            }
        }

        // 音色: TOM1 HEAD のレイヤー A を SNARE HEAD のレイヤー A の音色（Instrument と Inst Bank）にする
        let (layer_block, layer) = &v.tom_layer_a;
        let span =
            vseries::LAYER_INSTRUMENT.offset as usize..vseries::LAYER_BANK.offset as usize + 4;
        if let (Some(orig), Some(new)) = (layer.get(span.clone()), v.snare_layer_a.get(span))
            && orig != new
        {
            let (orig, new) = (orig.to_vec(), new.to_vec());
            self.link.remember(
                *layer_block,
                layer,
                vseries::LAYER_INSTRUMENT.offset,
                orig.len(),
                "TOM1 HEAD レイヤー A の音色（測定）",
            )?;
            let addr = layer_block.at(vseries::LAYER_INSTRUMENT.offset);
            progress(
                t(
                    "Recording the swapped sound…",
                    "変えた音色の基準を録っています…",
                )
                .to_owned(),
            );
            self.write(addr, &new)?;
            self.wait_ms(800)?;
            for _ in 0..3 {
                let at = self.hit(v.tom)?;
                self.push(Group::VoiceRef, None, at, None);
                self.wait_until(at + HIT_GAP_US)?;
            }
            self.write(addr, &orig)?;
            self.wait_ms(800)?;
            let n = VOICE_DELAYS.len();
            for (i, d) in VOICE_DELAYS.into_iter().enumerate() {
                let k = i + 1;
                progress(tf!(
                    "Testing how fast a sound change works ({k}/{n})…",
                    "音色を変えて効くまでを測っています（{k}/{n}）…"
                ));
                let change = self.write(addr, &new)?;
                self.wait_until(change + d * 1000)?;
                let at = self.hit(v.tom)?;
                self.push(Group::Voice, Some(d), at, Some(change));
                self.wait_ms(500)?;
                self.write(addr, &orig)?;
                self.wait_ms(700)?;
            }
        }

        // キット: となりのキットに切り替える
        let target = if v.kit < vseries::KIT_COUNT {
            v.kit + 1
        } else {
            v.kit - 1
        };
        self.link.remember(
            vseries::current(),
            &v.current,
            vseries::KIT_NUM.offset,
            vseries::KIT_NUM.len,
            "今のキット（測定）",
        )?;
        let addr = vseries::current().at(vseries::KIT_NUM.offset);
        let to = vseries::KIT_NUM.encode(i32::from(target) - 1);
        let back = v.current.clone();
        progress(
            t(
                "Recording the next kit…",
                "切り替えたキットの基準を録っています…",
            )
            .to_owned(),
        );
        self.write(addr, &to)?;
        self.wait_ms(1500)?;
        for _ in 0..2 {
            let at = self.hit(v.tom)?;
            self.push(Group::KitRef, None, at, None);
            self.wait_until(at + HIT_GAP_US)?;
        }
        self.write(addr, &back)?;
        self.wait_ms(1500)?;
        let n = KIT_DELAYS.len();
        for (i, d) in KIT_DELAYS.into_iter().enumerate() {
            let k = i + 1;
            progress(tf!(
                "Testing how fast a kit change works ({k}/{n})…",
                "キットを切り替えて効くまでを測っています（{k}/{n}）…"
            ));
            let change = self.write(addr, &to)?;
            self.wait_until(change + d * 1000)?;
            let at = self.hit(v.tom)?;
            self.push(Group::Kit, Some(d), at, Some(change));
            self.wait_ms(600)?;
            self.write(addr, &back)?;
            self.wait_ms(1500)?;
        }

        if v.aux.note.is_some() {
            progress(t("Playing AUX4…", "AUX4 を鳴らしています…").to_owned());
            for _ in 0..2 {
                let at = self.hit(v.aux)?;
                self.push(Group::Aux, None, at, None);
                self.wait_until(at + HIT_GAP_US)?;
            }
        }
        Ok(())
    }
}

/// 音源の USB の音の録音
struct Recorder {
    _stream: cpal::Stream,
    shared: Arc<Mutex<Captured>>,
    rate: u32,
    channels: u16,
    format: String,
    name: String,
}

#[derive(Default)]
struct Captured {
    /// 全チャンネルの平均
    samples: Vec<f32>,
    /// コールバックごとの（バッファの頭のフレーム, それを録ったホスト時刻の見積もり）
    points: Vec<(u64, i64)>,
    frames: u64,
}

/// 録り終えた音
struct Snap {
    samples: Vec<f32>,
    clock: FrameClock,
    rate: u32,
}

impl Snap {
    /// ホスト時刻 `from_us`〜`to_us` の分
    fn span(&self, from_us: f64, to_us: f64) -> &[f32] {
        let len = self.samples.len() as f64;
        let a = self.clock.frame_at(from_us).clamp(0.0, len) as usize;
        let b = self.clock.frame_at(to_us).clamp(0.0, len) as usize;
        &self.samples[a..b.max(a)]
    }
}

impl Recorder {
    /// 名前に機種の名前（V71・V51・V31）を含む入力を開く。無ければ `None`（`note` に入力の名前を並べる）
    fn open(note: &mut dyn FnMut(String)) -> Result<Option<Self>> {
        let host = cpal::default_host();
        let mut names = Vec::new();
        let mut found = None;
        for d in host.input_devices()? {
            let name = d
                .description()
                .map(|x| x.name().to_owned())
                .unwrap_or_default();
            if found.is_none() && module_link::port_model(&name).is_some() {
                found = Some((d, name.clone()));
            }
            names.push(name);
        }
        let Some((device, name)) = found else {
            note(format!("録音の入力: {}", names.join(", ")));
            return Ok(None);
        };
        let supported = device.default_input_config()?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let rate = config.sample_rate;
        let channels = config.channels;
        let shared = Arc::new(Mutex::new(Captured::default()));
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, config, shared.clone()),
            SampleFormat::I16 => build::<i16>(&device, config, shared.clone()),
            SampleFormat::I32 => build::<i32>(&device, config, shared.clone()),
            SampleFormat::U16 => build::<u16>(&device, config, shared.clone()),
            other => bail!("input format {other:?} is not supported"),
        }?;
        stream.play()?;
        Ok(Some(Self {
            _stream: stream,
            shared,
            rate,
            channels,
            format: format!("{format:?}"),
            name,
        }))
    }

    fn snapshot(&self) -> Option<Snap> {
        let c = self.shared.lock().ok()?;
        if c.samples.is_empty() {
            return None;
        }
        let clock = FrameClock::fit(&c.points, self.rate)?;
        Some(Snap {
            samples: c.samples.clone(),
            clock,
            rate: self.rate,
        })
    }
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Mutex<Captured>>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let stream = device.build_input_stream(
        config,
        move |data: &[T], info: &cpal::InputCallbackInfo| {
            let now = host_us();
            let ts = info.timestamp();
            let delay = ts.callback.duration_since(ts.capture).as_micros() as i64;
            // おかしな時刻（1 秒より前）は使わない
            let first_us = now - if delay < 1_000_000 { delay } else { 0 };
            if let Ok(mut c) = shared.lock() {
                let start = c.frames;
                c.points.push((start, first_us));
                for frame in data.chunks(channels) {
                    let sum: f32 = frame.iter().map(|&s| s.to_sample::<f32>()).sum();
                    c.samples.push(sum / frame.len() as f32);
                }
                c.frames += (data.len() / channels) as u64;
            }
        },
        |e| log::warn!("drum module capture: {e}"),
        None,
    )?;
    Ok(stream)
}

/// 叩いた 1 回の音
struct Heard {
    /// 一番大きい所（dBFS）
    level: f32,
    /// 鳴り始めの、基準の遅れからのずれ（ms）
    onset_shift_ms: Option<f64>,
    shape: Option<Shape>,
}

/// 叩いてから鳴るまでの遅れ `latency_us` のつもりで、叩いた 1 回の音を調べる
fn hear(snap: &Snap, sent_us: i64, latency_us: f64, threshold_db: f32) -> Heard {
    let at = sent_us as f64 + latency_us;
    let level = listen::peak_db(snap.span(at - 5_000.0, at + 150_000.0));
    let search_from = at - 20_000.0;
    let search = snap.span(search_from, at + 100_000.0);
    let onset_shift_ms = listen::onset(search, threshold_db).map(|i| {
        let frame = snap.clock.frame_at(search_from).max(0.0).floor() + i as f64;
        (snap.clock.host_at(frame) - at) / 1000.0
    });
    let shape = Shape::of(snap.span(at - 2_000.0, at + 85_000.0), snap.rate);
    Heard {
        level,
        onset_shift_ms,
        shape,
    }
}

fn fmt_ms(v: Option<f64>) -> String {
    // 丸めると 0 になる小さい負の数を「-0.0」と書かない
    v.map_or_else(
        || "-".to_owned(),
        |x| format!("{:+.1}", if x.abs() < 0.05 { 0.0 } else { x }),
    )
}

/// 録った音を調べて表にする
fn analyze(snap: &Snap, noise: (i64, i64), hits: &[Hit], r: &mut Report) {
    let quiet = snap.span(noise.0 as f64, noise.1 as f64);
    let noise_rms = listen::rms_db(quiet);
    let noise_peak = listen::peak_db(quiet);
    r.line(format!(
        "- 雑音: 実効値 {noise_rms:.1} dBFS、一番大きい所 {noise_peak:.1} dBFS"
    ));
    // 基準の遅れ: 叩いてから 300 ms のうち、鳴り始めた所
    let base: Vec<&Hit> = hits.iter().filter(|h| h.group == Group::Base).collect();
    let mut latencies = Vec::new();
    for h in &base {
        let from = h.sent_us as f64;
        let w = snap.span(from, from + 300_000.0);
        let peak = listen::peak_db(w);
        let threshold = (noise_peak + 12.0).max(peak - 30.0);
        if peak > noise_peak + 12.0
            && let Some(i) = listen::onset(w, threshold)
        {
            let frame = snap.clock.frame_at(from).max(0.0).floor() + i as f64;
            latencies.push(snap.clock.host_at(frame) - from);
        }
    }
    let Some(latency) = listen::median(&latencies) else {
        r.line(format!(
            "- 基準の {} 回の音が録音に入っていない（音源の USB の音に TOM1 が出ていない）。ここで調べるのをやめた",
            base.len()
        ));
        return;
    };
    let of = |g: Group| -> Vec<usize> { (0..hits.len()).filter(|&i| hits[i].group == g).collect() };
    let base_idx = of(Group::Base);
    let base_level = listen::median(
        &base_idx
            .iter()
            .map(|&i| f64::from(hear(snap, hits[i].sent_us, latency, 0.0).level))
            .collect::<Vec<_>>(),
    )
    .unwrap_or(f64::from(listen::SILENCE_DB)) as f32;
    // 鳴り始めは、基準の大きさから 30 dB 下（雑音に近ければ雑音の 12 dB 上）を超えた所。
    // ずれは基準の鳴り始めを 0 とする
    let threshold = (noise_peak + 12.0).max(base_level - 30.0);
    let mut heard: Vec<Heard> = hits
        .iter()
        .map(|h| hear(snap, h.sent_us, latency, threshold))
        .collect();
    let base_shift = listen::median(
        &base_idx
            .iter()
            .filter_map(|&i| heard[i].onset_shift_ms)
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    for h in &mut heard {
        h.onset_shift_ms = h.onset_shift_ms.map(|x| x - base_shift);
    }
    let shapes = |idx: &[usize]| -> Option<Shape> {
        let v: Vec<Shape> = idx.iter().filter_map(|&i| heard[i].shape.clone()).collect();
        Shape::mean(&v)
    };
    let base_shape = shapes(&base_idx);
    let lat_list: Vec<String> = latencies
        .iter()
        .map(|x| format!("{:.1}", x / 1000.0))
        .collect();
    r.line(format!(
        "- MIDI を送ってから録音に音が出るまで: 中央値 {:.1} ms（{} ms）。録音の遅れを含む",
        latency / 1000.0,
        lat_list.join(", ")
    ));
    r.line(format!(
        "- TOM1 HEAD の大きさ（基準）: {base_level:.1} dBFS"
    ));
    // 鳴ったかどうか
    let silent_below = (noise_peak + 6.0).max(base_level - 40.0);
    let judge = |level: f32| -> &'static str {
        if level < silent_below {
            "鳴らない"
        } else if level > base_level - 3.0 {
            "元の大きさ"
        } else {
            "小さく鳴る"
        }
    };
    let delay_ms = |h: &Hit| h.change_us.map(|c| (h.sent_us - c) as f64 / 1000.0);

    for (g, title) in [
        (
            Group::Mute,
            "1. 音量を -INF にしてから叩くまでの間と、鳴った大きさ",
        ),
        (
            Group::Unmute,
            "2. 音量を -INF から戻してから叩くまでの間と、鳴った大きさ",
        ),
    ] {
        let idx = of(g);
        if idx.is_empty() {
            continue;
        }
        r.h(title);
        r.line("| 間（狙い） | 間（実際） | 大きさ（基準との差） | 判定 | 鳴り始めのずれ |");
        r.line("|---|---|---|---|---|");
        for &i in &idx {
            let h = &hits[i];
            r.line(format!(
                "| {} ms | {} ms | {:+.1} dB | {} | {} ms |",
                h.target_ms.unwrap_or_default(),
                fmt_ms(delay_ms(h)),
                heard[i].level - base_level,
                judge(heard[i].level),
                fmt_ms(heard[i].onset_shift_ms)
            ));
        }
    }

    let tail = of(Group::Tail);
    if !tail.is_empty() {
        r.h("3. 鳴っている途中で -INF にしたとき");
        r.line(format!(
            "叩いてから {} ms 後に -INF を書いた。書いたあとの残りの音と「プツッ」（高い音の成分）を、何も変えない基準の同じ所と比べた。",
            TAIL_AT_US / 1000
        ));
        r.line(String::new());
        r.line("| 回 | 残りの音（基準との差） | プツッ（基準との差） |");
        r.line("|---|---|---|");
        for (n, &i) in tail.iter().enumerate() {
            let h = &hits[i];
            let Some(change) = h.change_us else {
                continue;
            };
            let rel = (change - h.sent_us) as f64;
            let rest = |sent: i64| {
                let at = sent as f64 + rel + latency;
                listen::rms_db(snap.span(at + 10_000.0, at + 200_000.0))
            };
            let click = |sent: i64| {
                let at = sent as f64 + rel + latency;
                listen::click_db(snap.span(at - 5_000.0, at + 30_000.0))
            };
            let base_rest = listen::median(
                &base_idx
                    .iter()
                    .map(|&b| f64::from(rest(hits[b].sent_us)))
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default() as f32;
            let base_click = listen::median(
                &base_idx
                    .iter()
                    .map(|&b| f64::from(click(hits[b].sent_us)))
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default() as f32;
            r.line(format!(
                "| {} | {:+.1} dB | {:+.1} dB |",
                n + 1,
                rest(h.sent_us) - base_rest,
                click(h.sent_us) - base_click
            ));
        }
    }

    for (g, refg, title, what) in [
        (
            Group::Voice,
            Group::VoiceRef,
            "4. 音色を変えてから叩くまでの間と、鳴った音色",
            "TOM1 HEAD のレイヤー A を SNARE HEAD のレイヤー A の音色にした",
        ),
        (
            Group::Kit,
            Group::KitRef,
            "5. キットを切り替えてから叩くまでの間と、鳴った音",
            "今のキットから、となりのキットに切り替えた（Current の KitNum を DT1 で書く）",
        ),
    ] {
        let idx = of(g);
        let ref_idx = of(refg);
        if idx.is_empty() {
            continue;
        }
        r.h(title);
        r.line(format!(
            "{what}。音の色（帯ごとの強さ）が、元の音と変えた音のどちらに近いかで見分けた。"
        ));
        r.line(String::new());
        let new_shape = shapes(&ref_idx);
        let (Some(bs), Some(ns)) = (&base_shape, &new_shape) else {
            r.line("- 音の色を求められなかった");
            continue;
        };
        let apart = bs.distance(ns);
        r.line(format!(
            "- 元の音と変えた音の違い: {apart:.1} dB{}",
            if apart < 2.0 {
                "（小さいので見分けにくい）"
            } else {
                ""
            }
        ));
        let ref_level = listen::median(
            &ref_idx
                .iter()
                .map(|&i| f64::from(heard[i].level))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        r.line(format!(
            "- 変えた音の大きさ: {:+.1} dB（基準との差）",
            ref_level as f32 - base_level
        ));
        r.line(String::new());
        r.line("| 間（狙い） | 間（実際） | 元の音との違い | 変えた音との違い | 鳴った音 | 大きさ（基準との差） | 鳴り始めのずれ |");
        r.line("|---|---|---|---|---|---|---|");
        for &i in &idx {
            let h = &hits[i];
            let (d_old, d_new, which) = match &heard[i].shape {
                Some(s) => {
                    let (a, b) = (s.distance(bs), s.distance(ns));
                    (
                        format!("{a:.1} dB"),
                        format!("{b:.1} dB"),
                        if heard[i].level < silent_below {
                            "鳴らない"
                        } else if b < a {
                            "変えた音"
                        } else {
                            "元の音"
                        },
                    )
                }
                None => ("-".into(), "-".into(), "-"),
            };
            r.line(format!(
                "| {} ms | {} ms | {d_old} | {d_new} | {which} | {:+.1} dB | {} ms |",
                h.target_ms.unwrap_or_default(),
                fmt_ms(delay_ms(h)),
                heard[i].level - base_level,
                fmt_ms(heard[i].onset_shift_ms)
            ));
        }
    }

    let aux = of(Group::Aux);
    if !aux.is_empty() {
        r.h("6. AUX4 HEAD を MIDI で鳴らしたとき");
        for &i in &aux {
            let level = heard[i].level;
            r.line(format!(
                "- {:.1} dBFS（雑音との差 {:+.1} dB）: {}",
                level,
                level - noise_peak,
                if level > noise_peak + 10.0 {
                    "鳴った"
                } else {
                    "鳴らない"
                }
            ));
        }
    }

    r.h("生の記録");
    r.line("<details><summary>叩いた順</summary>");
    r.line(String::new());
    r.line("| # | 種類 | 送った時刻 | 書き換え→叩く | 大きさ | 鳴り始めのずれ |");
    r.line("|---|---|---|---|---|---|");
    let t0 = hits.first().map_or(0, |h| h.sent_us);
    for (n, (h, s)) in hits.iter().zip(&heard).enumerate() {
        r.line(format!(
            "| {} | {} | {:.1} s | {} ms | {:.1} dBFS | {} ms |",
            n + 1,
            h.group.label(),
            (h.sent_us - t0) as f64 / 1e6,
            fmt_ms(delay_ms(h)),
            s.level,
            fmt_ms(s.onset_shift_ms)
        ));
    }
    r.line(String::new());
    r.line("</details>");
    r.line(format!(
        "\n録音の時計: {:.2} フレーム/秒（名目 {} Hz）",
        snap.clock.rate, snap.rate
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// 叩いた音のような音（`hz` の正弦波が消えていく）を、ホスト時刻 `at_us` から入れる
    fn put(samples: &mut [f32], at_us: i64, hz: f32, amp: f32) {
        let start = (at_us as f64 * f64::from(RATE) / 1e6) as usize;
        for i in 0..RATE as usize / 2 {
            if let Some(s) = samples.get_mut(start + i) {
                let t = i as f32 / RATE as f32;
                *s += amp * (-t * 12.0).exp() * (2.0 * std::f32::consts::PI * hz * t).sin();
            }
        }
    }

    fn snap(samples: Vec<f32>) -> Snap {
        Snap {
            samples,
            clock: FrameClock {
                rate: f64::from(RATE),
                offset_us: 0.0,
            },
            rate: RATE,
        }
    }

    fn hit(group: Group, sent_us: i64, change_us: Option<i64>, target_ms: Option<i64>) -> Hit {
        Hit {
            group,
            target_ms,
            sent_us,
            change_us,
        }
    }

    #[test]
    fn reports_mutes_and_sound_changes_from_a_recording() {
        // 遅れ 10 ms。雑音は小さく入れる
        let lat = 10_000;
        let mut s: Vec<f32> = (0..RATE as usize * 12)
            .map(|i| ((i * 7919) % 101) as f32 / 101.0 * 1e-4)
            .collect();
        let mut hits = Vec::new();
        for k in 0..4 {
            let at = 1_000_000 + k * 700_000;
            put(&mut s, at + lat, 150.0, 0.5);
            hits.push(hit(Group::Base, at, None, None));
        }
        // -INF が効いた（鳴らない）ものと、効かなかったもの
        hits.push(hit(Group::Mute, 4_000_000, Some(4_000_000), Some(0)));
        put(&mut s, 4_700_000 + lat, 150.0, 0.5);
        hits.push(hit(Group::Mute, 4_700_000, Some(4_699_000), Some(1)));
        // 変えた音色の基準と、変えてから叩いたもの
        for k in 0..3 {
            let at = 5_400_000 + k * 700_000;
            put(&mut s, at + lat, 1200.0, 0.5);
            hits.push(hit(Group::VoiceRef, at, None, None));
        }
        put(&mut s, 7_500_000 + lat, 150.0, 0.5);
        hits.push(hit(Group::Voice, 7_500_000, Some(7_500_000), Some(0)));
        put(&mut s, 8_200_000 + lat, 1200.0, 0.5);
        hits.push(hit(Group::Voice, 8_200_000, Some(8_150_000), Some(50)));
        let mut r = Report::default();
        analyze(&snap(s), (100_000, 500_000), &hits, &mut r);
        let out = r.out;
        assert!(out.contains("中央値 10.0 ms"), "{out}");
        assert!(out.contains("| 0 ms | +0.0 ms | "), "{out}");
        let mute_rows: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("| 0 ms") || l.starts_with("| 1 ms"))
            .collect();
        assert!(mute_rows[0].contains("鳴らない"), "{out}");
        assert!(mute_rows[1].contains("元の大きさ"), "{out}");
        let voice_rows: Vec<&str> = out
            .lines()
            .filter(|l| l.contains("元の音") || l.contains("変えた音"))
            .filter(|l| l.starts_with("| "))
            .collect();
        assert!(
            voice_rows
                .iter()
                .any(|l| l.starts_with("| 0 ms") && l.contains("| 元の音 |")),
            "{out}"
        );
        assert!(
            voice_rows
                .iter()
                .any(|l| l.starts_with("| 50 ms") && l.contains("| 変えた音 |")),
            "{out}"
        );
    }

    #[test]
    fn says_so_when_the_recording_has_no_drums() {
        let s = vec![0.0f32; RATE as usize * 4];
        let hits = vec![hit(Group::Base, 1_000_000, None, None)];
        let mut r = Report::default();
        analyze(&snap(s), (100_000, 500_000), &hits, &mut r);
        assert!(r.out.contains("録音に入っていない"), "{}", r.out);
    }

    #[test]
    fn pad_channels() {
        let mut midi = vec![0u8; vseries::kit_midi(1).size as usize];
        midi[0x2C..0x30].copy_from_slice(&[0, 0, 3, 0]);
        midi[Unit::Tom1Head.channel_field().offset as usize] = 16;
        let p = Pad::read(&midi, Unit::Tom1Head);
        assert_eq!((p.note, p.channel, p.global), (Some(48), 9, true));
        assert!(p.label().contains("GLOBAL"));
        midi[0x2C..0x30].copy_from_slice(&[0, 0, 8, 0]);
        midi[Unit::Tom1Head.channel_field().offset as usize] = 2;
        let p = Pad::read(&midi, Unit::Tom1Head);
        assert_eq!((p.note, p.channel, p.global), (None, 2, false));
        assert_eq!(p.label(), "ノート OFF、3 ch");
    }
}
