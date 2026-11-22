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
    /// 狭い幅が広い幅を越えないようにそろえる（PERFECT ≤ GREAT ≤ GOOD ≤ POOR。越えた幅は前の幅まで広げる）
    pub fn ordered(self) -> Self {
        let great = self.great.max(self.perfect);
        let good = self.good.max(great);
        Self {
            perfect: self.perfect,
            great,
            good,
            poor: self.poor.max(good),
        }
    }

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

impl Judgement {
    pub const ALL: [Judgement; 5] = [
        Self::Perfect,
        Self::Great,
        Self::Good,
        Self::Poor,
        Self::Miss,
    ];

    /// 記録に書く名前
    pub fn name(self) -> &'static str {
        match self {
            Self::Perfect => "PERFECT",
            Self::Great => "GREAT",
            Self::Good => "GOOD",
            Self::Poor => "POOR",
            Self::Miss => "MISS",
        }
    }

    /// [`Judgement::name`] の逆
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|j| j.name() == name)
    }
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
    /// 左シンバルと右シンバルを叩き分けない（上流の CymbalFree）。ライドも共通にするかは `cy` に従う
    pub cymbal_free: bool,
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
        let free =
            self.cymbal_free && (pair(LC, CY) || (self.cy == CyGroup::Common && pair(LC, RD)));
        hh || ft || cy || bd || free
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
    /// 叩いた時刻 − 音符の時刻（マイナスは早い）。見逃しと、自動で叩いた音符は `None`
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
    /// 判定を 1 つ数える（コンボも進める）
    pub fn add(&mut self, j: Judgement) {
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
    /// ペダル（BD・LP・LBD）の判定の幅（上流 `stDrumPedalHitRanges`）
    pedal_windows: HitWindows,
    groups: LaneGroups,
    /// これより前の音符はすべて判定済み（見逃しの確認をここから始める）
    first_open: usize,
    tally: Tally,
    /// 自動で叩くレーン（音符のレーン。`DrumLane as usize` の添字）
    auto: [bool; 12],
}

impl Judge {
    /// `notes` は時刻順に並んでいること（`TempoMap::drum_notes` の結果）。
    /// 叩き分けの寄せ方は、この音符にあわせて [`LaneGroups::effective_for`] で決める
    pub fn new(notes: Vec<DrumNote>, windows: HitWindows, groups: LaneGroups) -> Self {
        let groups = groups.effective_for(&notes);
        Self::with_groups(notes, windows, groups)
    }

    /// 叩き分けの寄せ方を `groups` のまま使う。練習では曲全体の音符で決めた寄せ方を渡し、
    /// 区間によって判定が変わらないようにする
    pub fn with_groups(notes: Vec<DrumNote>, windows: HitWindows, groups: LaneGroups) -> Self {
        let done = vec![false; notes.len()];
        Self {
            notes,
            done,
            windows,
            pedal_windows: windows,
            groups,
            first_open: 0,
            tally: Tally::default(),
            auto: [false; 12],
        }
    }

    /// 自動で叩くレーンを決める（上流の AutoPlay）。そのレーンの音符は、時刻が来たら [`Judge::advance`] が
    /// PERFECT で取り、叩いても取らない
    pub fn set_auto(&mut self, lanes: &[DrumLane]) {
        self.auto = [false; 12];
        for &l in lanes {
            self.auto[l as usize] = true;
        }
    }

    /// ペダル（BD・LP・LBD）の音符の判定の幅を決める。決めなければほかのレーンと同じ
    pub fn set_pedal_windows(&mut self, windows: HitWindows) {
        self.pedal_windows = windows;
    }

    /// コンボを切る（上流の Tight で、音符の無いところを叩いたとき。判定の数は変えない）
    pub fn break_combo(&mut self) {
        self.tally.combo = 0;
    }

    /// レーン `lane` の音符を自動で叩くか
    pub fn is_auto(&self, lane: DrumLane) -> bool {
        self.auto[lane as usize]
    }

    /// 自動で叩くレーンがあるか
    pub fn has_auto(&self) -> bool {
        self.auto.iter().any(|&a| a)
    }

    pub fn notes(&self) -> &[DrumNote] {
        &self.notes
    }

