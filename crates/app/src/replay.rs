//! リプレイ。普通のプレイの記録（音符ごとの判定とずれ、空打ち）から叩いた時刻を組み立て直し、
//! 遊んだときと同じ決まり（判定の幅・叩き分け・自動で叩くレーンなど。記録の設定の JSON から）で判定し直して再生する。
//!
//! 記録は前から残しているので、リプレイのために新しく残すものは無い（これまでのプレイも見られる）。
//! 叩いた時刻は判定の時刻（入力の補正とこの曲の補正を引いたもの）で、音符の時刻 + ずれ。
//! 1 回の打撃で同じ時刻の音符をいくつも取ったとき（叩き分けを共通にしたレーン）は、記録には同じ時刻の行が
//! 並ぶので、組み立て直すときに 1 回にまとめる（[`rehearse`]）。

use std::collections::HashMap;

use dtx::{DrumLane, DrumNote, NotePos};
use engine::input::Articulation;
use engine::judge::{
    BdGroup, CyGroup, FtGroup, HhGroup, Hit, HitWindows, Judge, Judgement, LaneGroups, Tally,
};
use records::{NoteHit, StrayHit};

use crate::game::{GaugeRule, Rules};

/// 叩いた 1 回
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Take {
    /// 判定の時刻（ミリ秒）
    pub ms: f64,
    pub lane: DrumLane,
    pub velocity: u8,
    pub articulation: Option<Articulation>,
    pub hh_pedal: Option<u8>,
    /// 記録で取った音符（譜面の音符の添字）。空打ちは None
    pub target: Option<usize>,
}

/// 記録の設定（[`crate::game::Play::session`] が書く JSON）のうち、判定と再生に使うもの
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecordedRules {
    pub windows: Option<HitWindows>,
    pub pedal_windows: Option<HitWindows>,
    pub tight: bool,
    pub groups: Option<LaneGroups>,
    pub gauge: Option<GaugeRule>,
    pub auto: Vec<DrumLane>,
    /// この曲の補正（ミリ秒）
    pub song_offset_ms: f64,
}

