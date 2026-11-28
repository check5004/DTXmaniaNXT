//! キットを選んだときに音源（V71・V51・V31）で鳴らす試しのフレーズ。
//!
//! 8 ビートを 1 小節（4 拍目の裏でハイハットを開ける）、タム回しを 1 小節、最後にクラッシュとキック。
//! テンポは 120。ノート番号とチャンネルはそのキットの [KitMidi] から読む。
//!
//! ハイハットの開け閉めは、工場出荷の設定で Hi-Hat Pedal CC になっている CC 4 で伝える
//! （3 機種の MIDI Implementation の例 3）。終わったら Reset All Controllers を送り、受けたペダルの位置を
//! 最初の 0 に戻す。

use std::time::Duration;

use drum_module::vseries::{self, Field, Unit};

/// テンポ（4 分音符の数 / 分）
const BPM: u64 = 120;
/// ハイハットを開けたときの CC 4 の値
const PEDAL_OPEN: u8 = 0;
/// ハイハットを閉じたときの CC 4 の値
const PEDAL_CLOSED: u8 = 127;
/// Reset All Controllers
const RESET_ALL_CONTROLLERS: u8 = 121;

/// フレーズで鳴らす太鼓
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drum {
    Kick,
    Snare,
    HiHatClosed,
    HiHatOpen,
    HiHatPedal,
    Tom1,
    Tom2,
    Tom3,
    Tom4,
    Crash,
}

/// 楽譜の 1 打
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    /// 頭からのミリ秒
    pub at_ms: u64,
    pub drum: Drum,
    pub velocity: u8,
}

/// フレーズの楽譜（時刻の順）
pub fn score() -> Vec<Hit> {
    let beat = 60_000 / BPM;
    let eighth = beat / 2;
    let sixteenth = beat / 4;
    let bar = beat * 4;
    let mut s = Vec::new();
    let mut hit = |at_ms, drum, velocity| {
        s.push(Hit {
            at_ms,
            drum,
            velocity,
        })
    };
    // 1 小節目: 8 ビート。キックは 1・3 拍と 3 拍の裏、スネアは 2・4 拍。4 拍の裏でハイハットを開ける
    for i in 0..8 {
        let at = i * eighth;
        if i == 7 {
            hit(at, Drum::HiHatOpen, 100);
        } else {
            hit(at, Drum::HiHatClosed, if i % 2 == 0 { 100 } else { 70 });
        }
        match i {
            0 | 4 => hit(at, Drum::Kick, 110),
            5 => hit(at, Drum::Kick, 90),
            2 | 6 => hit(at, Drum::Snare, 115),
            _ => {}
        }
    }
    // 2 小節目: 16 分のタム回し（スネア → タム 1 → タム 2 → タム 3・4）。頭でハイハットを足で閉じる
    let roll = [
        [Drum::Snare; 4],
        [Drum::Tom1; 4],
        [Drum::Tom2; 4],
        [Drum::Tom3, Drum::Tom3, Drum::Tom4, Drum::Tom4],
    ];
    let accents = [105, 70, 85, 75];
    hit(bar, Drum::Kick, 100);
    hit(bar, Drum::HiHatPedal, 90);
    for (b, drums) in (0..).zip(roll) {
        for (j, drum) in (0..).zip(drums) {
            hit(bar + (b * 4 + j) * sixteenth, drum, accents[j as usize]);
        }
    }
    // 3 小節目の頭: クラッシュとキック
    hit(bar * 2, Drum::Crash, 120);
    hit(bar * 2, Drum::Kick, 115);
    // 同じ時刻のものは書いた順のまま
    s.sort_by_key(|h| h.at_ms);
    s
}

/// 送る MIDI のメッセージ 1 つ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    /// 頭からの時間
    pub at: Duration,
    pub bytes: [u8; 3],
}

impl Event {
    /// ノートオンなら（チャンネル 0〜15、ノート番号）
    pub fn note(&self) -> Option<(u8, u8)> {
        (self.bytes[0] & 0xF0 == 0x90).then_some((self.bytes[0] & 0x0F, self.bytes[1]))
    }
}

