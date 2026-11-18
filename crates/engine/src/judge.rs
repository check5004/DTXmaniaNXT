//! 判定。画面のフレームではなく、叩いた時刻（曲の頭からのミリ秒）で決める。

use dtx::{DrumLane, DrumNote};

/// 判定の幅（音符の時刻からの前後のずれ、ミリ秒）。既定値は上流と同じ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitWindows {
    pub perfect: f64,
    pub great: f64,
    pub good: f64,
    pub poor: f64,
}

impl Default for HitWindows {
    fn default() -> Self {
        Self {
            perfect: 34.0,
            great: 67.0,
            good: 84.0,
            poor: 117.0,
        }
    }
}

impl HitWindows {
    pub fn classify(&self, offset_ms: f64) -> Option<Judgement> {
        let d = offset_ms.abs();
        if d <= self.perfect {
            Some(Judgement::Perfect)
        } else if d <= self.great {
            Some(Judgement::Great)
        } else if d <= self.good {
            Some(Judgement::Good)
        } else if d <= self.poor {
            Some(Judgement::Poor)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Judgement {
    Perfect,
    Great,
    Good,
    Poor,
    Miss,
}

/// ハイハット・左シンバルのまとめ方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HhGroup {
    /// HH・HHO・LC をすべて叩き分ける
    #[default]
    Separate,
    /// HH と HHO は叩き分け、LC は HH と共通
    HiHatOnly,
    /// LC だけ叩き分け、HH と HHO は共通
    LeftCymbalOnly,
    /// HH・HHO・LC をすべて共通
    Common,
}

/// ロータムとフロアタムのまとめ方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FtGroup {
    #[default]
    Separate,
    Common,
}

/// 右シンバルとライドのまとめ方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CyGroup {
    #[default]
    Separate,
    Common,
}

/// バスドラム・左ペダル・左バスドラムのまとめ方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BdGroup {
    /// BD・LP・LBD をすべて叩き分ける
    #[default]
    Separate,
    /// LP と LBD は共通、BD は別
    LeftPedalsCommon,
    /// BD と LBD は共通、LP は別
    BassDrumsCommon,
    /// 3 つとも共通
    Common,
}

/// 叩き分けの設定。叩いたレーンが、どのレーンの音符を取れるかを決める
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LaneGroups {
    pub hh: HhGroup,
    pub ft: FtGroup,
    pub cy: CyGroup,
    pub bd: BdGroup,
}

impl LaneGroups {
    /// 譜面に無いレーンがあると叩き分けようがないので、上流と同じく共通に寄せる
    pub fn effective_for(mut self, notes: &[DrumNote]) -> Self {
        let has = |lane: DrumLane| notes.iter().any(|n| n.lane == lane);
        if !has(DrumLane::RD) && self.cy == CyGroup::Separate {
            self.cy = CyGroup::Common;
        }
        let (hho, lc) = (has(DrumLane::HHO), has(DrumLane::LC));
        self.hh = match (self.hh, hho, lc) {
            (HhGroup::Separate, false, false) => HhGroup::Common,
            (HhGroup::Separate, false, true) => HhGroup::LeftCymbalOnly,
            (HhGroup::Separate, true, false) => HhGroup::HiHatOnly,
            (HhGroup::HiHatOnly, false, _) => HhGroup::Common,
            (HhGroup::LeftCymbalOnly, _, false) => HhGroup::Common,
            (g, _, _) => g,
        };
        self
    }

    /// `hit` のレーンを叩いたとき、`note` のレーンの音符を取れるか
    pub fn accepts(&self, hit: DrumLane, note: DrumLane) -> bool {
        use DrumLane::*;
        if hit == note {
            return true;
        }
        let pair = |a: DrumLane, b: DrumLane| (hit == a && note == b) || (hit == b && note == a);
        let hh = match self.hh {
            HhGroup::Separate => false,
            HhGroup::HiHatOnly => pair(HH, LC),
            HhGroup::LeftCymbalOnly => pair(HH, HHO),
            HhGroup::Common => pair(HH, HHO) || pair(HH, LC) || pair(HHO, LC),
        };
        let ft = self.ft == FtGroup::Common && pair(LT, FT);
        let cy = self.cy == CyGroup::Common && pair(CY, RD);
        let bd = match self.bd {
            BdGroup::Separate => false,
            BdGroup::LeftPedalsCommon => pair(LP, LBD),
            BdGroup::BassDrumsCommon => pair(BD, LBD),
            BdGroup::Common => pair(BD, LP) || pair(BD, LBD) || pair(LP, LBD),
        };
        hh || ft || cy || bd
    }
}

/// 叩いた 1 打（曲の時刻にそろえたもの）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub ms: f64,
    pub lane: DrumLane,
    pub velocity: u8,
}

/// 判定の結果 1 つ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JudgeEvent {
    /// `Judge::notes()` の添字
    pub note: usize,
    pub judgement: Judgement,
    /// 叩いた時刻 − 音符の時刻（マイナスは早い）。見逃しは `None`
    pub offset_ms: Option<f64>,
    pub velocity: Option<u8>,
}