impl RecordedRules {
    /// 記録の設定の JSON から読む。読めない所は飛ばす（今の設定のまま）
    pub fn parse(json: &str) -> Self {
        let Some(Json::Obj(fields)) = Json::parse(json) else {
            return Self::default();
        };
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        let windows = |key: &str| match get(key) {
            Some(Json::Arr(a)) if a.len() == 4 => {
                let n: Vec<f64> = a.iter().filter_map(Json::num).collect();
                (n.len() == 4).then(|| HitWindows {
                    perfect: n[0],
                    great: n[1],
                    good: n[2],
                    poor: n[3],
                })
            }
            _ => None,
        };
        let gauge = match get("gauge") {
            Some(Json::Obj(g)) => {
                let field = |key: &str| g.iter().find(|(k, _)| k == key).map(|(_, v)| v);
                let d = GaugeRule::default();
                Some(GaugeRule {
                    miss_factor: field("miss_factor")
                        .and_then(Json::num)
                        .unwrap_or(d.miss_factor),
                    risky: field("risky")
                        .and_then(Json::num)
                        .map_or(d.risky, |n| n.max(0.0) as u32),
                    hazard: matches!(field("hazard"), Some(Json::Bool(true))),
                })
            }
            _ => None,
        };
        let auto = match get("auto") {
            Some(Json::Arr(a)) => a
                .iter()
                .filter_map(|v| match v {
                    Json::Str(s) => DrumLane::from_name(s),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        Self {
            windows: windows("windows_ms"),
            pedal_windows: windows("pedal_windows_ms"),
            tight: matches!(get("tight"), Some(Json::Bool(true))),
            groups: match get("groups") {
                Some(Json::Str(s)) => parse_groups(s),
                _ => None,
            },
            gauge,
            auto,
            song_offset_ms: get("song_offset_ms").and_then(Json::num).unwrap_or(0.0),
        }
    }

    /// 今の設定の決まり `rules` を、遊んだときの決まりにする。記録に無いものは遊んだときの既定
    /// （書いていないのは既定と同じだったから）。メトロノームは鳴らさない
    pub fn apply(&self, rules: &mut Rules) {
        if let Some(w) = self.windows {
            rules.windows = w;
            rules.pedal_windows = self.pedal_windows.unwrap_or(w);
        }
        rules.tight = self.tight;
        rules.groups = self.groups.unwrap_or_default();
        rules.gauge = self.gauge.unwrap_or_default();
        rules.auto = self.auto.clone();
        rules.metronome = false;
    }
}

/// 叩き分けの記録（`LaneGroups { hh: Separate, ft: Common, ... }`、Debug の形）を読む
fn parse_groups(s: &str) -> Option<LaneGroups> {
    let inner = s.trim().strip_prefix("LaneGroups")?.trim();
    let inner = inner.strip_prefix('{')?.strip_suffix('}')?;
    let mut g = LaneGroups::default();
    for part in inner.split(',') {
        let Some((k, v)) = part.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "hh" => {
                g.hh = match v {
                    "Separate" => HhGroup::Separate,
                    "HiHatOnly" => HhGroup::HiHatOnly,
                    "LeftCymbalOnly" => HhGroup::LeftCymbalOnly,
                    "Common" => HhGroup::Common,
                    _ => return None,
                }
            }
            "ft" => {
                g.ft = match v {
                    "Separate" => FtGroup::Separate,
                    "Common" => FtGroup::Common,
                    _ => return None,
                }
            }
            "cy" => {
                g.cy = match v {
                    "Separate" => CyGroup::Separate,
                    "Common" => CyGroup::Common,
                    _ => return None,
                }
            }
            "bd" => {
                g.bd = match v {
                    "Separate" => BdGroup::Separate,
                    "LeftPedalsCommon" => BdGroup::LeftPedalsCommon,
                    "BassDrumsCommon" => BdGroup::BassDrumsCommon,
                    "Common" => BdGroup::Common,
                    _ => return None,
                }
            }
            "cymbal_free" => g.cymbal_free = v == "true",
            _ => {}
        }
    }
    Some(g)
}

/// 記録から叩いた時刻を組み立てる（時刻の順）。譜面に見つからない音符（譜面を直した）の行は飛ばし、その数も返す
pub fn takes(notes: &[DrumNote], hits: &[NoteHit], strays: &[StrayHit]) -> (Vec<Take>, usize) {
    let index: HashMap<(NotePos, DrumLane), usize> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| ((n.at, n.lane), i))
        .collect();
    let mut missing = 0;
    let mut out: Vec<Take> = Vec::with_capacity(hits.len() + strays.len());
    for h in hits {
        let Some(offset) = h.offset_ms else {
            continue;
        };
        let Some(&i) = index.get(&(h.at, h.lane)) else {
            missing += 1;
            continue;
        };
        out.push(Take {
            ms: notes[i].ms + offset,
            lane: h.lane,
            velocity: h.velocity.unwrap_or(100),
            articulation: h.articulation,
            hh_pedal: h.hh_pedal,
            target: Some(i),
        });
    }
    out.extend(strays.iter().map(|s| Take {
        ms: s.at_ms,
        lane: s.lane,
        velocity: s.velocity,
        articulation: s.articulation,
        hh_pedal: s.hh_pedal,
        target: None,
    }));
    out.sort_by(|a, b| a.ms.total_cmp(&b.ms));
    (out, missing)
}

/// 組み立てた打撃を判定にかけてみた結果
#[derive(Debug, Clone)]
pub struct Rehearsal {
    /// 同じ打撃の重なり（1 回で取った音符ぶんの行）をまとめた打撃
    pub takes: Vec<Take>,
    /// 打撃ごとに、取った最初の音符（空打ちは None）
    pub matched: Vec<Option<usize>>,
    pub tally: Tally,
    /// 音符ごとの判定（見逃しも）とずれ（ミリ秒）。判定しなかった音符は None
    pub judged: Vec<Option<(Judgement, Option<f64>)>>,
}

/// 打撃を時刻の順に判定にかける（`judge` は遊んだときと同じ決まりにした、まだ何も判定していないもの）。
/// 同じ時刻の打撃が、前の打撃がもう取った音符を指していたら、同じ 1 回の打撃とみなして飛ばす
pub fn rehearse(mut judge: Judge, takes: Vec<Take>) -> Rehearsal {
    let mut judged = vec![None; judge.notes().len()];
    let mut kept = Vec::with_capacity(takes.len());
    let mut matched = Vec::with_capacity(takes.len());
    let mut last: Option<(f64, Vec<usize>)> = None;
    for t in takes {
        if let (Some(target), Some((ms, closed))) = (t.target, &last)
            && (t.ms - ms).abs() < 1e-6
            && closed.contains(&target)
        {
            continue;
        }
        for e in judge.advance(t.ms) {
            judged[e.note] = Some((e.judgement, e.offset_ms));
        }
        let events = judge.hit(Hit {
            ms: t.ms,
            lane: t.lane,
            velocity: t.velocity,
        });
        for e in &events {
            judged[e.note] = Some((e.judgement, e.offset_ms));
        }
        matched.push(events.first().map(|e| e.note));
        last = Some((t.ms, events.iter().map(|e| e.note).collect()));
        kept.push(t);
    }
    for e in judge.advance(f64::INFINITY) {
        judged[e.note] = Some((e.judgement, e.offset_ms));
    }
    Rehearsal {
        takes: kept,
        matched,
        tally: judge.tally().clone(),
        judged,
    }
}

/// 記録の判定と、判定し直した結果が違う音符の数（ログに出して確かめる）
pub fn differences(
    notes: &[DrumNote],
    hits: &[NoteHit],
    judged: &[Option<(Judgement, Option<f64>)>],
) -> usize {
    let index: HashMap<(NotePos, DrumLane), usize> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| ((n.at, n.lane), i))
        .collect();
    hits.iter()
        .filter(|h| {
            index
                .get(&(h.at, h.lane))
                .is_some_and(|&i| judged[i].map(|j| j.0) != Some(h.judgement))
        })
        .count()
}