/// 太鼓のノート番号の位置と、チャンネルを読むパッド
fn source(drum: Drum) -> (Field, Unit) {
    match drum {
        Drum::Kick => (Unit::Kick.note_field(), Unit::Kick),
        Drum::Snare => (Unit::SnareHead.note_field(), Unit::SnareHead),
        Drum::HiHatOpen => (Unit::HiHatHead.note_field(), Unit::HiHatHead),
        Drum::HiHatClosed => (vseries::HH_CLOSE_NOTE, Unit::HiHatHead),
        Drum::HiHatPedal => (vseries::HH_PEDAL_NOTE, Unit::HiHatHead),
        Drum::Tom1 => (Unit::Tom1Head.note_field(), Unit::Tom1Head),
        Drum::Tom2 => (Unit::Tom2Head.note_field(), Unit::Tom2Head),
        Drum::Tom3 => (Unit::Tom3Head.note_field(), Unit::Tom3Head),
        Drum::Tom4 => (Unit::Tom4Head.note_field(), Unit::Tom4Head),
        Drum::Crash => (Unit::Crash1Head.note_field(), Unit::Crash1Head),
    }
}

/// パッドのチャンネル（0〜15。GLOBAL なら工場出荷の 10 ch）
fn channel(midi: &[u8], unit: Unit) -> u8 {
    match unit.channel_field().read(midi) {
        Some(ch @ 0..=15) => ch as u8,
        _ => vseries::GLOBAL_CHANNEL_DEFAULT,
    }
}

/// 太鼓を鳴らすノート番号とチャンネル（ノートが OFF なら None）
fn pad(midi: &[u8], drum: Drum) -> Option<(u8, u8)> {
    let (field, unit) = source(drum);
    let note = field.read(midi).filter(|n| (0..128).contains(n))?;
    Some((channel(midi, unit), note as u8))
}

/// キットの [KitMidi] の塊 `midi` から、フレーズで送るメッセージを作る（時刻の順）
pub fn events(midi: &[u8]) -> Vec<Event> {
    let hh = channel(midi, Unit::HiHatHead);
    let control = |at, cc, value| Event {
        at,
        bytes: [0xB0 | hh, cc, value],
    };
    let mut out = Vec::new();
    let mut pedal = None;
    for h in score() {
        let at = Duration::from_millis(h.at_ms);
        let want = match h.drum {
            Drum::HiHatOpen => Some(PEDAL_OPEN),
            Drum::HiHatClosed | Drum::HiHatPedal => Some(PEDAL_CLOSED),
            _ => None,
        };
        // ペダルの位置はノートより先に
        if let Some(v) = want
            && pedal != Some(v)
        {
            out.push(control(at, vseries::HH_PEDAL_CC, v));
            pedal = Some(v);
        }
        if let Some((ch, note)) = pad(midi, h.drum) {
            out.push(Event {
                at,
                bytes: [0x90 | ch, note, h.velocity.clamp(1, 127)],
            });
        }
    }
    let end = out.last().map_or(Duration::ZERO, |e| e.at);
    out.push(control(end, RESET_ALL_CONTROLLERS, 0));
    out
}