/// 成績の集計
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    pub perfect: u32,
    pub great: u32,
    pub good: u32,
    pub poor: u32,
    pub miss: u32,
    pub combo: u32,
    pub max_combo: u32,
    /// 音符の無いところを叩いた数
    pub stray: u32,
}

impl Tally {
    fn add(&mut self, j: Judgement) {
        match j {
            Judgement::Perfect => self.perfect += 1,
            Judgement::Great => self.great += 1,
            Judgement::Good => self.good += 1,
            Judgement::Poor => self.poor += 1,
            Judgement::Miss => self.miss += 1,
        }
        // コンボは Good 以上で続く
        if matches!(j, Judgement::Perfect | Judgement::Great | Judgement::Good) {
            self.combo += 1;
            self.max_combo = self.max_combo.max(self.combo);
        } else {
            self.combo = 0;
        }
    }
}

/// 1 曲ぶんの判定
#[derive(Debug, Clone)]
pub struct Judge {
    notes: Vec<DrumNote>,
    done: Vec<bool>,
    windows: HitWindows,
    groups: LaneGroups,
    /// これより前の音符はすべて判定済み（見逃しの確認をここから始める）
    first_open: usize,
    tally: Tally,
}

impl Judge {
    /// `notes` は時刻順に並んでいること（`TempoMap::drum_notes` の結果）
    pub fn new(notes: Vec<DrumNote>, windows: HitWindows, groups: LaneGroups) -> Self {
        let groups = groups.effective_for(&notes);
        let done = vec![false; notes.len()];
        Self {
            notes,
            done,
            windows,
            groups,
            first_open: 0,
            tally: Tally::default(),
        }
    }

    pub fn notes(&self) -> &[DrumNote] {
        &self.notes
    }

    pub fn is_done(&self, note: usize) -> bool {
        self.done[note]
    }

    pub fn tally(&self) -> &Tally {
        &self.tally
    }

    pub fn groups(&self) -> LaneGroups {
        self.groups
    }

    pub fn windows(&self) -> HitWindows {
        self.windows
    }

    /// 1 打を判定する。取れるレーンごとに、まだ判定していない一番近い音符を探し、
    /// 判定の幅に入ったもののうち一番早い音符を取る。同じ時刻の音符が複数あればまとめて取る。
    /// 何も取れなければ空（空打ち）
    pub fn hit(&mut self, hit: Hit) -> Vec<JudgeEvent> {
        let mut candidates: Vec<usize> = DrumLane::ALL
            .into_iter()
            .filter(|&lane| self.groups.accepts(hit.lane, lane))
            .filter_map(|lane| self.nearest_open(lane, hit.ms))
            .collect();
        let Some(earliest) = candidates
            .iter()
            .map(|&i| self.notes[i].ms)
            .min_by(f64::total_cmp)
        else {
            self.tally.stray += 1;
            return Vec::new();
        };
        candidates.retain(|&i| (self.notes[i].ms - earliest).abs() < 1e-6);
        candidates.sort_unstable();
        candidates
            .into_iter()
            .map(|i| {
                let offset = hit.ms - self.notes[i].ms;
                let judgement = self.windows.classify(offset).unwrap_or(Judgement::Poor);
                self.close(i, judgement);
                JudgeEvent {
                    note: i,
                    judgement,
                    offset_ms: Some(offset),
                    velocity: Some(hit.velocity),
                }
            })
            .collect()
    }

    /// 時刻 `now_ms` までに判定の幅を過ぎた音符を見逃しにする
    pub fn advance(&mut self, now_ms: f64) -> Vec<JudgeEvent> {
        let mut out = Vec::new();
        let mut i = self.first_open;
        while i < self.notes.len() && self.notes[i].ms + self.windows.poor < now_ms {
            if !self.done[i] {
                self.close(i, Judgement::Miss);
                out.push(JudgeEvent {
                    note: i,
                    judgement: Judgement::Miss,
                    offset_ms: None,
                    velocity: None,
                });
            }
            i += 1;
        }
        self.first_open = i;
        out
    }

    fn close(&mut self, i: usize, j: Judgement) {
        self.done[i] = true;
        self.tally.add(j);
    }