/// 再生しているリプレイ（[`crate::game::Play`] が持つ）
#[derive(Debug, Clone)]
pub struct Feed {
    /// 打撃（時刻の順）
    pub takes: Vec<Take>,
    /// 打撃ごとに、取った最初の音符（空打ちは None）
    pub matched: Vec<Option<usize>>,
    /// 打撃ごとの、鳴らす音とその音のレーン（鳴らす音が無ければ None）
    pub sounds: Vec<Option<(u16, DrumLane)>>,
    /// 次に判定にかける打撃と、次に出力へ渡す音
    pub next_hit: usize,
    pub next_sound: usize,
    /// ここ（判定の時刻）を過ぎたら再生を終える
    pub end_ms: f64,
    /// まだ何も判定していない判定（巻き戻すときに使う）
    pub fresh: Judge,
    /// 音符ごとの判定とずれ（最後まで判定にかけたもの。止めたときの横の譜面に使う）
    pub judged: Vec<Option<(Judgement, Option<f64>)>>,
}

/// 平たい JSON の値（記録の設定を読むだけの小さな読み手）
#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn parse(s: &str) -> Option<Self> {
        let mut p = Parser {
            s: s.as_bytes(),
            at: 0,
        };
        let v = p.value()?;
        p.space();
        (p.at == p.s.len()).then_some(v)
    }

    fn num(&self) -> Option<f64> {
        match self {
            Self::Num(n) => Some(*n),
            _ => None,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.s.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        self.space();
        if self.s.get(self.at) == Some(&b) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn word(&mut self, w: &str) -> bool {
        if self.s[self.at..].starts_with(w.as_bytes()) {
            self.at += w.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.space();
        match *self.s.get(self.at)? {
            b'{' => {
                self.at += 1;
                let mut fields = Vec::new();
                if self.eat(b'}') {
                    return Some(Json::Obj(fields));
                }
                loop {
                    self.space();
                    let Json::Str(k) = self.string()? else {
                        return None;
                    };
                    if !self.eat(b':') {
                        return None;
                    }
                    fields.push((k, self.value()?));
                    if self.eat(b'}') {
                        return Some(Json::Obj(fields));
                    }
                    if !self.eat(b',') {
                        return None;
                    }
                }
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(self.value()?);
                    if self.eat(b']') {
                        return Some(Json::Arr(items));
                    }
                    if !self.eat(b',') {
                        return None;
                    }
                }
            }
            b'"' => self.string(),
            b't' if self.word("true") => Some(Json::Bool(true)),
            b'f' if self.word("false") => Some(Json::Bool(false)),
            b'n' if self.word("null") => Some(Json::Null),
            _ => {
                let start = self.at;
                while self
                    .s
                    .get(self.at)
                    .is_some_and(|b| b.is_ascii_digit() || b"+-.eE".contains(b))
                {
                    self.at += 1;
                }
                std::str::from_utf8(&self.s[start..self.at])
                    .ok()?
                    .parse()
                    .ok()
                    .map(Json::Num)
            }
        }
    }

    fn string(&mut self) -> Option<Json> {
        if self.s.get(self.at) != Some(&b'"') {
            return None;
        }
        self.at += 1;
        let mut out = Vec::new();
        loop {
            let b = *self.s.get(self.at)?;
            self.at += 1;
            match b {
                b'"' => return String::from_utf8(out).ok().map(Json::Str),
                b'\\' => {
                    let e = *self.s.get(self.at)?;
                    self.at += 1;
                    out.push(match e {
                        b'n' => b'\n',
                        b't' => b'\t',
                        other => other,
                    });
                }
                other => out.push(other),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::judge::JudgeEvent;

    fn note(ms: f64, lane: DrumLane, measure: u32, num: u32) -> DrumNote {
        DrumNote {
            ms,
            lane,
            wav: 1,
            at: NotePos::new(measure, num, 16),
            beat: 0.0,
        }
    }

    /// 1 小節 2000 ms、16 分ごとに並べた譜面
    fn chart() -> Vec<DrumNote> {
        let mut notes = Vec::new();
        for m in 0..4u32 {
            for i in 0..16u32 {
                let ms = f64::from(m) * 2000.0 + f64::from(i) * 125.0;
                notes.push(note(ms, DrumLane::HH, m, i));
                if i % 4 == 0 {
                    notes.push(note(ms, DrumLane::BD, m, i));
                }
                if i % 8 == 4 {
                    notes.push(note(ms, DrumLane::SD, m, i));
                }
                if m == 3 && i == 15 {
                    notes.push(note(ms, DrumLane::LC, m, i));
                }
            }
        }
        notes.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        notes
    }

    /// 遊んだときと同じく、フレームごとに打撃を判定し、見逃しを進めて、記録の行を作る
    fn live(
        mut judge: Judge,
        inputs: &[(f64, DrumLane, u8)],
    ) -> (Vec<NoteHit>, Vec<StrayHit>, Tally) {
        let mut hits: Vec<Option<NoteHit>> = vec![None; judge.notes().len()];
        let mut strays = Vec::new();
        let mut record = |judge: &Judge, events: &[JudgeEvent]| {
            for e in events {
                let n = &judge.notes()[e.note];
                hits[e.note] = Some(NoteHit {
                    at: n.at,
                    lane: n.lane,
                    judgement: e.judgement,
                    offset_ms: e.offset_ms,
                    velocity: e.velocity,
                    articulation: None,
                    hh_pedal: None,
                    beat_pos: None,
                });
            }
        };
        let mut next = 0;
        let mut now = -100.0;
        while now < 9000.0 {
            now += 16.7;
            while let Some(&(ms, lane, velocity)) = inputs.get(next) {
                if ms > now {
                    break;
                }
                next += 1;
                let events = judge.hit(Hit { ms, lane, velocity });
                if events.is_empty() {
                    strays.push(StrayHit {
                        at_ms: ms,
                        lane,
                        velocity,
                        articulation: None,
                        hh_pedal: None,
                    });
                }
                record(&judge, &events);
            }
            let events = judge.advance(now);
            record(&judge, &events);
        }
        let tally = judge.tally().clone();
        (hits.into_iter().flatten().collect(), strays, tally)
    }

    /// 早め・遅め・抜け・空打ちを混ぜた演奏
    fn inputs(notes: &[DrumNote]) -> Vec<(f64, DrumLane, u8)> {
        let mut out = Vec::new();
        for (i, n) in notes.iter().enumerate() {
            match i % 9 {
                // 抜け（MISS）
                0 => {}
                // 遅め（GOOD〜POOR）
                1 => out.push((n.ms + 75.0, n.lane, 80)),
                2 => out.push((n.ms - 40.0, n.lane, 90)),
                _ => out.push((n.ms + (i % 5) as f64 * 6.0 - 12.0, n.lane, 100)),
            }
        }
        // 空打ち
        out.push((3333.0, DrumLane::FT, 70));
        out.push((5100.0, DrumLane::CY, 120));
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    #[test]
    fn the_rebuilt_play_judges_the_same() {
        let notes = chart();
        let judge = Judge::with_groups(notes.clone(), HitWindows::default(), LaneGroups::default());
        let (hits, strays, tally) = live(judge.clone(), &inputs(&notes));
        assert!(
            tally.miss > 0 && tally.stray >= 2 && tally.perfect > 0,
            "{tally:?}"
        );
        let (takes, missing) = takes(&notes, &hits, &strays);
        assert_eq!(missing, 0);
        assert!(takes.windows(2).all(|w| w[0].ms <= w[1].ms));
        let r = rehearse(judge, takes);
        assert_eq!(r.tally, tally);
        assert_eq!(differences(&notes, &hits, &r.judged), 0);
        assert_eq!(
            r.matched.iter().filter(|m| m.is_none()).count(),
            tally.stray as usize
        );
    }

    #[test]
    fn one_hit_that_took_two_notes_is_replayed_once() {
        // HH と LC を共通にすると、同じ時刻の HH と LC を 1 回で取る（記録には同じ時刻の行が 2 つ）
        let notes = chart();
        let groups = LaneGroups {
            hh: HhGroup::Common,
            ..LaneGroups::default()
        };
        let judge = Judge::with_groups(notes.clone(), HitWindows::default(), groups);
        let lc = notes.iter().find(|n| n.lane == DrumLane::LC).unwrap().ms;
        let ins: Vec<_> = notes
            .iter()
            .filter(|n| n.lane != DrumLane::LC)
            .map(|n| (n.ms + 5.0, n.lane, 100))
            .collect();
        let (hits, strays, tally) = live(judge.clone(), &ins);
        assert_eq!(tally.miss, 0);
        assert!(strays.is_empty());
        assert_eq!(
            hits.iter()
                .filter(|h| (h.at.measure, h.at.num) == (3, 15))
                .filter(|h| h.offset_ms == Some(5.0))
                .count(),
            2
        );
        let (all, _) = takes(&notes, &hits, &strays);
        let r = rehearse(judge, all.clone());
        assert_eq!(r.takes.len(), all.len() - 1);
        assert_eq!(r.tally, tally);
        assert!(r.takes.iter().any(|t| (t.ms - (lc + 5.0)).abs() < 1e-9));
    }

    #[test]
    fn rows_for_notes_no_longer_in_the_chart_are_skipped() {
        let notes = chart();
        let judge = Judge::with_groups(notes.clone(), HitWindows::default(), LaneGroups::default());
        let (mut hits, strays, _) = live(judge, &inputs(&notes));
        hits[3].at = NotePos::new(99, 0, 1);
        let (_, missing) = takes(&notes, &hits, &strays);
        assert_eq!(missing, 1);
    }

    #[test]
    fn settings_written_by_a_play_are_read_back() {
        let s = "{\"input_offset_ms\":-31,\"windows_ms\":[30,60,80,110],\"drum_sounds\":true,\
                 \"song_offset_ms\":12.5,\"pedal_windows_ms\":[40,70,90,120],\"tight\":true,\
                 \"groups\":\"LaneGroups { hh: HiHatOnly, ft: Common, cy: Separate, bd: BassDrumsCommon, cymbal_free: true }\",\
                 \"gauge\":{\"miss_factor\":1,\"risky\":3,\"hazard\":true},\"auto\":[\"BD\",\"LP\"]}";
        let r = RecordedRules::parse(s);
        assert_eq!(r.windows.map(|w| w.poor), Some(110.0));
        assert_eq!(r.pedal_windows.map(|w| w.perfect), Some(40.0));
        assert!(r.tight);
        assert_eq!(
            r.groups,
            Some(LaneGroups {
                hh: HhGroup::HiHatOnly,
                ft: FtGroup::Common,
                cy: CyGroup::Separate,
                bd: BdGroup::BassDrumsCommon,
                cymbal_free: true,
            })
        );
        assert_eq!(
            r.gauge,
            Some(GaugeRule {
                miss_factor: 1.0,
                risky: 3,
                hazard: true
            })
        );
        assert_eq!(r.auto, vec![DrumLane::BD, DrumLane::LP]);
        assert_eq!(r.song_offset_ms, 12.5);

        let mut rules = Rules {
            auto: vec![DrumLane::HH],
            tight: true,
            metronome: true,
            ..Rules::default()
        };
        RecordedRules::parse("{\"input_offset_ms\":0,\"windows_ms\":[34,67,84,117]}")
            .apply(&mut rules);
        assert!(rules.auto.is_empty() && !rules.tight && !rules.metronome);
        assert_eq!(rules.pedal_windows, rules.windows);
        assert_eq!(rules.groups, LaneGroups::default());
    }

    #[test]
    fn broken_settings_change_nothing() {
        assert_eq!(RecordedRules::parse("{"), RecordedRules::default());
        assert_eq!(RecordedRules::parse(""), RecordedRules::default());
        assert_eq!(
            RecordedRules::parse("{\"groups\":\"LaneGroups { hh: Odd }\"}").groups,
            None
        );
    }
}