/// 途中でやめたときに送るもの（受けたペダルの位置を戻す）
pub fn stop(events: &[Event]) -> Option<[u8; 3]> {
    events
        .last()
        .filter(|e| e.bytes[1] == RESET_ALL_CONTROLLERS)
        .map(|e| e.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &[Hit], drum: Drum) -> Vec<u64> {
        s.iter()
            .filter(|h| h.drum == drum)
            .map(|h| h.at_ms)
            .collect()
    }

    #[test]
    fn the_score_is_an_eight_beat_bar_and_a_tom_fill() {
        let s = score();
        assert!(s.windows(2).all(|w| w[0].at_ms <= w[1].at_ms));
        // 1 小節目: 8 分のハイハット（最後はオープン）、スネアは 2・4 拍
        assert_eq!(
            at(&s, Drum::HiHatClosed),
            vec![0, 250, 500, 750, 1000, 1250, 1500]
        );
        assert_eq!(at(&s, Drum::HiHatOpen), vec![1750]);
        assert_eq!(at(&s, Drum::Kick), vec![0, 1000, 1250, 2000, 4000]);
        // 2 小節目: スネア 4 つのあとにタム、ペダルで閉じる
        assert_eq!(at(&s, Drum::Snare), vec![500, 1500, 2000, 2125, 2250, 2375]);
        assert_eq!(at(&s, Drum::Tom1), vec![2500, 2625, 2750, 2875]);
        assert_eq!(at(&s, Drum::Tom2), vec![3000, 3125, 3250, 3375]);
        assert_eq!(at(&s, Drum::Tom3), vec![3500, 3625]);
        assert_eq!(at(&s, Drum::Tom4), vec![3750, 3875]);
        assert_eq!(at(&s, Drum::HiHatPedal), vec![2000]);
        assert_eq!(at(&s, Drum::Crash), vec![4000]);
    }

    /// 工場出荷に近いキット（チャンネルは全部 GLOBAL）
    fn kit_midi() -> Vec<u8> {
        let mut midi = vec![0u8; vseries::kit_midi(1).size as usize];
        let mut set = |field: Field, note: i32| {
            let at = field.offset as usize;
            midi[at..at + 4].copy_from_slice(&field.encode(note));
        };
        set(Unit::Kick.note_field(), 36);
        set(Unit::SnareHead.note_field(), 38);
        set(Unit::HiHatHead.note_field(), 46);
        set(vseries::HH_CLOSE_NOTE, 42);
        set(vseries::HH_PEDAL_NOTE, 44);
        set(Unit::Tom1Head.note_field(), 48);
        set(Unit::Tom2Head.note_field(), 45);
        set(Unit::Tom3Head.note_field(), 43);
        set(Unit::Tom4Head.note_field(), vseries::NOTE_OFF);
        set(Unit::Crash1Head.note_field(), 49);
        for u in Unit::ALL {
            midi[u.channel_field().offset as usize] = vseries::CHANNEL_GLOBAL as u8;
        }
        midi
    }

    #[test]
    fn events_use_the_kit_notes_and_the_pedal_cc() {
        let mut midi = kit_midi();
        // タム 1 だけ 3 ch
        midi[Unit::Tom1Head.channel_field().offset as usize] = 2;
        let ev = events(&midi);
        assert!(ev.windows(2).all(|w| w[0].at <= w[1].at));
        // 最初にペダルを閉じ、閉じたハイハットとキック
        assert_eq!(ev[0].bytes, [0xB9, 4, 127]);
        assert_eq!(ev[1].bytes, [0x99, 42, 100]);
        assert_eq!(ev[2].bytes, [0x99, 36, 110]);
        // オープンの前にペダルを開け、次の小節の頭で閉じてからペダルの音
        let open = ev.iter().position(|e| e.bytes == [0x99, 46, 100]).unwrap();
        assert_eq!(ev[open - 1].bytes, [0xB9, 4, 0]);
        assert_eq!(ev[open].at, Duration::from_millis(1750));
        let pedal = ev.iter().position(|e| e.note() == Some((9, 44))).unwrap();
        assert_eq!(ev[pedal - 1].bytes, [0xB9, 4, 127]);
        // ペダルの CC はこの 3 回だけ
        assert_eq!(ev.iter().filter(|e| e.bytes[..2] == [0xB9, 4]).count(), 3);
        // タム 1 は 3 ch、OFF のタム 4 は送らない
        assert_eq!(ev.iter().filter(|e| e.note() == Some((2, 48))).count(), 4);
        assert!(!ev.iter().any(|e| e.note().is_some_and(|(_, n)| n == 41)));
        assert_eq!(ev.iter().filter(|e| e.note().is_some()).count(), 31);
        // 最後に Reset All Controllers、途中でやめたときもそれを送る
        let last = ev.last().unwrap();
        assert_eq!(last.bytes, [0xB9, 121, 0]);
        assert_eq!(last.at, Duration::from_millis(4000));
        assert_eq!(stop(&ev), Some([0xB9, 121, 0]));
    }

    #[test]
    fn the_pedal_follows_the_hi_hat_channel() {
        let mut midi = kit_midi();
        midi[Unit::HiHatHead.channel_field().offset as usize] = 0;
        let ev = events(&midi);
        assert_eq!(ev[0].bytes, [0xB0, 4, 127]);
        assert_eq!(ev[1].bytes, [0x90, 42, 100]);
        assert_eq!(ev.last().unwrap().bytes, [0xB0, 121, 0]);
    }
}