    /// `lane` のまだ判定していない音符のうち、判定の幅の中で `ms` に一番近いもの
    fn nearest_open(&self, lane: DrumLane, ms: f64) -> Option<usize> {
        let lo = self
            .notes
            .partition_point(|n| n.ms < ms - self.windows.poor);
        let hi = self
            .notes
            .partition_point(|n| n.ms <= ms + self.windows.poor);
        (lo..hi)
            .filter(|&i| !self.done[i] && self.notes[i].lane == lane)
            .min_by(|&a, &b| {
                (self.notes[a].ms - ms)
                    .abs()
                    .total_cmp(&(self.notes[b].ms - ms).abs())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DrumLane::*;

    fn note(ms: f64, lane: DrumLane) -> DrumNote {
        DrumNote { ms, lane, wav: 1 }
    }

    fn hit(ms: f64, lane: DrumLane) -> Hit {
        Hit {
            ms,
            lane,
            velocity: 100,
        }
    }

    fn judge(notes: Vec<DrumNote>) -> Judge {
        Judge::new(notes, HitWindows::default(), LaneGroups::default())
    }

    #[test]
    fn windows_match_upstream_defaults() {
        let w = HitWindows::default();
        assert_eq!(w.classify(0.0), Some(Judgement::Perfect));
        assert_eq!(w.classify(-34.0), Some(Judgement::Perfect));
        assert_eq!(w.classify(50.0), Some(Judgement::Great));
        assert_eq!(w.classify(-80.0), Some(Judgement::Good));
        assert_eq!(w.classify(117.0), Some(Judgement::Poor));
        assert_eq!(w.classify(118.0), None);
    }

    #[test]
    fn hit_takes_the_nearest_note_and_reports_offset() {
        let mut j = judge(vec![note(1000.0, SD), note(1500.0, SD)]);
        let ev = j.hit(hit(1490.0, SD));
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].note, 1);
        assert_eq!(ev[0].judgement, Judgement::Perfect);
        assert_eq!(ev[0].offset_ms, Some(-10.0));
        // 同じ音符は 2 回取れない
        assert!(j.hit(hit(1495.0, SD)).is_empty());
        assert_eq!(j.tally().stray, 1);
    }

    #[test]
    fn wrong_lane_does_not_count() {
        let mut j = judge(vec![note(1000.0, SD)]);
        assert!(j.hit(hit(1000.0, BD)).is_empty());
    }

    #[test]
    fn too_far_is_a_stray_hit() {
        let mut j = judge(vec![note(1000.0, SD)]);
        assert!(j.hit(hit(1200.0, SD)).is_empty());
        assert!(!j.is_done(0));
    }

    #[test]
    fn misses_after_poor_window() {
        let mut j = judge(vec![note(1000.0, SD), note(2000.0, BD)]);
        assert!(j.advance(1117.0).is_empty());
        let ev = j.advance(1118.0);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].judgement, Judgement::Miss);
        assert_eq!(j.tally().miss, 1);
        assert_eq!(j.tally().combo, 0);
    }

    #[test]
    fn combo_and_max_combo() {
        let mut j = judge(vec![
            note(0.0, SD),
            note(500.0, SD),
            note(1000.0, SD),
            note(1500.0, SD),
        ]);
        j.hit(hit(0.0, SD));
        j.hit(hit(510.0, SD));
        j.hit(hit(1100.0, SD)); // Poor: コンボが切れる
        j.hit(hit(1500.0, SD));
        let t = j.tally();
        assert_eq!((t.perfect, t.poor, t.combo, t.max_combo), (3, 1, 1, 2));
    }

    #[test]
    fn simultaneous_notes_in_a_common_group_are_taken_together() {
        let groups = LaneGroups {
            hh: HhGroup::Common,
            ..Default::default()
        };
        let notes = vec![note(1000.0, HH), note(1000.0, LC), note(1000.0, HHO)];
        let mut j = Judge::new(notes, HitWindows::default(), groups);
        assert_eq!(j.hit(hit(1005.0, HH)).len(), 3);
    }

    #[test]
    fn common_group_takes_the_earliest_note() {
        let groups = LaneGroups {
            ft: FtGroup::Common,
            ..Default::default()
        };
        let mut j = Judge::new(
            vec![note(1000.0, LT), note(1060.0, FT)],
            HitWindows::default(),
            groups,
        );
        let ev = j.hit(hit(1050.0, FT));
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].note, 0);
        assert_eq!(ev[0].judgement, Judgement::Great);
    }

    #[test]
    fn separate_groups_fall_back_when_the_chart_lacks_a_lane() {
        // HHO も LC も無い譜面では、HH の叩き分けは意味が無いので共通になる
        let g = LaneGroups::default().effective_for(&[note(0.0, HH), note(0.0, CY)]);
        assert_eq!(g.hh, HhGroup::Common);
        assert_eq!(g.cy, CyGroup::Common);
        // HHO はあるが LC が無い
        let g = LaneGroups::default().effective_for(&[note(0.0, HHO), note(0.0, RD)]);
        assert_eq!(g.hh, HhGroup::HiHatOnly);
        assert_eq!(g.cy, CyGroup::Separate);
    }

    #[test]
    fn bd_groups() {
        let g = LaneGroups {
            bd: BdGroup::LeftPedalsCommon,
            ..Default::default()
        };
        assert!(g.accepts(LP, LBD));
        assert!(!g.accepts(BD, LP));
        let g = LaneGroups {
            bd: BdGroup::Common,
            ..Default::default()
        };
        assert!(g.accepts(LP, BD));
    }
}