    /// 後ろに音符を足す（練習の次の周）。`notes` は時刻順で、今ある最後の音符より前に来ないこと。
    /// 足した音符の添字の範囲を返す
    pub fn extend(&mut self, notes: impl IntoIterator<Item = DrumNote>) -> std::ops::Range<usize> {
        let from = self.notes.len();
        for n in notes {
            debug_assert!(
                self.notes.last().is_none_or(|last| last.ms <= n.ms),
                "notes must be added in time order"
            );
            self.notes.push(n);
            self.done.push(false);
        }
        from..self.notes.len()
    }

    /// 添字 `from` から後ろの音符を取り除く（練習で、まだ始まっていない周を取り下げるとき）。
    /// そのうち 1 つでも判定済みなら何もせず false
    pub fn truncate(&mut self, from: usize) -> bool {
        if from >= self.notes.len() {
            return true;
        }
        if self.done[from..].iter().any(|&d| d) {
            return false;
        }
        self.notes.truncate(from);
        self.done.truncate(from);
        self.first_open = self.first_open.min(from);
        true
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

    /// ペダル以外の判定の幅
    pub fn windows(&self) -> HitWindows {
        self.windows
    }

    pub fn pedal_windows(&self) -> HitWindows {
        self.pedal_windows
    }

    /// レーン `lane` の音符の判定の幅
    pub fn windows_for(&self, lane: DrumLane) -> HitWindows {
        if matches!(lane, DrumLane::BD | DrumLane::LP | DrumLane::LBD) {
            self.pedal_windows
        } else {
            self.windows
        }
    }

    /// POOR の幅のうち広いほう（空打ちがどの周のものかを決めるのに使う）
    pub fn widest_poor(&self) -> f64 {
        self.windows.poor.max(self.pedal_windows.poor)
    }

    /// 1 打を判定する。取れるレーンごとに、まだ判定していない一番近い音符を探し、
    /// 判定の幅に入ったもののうち一番早い音符を取る。同じ時刻の音符が複数あればまとめて取る。
    /// 何も取れなければ空（空打ち）
    pub fn hit(&mut self, hit: Hit) -> Vec<JudgeEvent> {
        let mut candidates: Vec<usize> = DrumLane::ALL
            .into_iter()
            .filter(|&lane| !self.is_auto(lane) && self.groups.accepts(hit.lane, lane))
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
                let judgement = self
                    .windows_for(self.notes[i].lane)
                    .classify(offset)
                    .unwrap_or(Judgement::Poor);
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

    /// 時刻 `now_ms` までに来た自動で叩くレーンの音符を PERFECT で取り、判定の幅を過ぎた音符を見逃しにする
    pub fn advance(&mut self, now_ms: f64) -> Vec<JudgeEvent> {
        let mut out = Vec::new();
        if self.has_auto() {
            let mut i = self.first_open;
            while i < self.notes.len() && self.notes[i].ms <= now_ms {
                if !self.done[i] && self.is_auto(self.notes[i].lane) {
                    self.close(i, Judgement::Perfect);
                    out.push(JudgeEvent {
                        note: i,
                        judgement: Judgement::Perfect,
                        offset_ms: None,
                        velocity: None,
                    });
                }
                i += 1;
            }
        }
        // POOR の幅はレーンで違うことがある（ペダル）。狭いほうの幅を過ぎた音符まで見て、
        // その音符のレーンの幅を過ぎていれば見逃しにする
        let narrowest = self.windows.poor.min(self.pedal_windows.poor);
        let mut i = self.first_open;
        let mut first_open = None;
        while i < self.notes.len() && self.notes[i].ms + narrowest < now_ms {
            let n = &self.notes[i];
            if !self.done[i] && n.ms + self.windows_for(n.lane).poor < now_ms {
                self.close(i, Judgement::Miss);
                out.push(JudgeEvent {
                    note: i,
                    judgement: Judgement::Miss,
                    offset_ms: None,
                    velocity: None,
                });
            }
            if !self.done[i] && first_open.is_none() {
                first_open = Some(i);
            }
            i += 1;
        }
        self.first_open = first_open.unwrap_or(i);
        out
    }

    /// 曲の時刻 `ms` へ飛ぶ（早送り・巻き戻し。上流 `tJumpInSong`）。`ms` より前のまだ判定していない音符は
    /// 数えずに済んだことにし、`ms` から後の音符はもう一度叩けるようにする。集計はそのまま（飛んだプレイは記録しない）
    pub fn jump_to(&mut self, ms: f64) {
        let at = self.notes.partition_point(|n| n.ms < ms);
        for d in &mut self.done[..at] {
            *d = true;
        }
        for d in &mut self.done[at..] {
            *d = false;
        }
        self.first_open = at;
    }

    fn close(&mut self, i: usize, j: Judgement) {
        self.done[i] = true;
        self.tally.add(j);
    }

    /// `lane` のまだ判定していない音符のうち、判定の幅の中で `ms` に一番近いもの
    fn nearest_open(&self, lane: DrumLane, ms: f64) -> Option<usize> {
        let poor = self.windows_for(lane).poor;
        let lo = self.notes.partition_point(|n| n.ms < ms - poor);
        let hi = self.notes.partition_point(|n| n.ms <= ms + poor);
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
        DrumNote {
            ms,
            lane,
            wav: 1,
            at: dtx::NotePos::new(0, 0, 1),
            beat: 0.0,
        }
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
    fn unjudged_notes_at_the_end_can_be_taken_back() {
        let mut j = judge(vec![note(0.0, SD), note(500.0, SD)]);
        let added = j.extend([note(1000.0, SD), note(1500.0, SD)]);
        assert_eq!(added, 2..4);
        j.hit(hit(0.0, SD));
        assert!(j.truncate(2));
        assert_eq!(j.notes().len(), 2);
        // 判定済みの音符があれば取り除かない
        assert!(!j.truncate(0));
        assert_eq!(j.notes().len(), 2);
        // 足し直せる
        assert_eq!(j.extend([note(1200.0, SD)]), 2..3);
        assert!(j.advance(2000.0).len() == 2);
    }

    #[test]
    fn judgement_names_round_trip() {
        for j in Judgement::ALL {
            assert_eq!(Judgement::from_name(j.name()), Some(j));
        }
        assert_eq!(Judgement::from_name("perfect"), None);
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
    fn extended_notes_are_judged_like_the_first_ones() {
        let mut j = judge(vec![note(1000.0, SD)]);
        let added = j.extend(vec![note(2000.0, SD), note(2000.0, BD)]);
        assert_eq!(added, 1..3);
        assert_eq!(j.notes().len(), 3);
        // 足した周の頭を少し早く叩いても、足した音符に当たる
        let ev = j.hit(hit(1990.0, SD));
        assert_eq!(ev[0].note, 1);
        let ev = j.advance(2200.0);
        assert_eq!(ev.iter().map(|e| e.note).collect::<Vec<_>>(), vec![0, 2]);
    }

    #[test]
    fn groups_can_be_kept_as_given() {
        // 区間に HHO が無くても、曲全体で決めた叩き分けのままにする
        let j = Judge::with_groups(
            vec![note(0.0, HH)],
            HitWindows::default(),
            LaneGroups::default(),
        );
        assert_eq!(j.groups().hh, HhGroup::Separate);
    }

    #[test]
    fn auto_lanes_are_taken_on_time_and_not_by_hits() {
        let mut j = Judge::new(
            vec![note(1000.0, BD), note(1000.0, SD), note(1500.0, BD)],
            HitWindows::default(),
            LaneGroups::default(),
        );
        j.set_auto(&[BD]);
        assert!(j.is_auto(BD) && !j.is_auto(SD) && j.has_auto());
        // 叩いても自動のレーンの音符は取らない
        assert!(
            j.hit(Hit {
                ms: 1495.0,
                lane: BD,
                velocity: 100
            })
            .is_empty()
        );
        // 時刻が来たら PERFECT で取る（ずれは無し）。叩くレーンはそのまま
        assert!(j.advance(999.0).is_empty());
        let ev = j.advance(1000.0);
        assert_eq!(ev.len(), 1);
        assert_eq!(
            (ev[0].note, ev[0].judgement, ev[0].offset_ms),
            (0, Judgement::Perfect, None)
        );
        let ev = j.hit(Hit {
            ms: 1010.0,
            lane: SD,
            velocity: 100,
        });
        assert_eq!(ev[0].note, 1);
        let ev = j.advance(2000.0);
        assert_eq!(ev.iter().map(|e| e.note).collect::<Vec<_>>(), vec![2]);
        assert_eq!(j.tally().perfect, 3);
        assert_eq!(j.tally().miss, 0);
    }

    #[test]
    fn jumping_skips_or_reopens_notes() {
        let mut j = Judge::new(
            vec![note(1000.0, SD), note(2000.0, SD), note(3000.0, SD)],
            HitWindows::default(),
            LaneGroups::default(),
        );
        // 早送り: 飛ばした音符は見逃しにも数えない
        j.jump_to(2500.0);
        assert!(j.is_done(0) && j.is_done(1) && !j.is_done(2));
        assert!(j.advance(2600.0).is_empty());
        assert_eq!(j.tally().miss, 0);
        j.hit(Hit {
            ms: 3000.0,
            lane: SD,
            velocity: 100,
        });
        // 巻き戻し: 先の音符をもう一度叩ける
        j.jump_to(1500.0);
        assert!(j.is_done(0) && !j.is_done(1) && !j.is_done(2));
        assert_eq!(
            j.hit(Hit {
                ms: 2000.0,
                lane: SD,
                velocity: 100
            })[0]
                .note,
            1
        );
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

    #[test]
    fn pedals_use_their_own_windows() {
        let mut j = judge(vec![note(1000.0, SD), note(1000.0, BD), note(3000.0, BD)]);
        j.set_pedal_windows(HitWindows {
            perfect: 10.0,
            great: 20.0,
            good: 30.0,
            poor: 40.0,
        });
        assert_eq!(j.widest_poor(), 117.0);
        // 50 ms 遅れ: スネアは GREAT、バスドラムは幅の外で空打ち
        assert_eq!(j.hit(hit(1050.0, SD))[0].judgement, Judgement::Great);
        assert!(j.hit(hit(1050.0, BD)).is_empty());
        // バスドラムは 40 ms を過ぎたら見逃し（スネアの幅を待たない）
        let missed = j.advance(1041.0);
        assert_eq!(missed.len(), 1);
        assert_eq!(missed[0].note, 1);
        assert_eq!(j.hit(hit(3015.0, BD))[0].judgement, Judgement::Great);
    }

    #[test]
    fn narrow_pedal_window_does_not_skip_open_notes() {
        let mut j = judge(vec![note(1000.0, SD), note(1010.0, BD)]);
        j.set_pedal_windows(HitWindows {
            perfect: 10.0,
            great: 20.0,
            good: 30.0,
            poor: 40.0,
        });
        // バスドラムは見逃しになるが、スネアはまだ叩ける
        assert_eq!(j.advance(1060.0).len(), 1);
        assert_eq!(j.hit(hit(1100.0, SD))[0].judgement, Judgement::Poor);
        assert_eq!(j.advance(5000.0).len(), 0);
    }

    #[test]
    fn breaking_the_combo_keeps_the_counts() {
        let mut j = judge(vec![note(1000.0, SD), note(2000.0, SD)]);
        j.hit(hit(1000.0, SD));
        assert_eq!(j.tally().combo, 1);
        j.break_combo();
        j.hit(hit(2000.0, SD));
        let t = j.tally();
        assert_eq!((t.perfect, t.miss, t.combo, t.max_combo), (2, 0, 1, 1));
    }

    #[test]
    fn windows_are_put_in_order() {
        let w = HitWindows {
            perfect: 50.0,
            great: 40.0,
            good: 100.0,
            poor: 90.0,
        }
        .ordered();
        assert_eq!((w.great, w.good, w.poor), (50.0, 100.0, 100.0));
    }

    #[test]
    fn cymbal_free_joins_the_left_and_right_cymbals() {
        let mut g = LaneGroups {
            cymbal_free: true,
            ..Default::default()
        };
        assert!(g.accepts(CY, LC) && g.accepts(LC, CY));
        // ライドはシンバルの寄せ方に従う
        assert!(!g.accepts(LC, RD) && !g.accepts(RD, LC));
        g.cy = CyGroup::Common;
        assert!(g.accepts(LC, RD) && g.accepts(RD, CY));
        assert!(!LaneGroups::default().accepts(CY, LC));
    }
}
