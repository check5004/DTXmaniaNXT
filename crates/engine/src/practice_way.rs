//! 曲の練習（区間練習）のやり方。今までの「段をのぼってクリア」に加えて、目的ごとのやり方を選べる。
//!
//! どのやり方も、選んだ区間を周回する [`crate::practice`] の上に乗る。ここではやり方ごとの決まり
//! （どのレーンを自動にするか、どの周がお手本か、積み上げの長さ、ブレイクで消す小節、消える譜面の割合）だけを決める。
//!
//! 苦手めぐり・フィルだけ・つなぎ目は、いくつかの所（[`Stops`]）を 1 か所 [`TOUR_LAPS`] 周ずつ順にめぐる。
//! めぐる所は、苦手めぐりは記録の苦手な所、フィルだけは [`find_fills`]、つなぎ目は [`find_seams`] が譜面から探す。
//!
//! 待ちモードは、正しいパッドを叩くまで譜面が止まる。時間は気にせず、順番と叩き分けだけを覚える。
//! 点は付けず、[`WAIT_LAPS`] 周で終える（譜面を止めるのは演奏の側）。

use dtx::{DrumLane, DrumNote, NotePos};

use crate::practice::{Attempt, Section, Span};

/// 曲の練習のやり方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Way {
    /// 段をのぼる（今まで）。始めの速度から目標の速度まで 10% ずつ
    #[default]
    Climb,
    /// パート練習。選んだパート（[`Part`]）だけを叩き、ほかは自動で鳴らす
    Part,
    /// お手本と交互。自動で叩く 1 周（見て聞くだけ）と、自分で叩く 1 周を交互に
    Demo,
    /// 積み上げ。1 小節がクリアできたら 2 小節、4 小節と区間ぜんぶまで伸ばす（向きは [`Build`]）
    Build,
    /// 曲のブレイク。曲の音とクリックが 2 小節ずつ消える（判定はそのまま）
    Break,
    /// 消える譜面。段の中で周を重ねるごとに音符が減り、最後の周は見えない。落とした音は次の周で戻る
    Fade,
    /// 苦手めぐり。記録の苦手な所を、まぜた順に 1 か所 2 周ずつめぐる
    Tour,
    /// フィルだけ。譜面からフィルの小節を探し、順にめぐる
    Fills,
    /// つなぎ目。形が変わる所（A メロからサビなど）の前後 1 小節ずつを、順にめぐる
    Seams,
    /// 待ちモード。正しいパッドを叩くまで譜面が止まる。点は付けない
    Wait,
}

impl Way {
    /// 選ぶ順（SD・FT で順に回す）
    pub const ALL: [Way; 10] = [
        Self::Climb,
        Self::Part,
        Self::Demo,
        Self::Wait,
        Self::Tour,
        Self::Build,
        Self::Fills,
        Self::Seams,
        Self::Break,
        Self::Fade,
    ];

    /// 設定と記録に書く名前
    pub fn key(self) -> &'static str {
        match self {
            Self::Climb => "climb",
            Self::Part => "part",
            Self::Demo => "demo",
            Self::Build => "build",
            Self::Break => "break",
            Self::Fade => "fade",
            Self::Tour => "tour",
            Self::Fills => "fills",
            Self::Seams => "seams",
            Self::Wait => "wait",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.key() == s)
    }

    /// 前後のやり方（端は反対の端へ回る）
    pub fn step(self, d: i64) -> Self {
        let n = Self::ALL.len() as i64;
        let i = Self::ALL.iter().position(|&w| w == self).unwrap_or(0) as i64;
        Self::ALL[(i + d).rem_euclid(n) as usize]
    }

    /// いくつかの所をめぐるやり方（苦手めぐり・フィルだけ・つなぎ目）
    pub fn tours(self) -> bool {
        matches!(self, Self::Tour | Self::Fills | Self::Seams)
    }

    /// 1 段の周の数（`step_laps` は選んだ数）。お手本と交互は、お手本と自分を 1 組にするので偶数に切り上げる。
    /// めぐるやり方は、1 か所 [`TOUR_LAPS`] 周。待ちモードは [`WAIT_LAPS`] 周
    pub fn laps_per_step(self, step_laps: u32) -> u32 {
        let n = step_laps.max(1);
        match self {
            Self::Demo => n.div_ceil(2) * 2,
            w if w.tours() => TOUR_LAPS,
            Self::Wait => WAIT_LAPS,
            _ => n,
        }
    }

    /// 段の `k` 周目（1 から）がお手本の周か。お手本と交互は奇数の周（段の頭はいつもお手本）
    pub fn demo_lap(self, k: usize) -> bool {
        self == Self::Demo && k % 2 == 1
    }
}

/// パート練習で叩くパート
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Part {
    /// 手（足のパッドのほか全部）
    #[default]
    Hands,
    /// 足（バスドラム・左ペダル・左バスドラム）
    Feet,
    HiHat,
    Snare,
    Toms,
    Cymbals,
}

impl Part {
    pub const ALL: [Part; 6] = [
        Self::Hands,
        Self::Feet,
        Self::HiHat,
        Self::Snare,
        Self::Toms,
        Self::Cymbals,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Hands => "hands",
            Self::Feet => "feet",
            Self::HiHat => "hihat",
            Self::Snare => "snare",
            Self::Toms => "toms",
            Self::Cymbals => "cymbals",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == s)
    }

    /// 自分で叩くレーン
    pub fn lanes(self) -> &'static [DrumLane] {
        use DrumLane::*;
        match self {
            Self::Hands => &[HH, SD, HT, LT, CY, FT, HHO, RD, LC],
            Self::Feet => &[BD, LP, LBD],
            Self::HiHat => &[HH, HHO],
            Self::Snare => &[SD],
            Self::Toms => &[HT, LT, FT],
            Self::Cymbals => &[CY, RD, LC],
        }
    }

    /// 自動で叩くレーン（パートの外）
    pub fn auto_lanes(self) -> Vec<DrumLane> {
        DrumLane::ALL
            .into_iter()
            .filter(|l| !self.lanes().contains(l))
            .collect()
    }

    /// レーンの集まり（`DrumLane as usize` のビット）
    pub fn mask(self) -> u16 {
        lane_mask(self.lanes().iter().copied())
    }
}

/// レーンの集まりをビットにする
pub fn lane_mask(lanes: impl IntoIterator<Item = DrumLane>) -> u16 {
    lanes.into_iter().fold(0, |m, l| m | 1 << l as u16)
}

/// 積み上げの向き
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Build {
    /// 区間の頭から後ろへ伸ばす
    #[default]
    Forward,
    /// 区間の終わりから前へ伸ばす（終わりの方が難しいとき）
    Backward,
}

impl Build {
    pub const ALL: [Build; 2] = [Self::Forward, Self::Backward];

    pub fn key(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Backward => "backward",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.key() == s)
    }
}

/// 積み上げの長さ（小節）。1・2・4・8 … と倍にし、最後は区間ぜんぶ
pub fn build_lengths(measures: u32) -> Vec<u32> {
    let n = measures.max(1);
    let mut v = Vec::new();
    let mut len = 1;
    while len < n {
        v.push(len);
        len *= 2;
    }
    v.push(n);
    v
}

/// 区間 `s` のうち、積み上げの長さ `len` 小節の所
pub fn build_section(s: Section, len: u32, build: Build) -> Section {
    let len = len.clamp(1, s.measures());
    match build {
        Build::Forward => Section::new(s.first, s.first + len - 1),
        Build::Backward => Section::new(s.last + 1 - len, s.last),
    }
}

/// 曲のブレイクで、続けて鳴らす・消す小節の数
pub const BREAK_BARS: u32 = 2;

/// 曲のブレイクで、練習を始めてから `bar` 小節目（0 から。周をまたいで数える）を消すか。
/// 2 小節鳴らして 2 小節消す、をくり返す
pub fn break_muted(bar: u32) -> bool {
    (bar / BREAK_BARS) % 2 == 1
}

/// 鳴らす範囲 `span` から、曲の時刻の範囲 `cuts`（時刻順、重ならない）を除いた残り。
/// `speed` は周の速度（残りの鳴らし始めの実時間を出すのに使う）
pub fn cut_span(span: Span, cuts: &[(f64, f64)], speed: f64) -> Vec<Span> {
    let mut out = Vec::new();
    let mut from = span.from_ms;
    let at = |ms: f64| span.at_ms + (ms - span.from_ms) / speed.max(0.01);
    for &(a, b) in cuts {
        if b <= from || a >= span.to_ms {
            continue;
        }
        if a > from {
            out.push(Span {
                from_ms: from,
                to_ms: a,
                at_ms: at(from),
            });
        }
        from = from.max(b);
    }
    if from < span.to_ms {
        out.push(Span {
            from_ms: from,
            to_ms: span.to_ms,
            at_ms: at(from),
        });
    }
    out
}

/// 消える譜面で、段の `k` 周目（1 から、`laps` 周のうち）に見せる音符の割合（0〜1）。
/// 1 周目はぜんぶ、最後の周は 0
pub fn fade_share(k: usize, laps: usize) -> f64 {
    if laps <= 1 {
        return 0.0;
    }
    let k = k.clamp(1, laps);
    1.0 - (k - 1) as f64 / (laps - 1) as f64
}

/// 消える譜面で、音符を隠す順番（0〜1）。小節と位置とレーンから決め、周が変わっても同じ音符は同じ値
/// （割合が減るほど隠れる音符が増える。低い値の音符ほど長く見える）
pub fn fade_rank(at: NotePos, lane: DrumLane) -> f64 {
    let mut h: u32 = 0x811c_9dc5;
    for b in at
        .measure
        .to_le_bytes()
        .into_iter()
        .chain(at.num.to_le_bytes())
        .chain(at.den.to_le_bytes())
        .chain([lane as u8])
    {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    // 下の桁は偏るので、混ぜてから上の桁を使う
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    f64::from(h >> 8) / f64::from(1u32 << 24)
}

/// めぐるやり方で、1 か所を叩く周の数
pub const TOUR_LAPS: u32 = 2;
/// めぐる所の数の上限
pub const MAX_STOPS: usize = 5;
/// 待ちモードの周の数。3 周目には順番が体に入っている
pub const WAIT_LAPS: u32 = 3;

/// めぐる所（めぐる順）。練習の決めごとに入れて写せるよう、決まった大きさの入れ物にする
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stops {
    list: [Section; MAX_STOPS],
    len: u8,
}

impl Default for Stops {
    fn default() -> Self {
        Self {
            list: [Section::new(0, 0); MAX_STOPS],
            len: 0,
        }
    }
}

impl Stops {
    /// `v` の頭から [`MAX_STOPS`] か所まで
    pub fn new(v: &[Section]) -> Self {
        let mut s = Self::default();
        for (slot, &x) in s.list.iter_mut().zip(v) {
            *slot = x;
        }
        s.len = v.len().min(MAX_STOPS) as u8;
        s
    }

    pub fn as_slice(&self) -> &[Section] {
        &self.list[..usize::from(self.len)]
    }

    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// めぐる所をすべて含む区間（最初の所の頭から最後の所の終わりまで）
    pub fn cover(&self) -> Option<Section> {
        let v = self.as_slice();
        let first = v.iter().map(|s| s.first).min()?;
        let last = v.iter().map(|s| s.last).max()?;
        Some(Section::new(first, last))
    }
}

/// `v` を `seed` で決まる順にまぜる（同じ `seed` なら同じ順）
pub fn shuffle<T>(v: &mut [T], seed: u64) {
    // xorshift64*（0 は動かないので混ぜてから）
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    if x == 0 {
        x = 1;
    }
    let mut next = move || {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for i in (1..v.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
}

/// めぐった 1 か所の出来
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// ◎ ノーミス
    Clean,
    /// ○ ミスはあったが、同じ所を続けては落としていない
    Fair,
    /// × 同じ所を 2 周とも落とした
    Weak,
}

impl Mark {
    /// 1 か所の挑戦（その所の周をまとめたもの）から。最後まで叩いた周が無ければ None
    pub fn of(a: &Attempt) -> Option<Self> {
        if a.laps == 0 {
            return None;
        }
        Some(if !a.spots.weak(TOUR_LAPS).is_empty() {
            Self::Weak
        } else if a.missed > 0 {
            Self::Fair
        } else {
            Self::Clean
        })
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Clean => "◎",
            Self::Fair => "○",
            Self::Weak => "×",
        }
    }
}

/// 小節の中の位置を 48 分の 1 の刻みにする（3 連も 16 分も収まる）
const GRID: u32 = 48;

/// 小節ごとの音符（48 分の位置、レーン）。位置とレーンの順で、同じものは 1 つ
fn hits_per_measure(notes: &[DrumNote], measures: usize) -> Vec<Vec<(u32, DrumLane)>> {
    let mut v: Vec<Vec<(u32, DrumLane)>> = vec![Vec::new(); measures];
    for n in notes {
        let Some(m) = v.get_mut(n.at.measure as usize) else {
            continue;
        };
        let den = n.at.den.max(1);
        let pos = ((u64::from(n.at.num) * u64::from(GRID) + u64::from(den) / 2) / u64::from(den))
            .min(u64::from(GRID - 1)) as u32;
        m.push((pos, n.lane));
    }
    for m in &mut v {
        m.sort_by_key(|&(p, l)| (p, l as u8));
        m.dedup();
    }
    v
}

/// 2 つの小節の音符の重なり（共通 / 合わせた数。0〜1）。どちらも空なら 1
fn overlap(a: &[(u32, DrumLane)], b: &[(u32, DrumLane)]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let common = a.iter().filter(|x| b.contains(x)).count();
    common as f64 / (a.len() + b.len() - common) as f64
}

fn half(hits: &[(u32, DrumLane)], second: bool) -> Vec<(u32, DrumLane)> {
    hits.iter()
        .copied()
        .filter(|&(p, _)| (p >= GRID / 2) == second)
        .collect()
}

fn is_tom(l: DrumLane) -> bool {
    matches!(l, DrumLane::HT | DrumLane::LT | DrumLane::FT)
}

/// リズムを刻むレーン（ハイハット・ライド）
fn keeps_time(l: DrumLane) -> bool {
    matches!(l, DrumLane::HH | DrumLane::HHO | DrumLane::RD)
}

/// 小節 `m` がフィルらしいか。らしければ強さ（大きいほどフィルらしい）
fn fill_score(hits: &[Vec<(u32, DrumLane)>], m: usize) -> Option<f64> {
    let h = &hits[m];
    if h.len() < 3 {
        return None;
    }
    let prev = m.checked_sub(1).map(|p| hits[p].as_slice()).unwrap_or(&[]);
    let next = hits.get(m + 1).map(Vec::as_slice).unwrap_or(&[]);
    let toms = h.iter().filter(|x| is_tom(x.1)).count();
    let like_prev = overlap(h, prev);
    // タムが多く、前後の小節と形が違う（タムで刻むビートは前後と同じ形なので外れる）
    if toms >= 3 && like_prev < 0.8 && overlap(h, next) < 0.8 {
        return Some(toms as f64 + (1.0 - like_prev) * 4.0);
    }
    if prev.len() < 3 {
        return None;
    }
    // 刻みが止まる（前の小節は 4 打以上刻んでいたのに、この小節は刻まない）
    let kept = |x: &[(u32, DrumLane)]| x.iter().filter(|y| keeps_time(y.1)).count();
    if kept(prev) >= 4 && kept(h) == 0 && h.len() >= 4 {
        return Some(2.0 + (1.0 - like_prev) * 4.0);
    }
    // 前半は前の小節と同じビートで、後半だけ形が変わる（前の 2 小節は同じビート）
    let steady = m >= 2 && overlap(prev, &hits[m - 2]) >= 0.6;
    let tail = half(h, true);
    if steady
        && tail.len() >= 3
        && overlap(&half(h, false), &half(prev, false)) >= 0.5
        && overlap(&tail, &half(prev, true)) < 0.4
    {
        return Some(1.0 + (1.0 - like_prev) * 4.0);
    }
    None
}

/// 譜面からフィルを探す。フィルの小節（続くなら 2 小節まで）を、曲の順に [`MAX_STOPS`] か所まで。
/// 同じ形のフィルは最初の 1 つだけ。多すぎれば、フィルらしいものから選ぶ
pub fn find_fills(notes: &[DrumNote], measures: usize) -> Vec<Section> {
    let hits = hits_per_measure(notes, measures);
    let scores: Vec<Option<f64>> = (0..measures).map(|m| fill_score(&hits, m)).collect();
    let mut found: Vec<(Section, f64)> = Vec::new();
    let mut m = 0;
    while m < measures {
        let Some(score) = scores[m] else {
            m += 1;
            continue;
        };
        // 続くフィルは 2 小節までまとめる
        let two = scores.get(m + 1).copied().flatten();
        let (s, score) = match two {
            Some(b) => (Section::new(m as u32, m as u32 + 1), score.max(b)),
            None => (Section::new(m as u32, m as u32), score),
        };
        m = s.last as usize + 1;
        let same = found.iter().any(|(f, _)| {
            f.measures() == s.measures()
                && (0..s.measures()).all(|i| {
                    overlap(&hits[(f.first + i) as usize], &hits[(s.first + i) as usize]) >= 0.85
                })
        });
        if !same {
            found.push((s, score));
        }
    }
    pick(found)
}

/// 強いものから [`MAX_STOPS`] か所選び、曲の順に並べる
fn pick(mut found: Vec<(Section, f64)>) -> Vec<Section> {
    if found.len() > MAX_STOPS {
        found.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.first.cmp(&b.0.first)));
        found.truncate(MAX_STOPS);
    }
    let mut v: Vec<Section> = found.into_iter().map(|f| f.0).collect();
    v.sort_by_key(|s| s.first);
    v
}

/// 形を比べるレーンのまとまり（ハイハット・ライド・シンバル・スネア・バスドラム・タム・左ペダル）
const GROUPS: usize = 7;

fn group(l: DrumLane) -> usize {
    use DrumLane::*;
    match l {
        HH | HHO => 0,
        RD => 1,
        CY | LC => 2,
        SD => 3,
        BD | LBD => 4,
        HT | LT | FT => 5,
        LP => 6,
    }
}

/// 小節の形：まとまりごとの音符の割合と、音符の数
fn profile(h: &[(u32, DrumLane)]) -> ([f64; GROUPS], f64) {
    let mut p = [0.0; GROUPS];
    for &(_, l) in h {
        p[group(l)] += 1.0;
    }
    let n = h.len() as f64;
    if n > 0.0 {
        for x in &mut p {
            *x /= n;
        }
    }
    (p, n)
}

/// 小節の形の平均
fn mean_profile(hits: &[Vec<(u32, DrumLane)>], ms: &[usize]) -> Option<([f64; GROUPS], f64)> {
    if ms.is_empty() {
        return None;
    }
    let mut p = [0.0; GROUPS];
    let mut n = 0.0;
    for &m in ms {
        let (q, c) = profile(&hits[m]);
        for (a, b) in p.iter_mut().zip(q) {
            *a += b;
        }
        n += c;
    }
    let k = ms.len() as f64;
    for x in &mut p {
        *x /= k;
    }
    Some((p, n / k))
}

/// 小節 `m` の前で形が変わる強さ（0 から。大きいほどはっきり変わる）。前後に音符が無ければ None
fn seam_score(hits: &[Vec<(u32, DrumLane)>], fills: &[bool], m: usize) -> Option<f64> {
    // フィルの小節は変わり目ではない（変わり目はフィルの次の小節）
    if m == 0 || hits[m - 1].is_empty() || hits[m].len() < 2 || fills[m] {
        return None;
    }
    // 前は、フィルを除いた近い 2 小節。後ろは m とその次（フィルなら m だけ）
    let before: Vec<usize> = (m.saturating_sub(4)..m)
        .rev()
        .filter(|&i| !fills[i] && hits[i].len() >= 2)
        .take(2)
        .collect();
    let mut after = vec![m];
    if hits.get(m + 1).is_some_and(|h| h.len() >= 2) && !fills[m + 1] {
        after.push(m + 1);
    }
    let (a, na) = mean_profile(hits, &before)?;
    let (b, nb) = mean_profile(hits, &after)?;
    let shape: f64 = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).sum::<f64>() / 2.0;
    let density = (nb / na.max(1.0)).ln().abs().min(1.0);
    let crash = |h: &[(u32, DrumLane)]| h.iter().any(|&(p, l)| p == 0 && group(l) == 2);
    let accent = if crash(&hits[m]) && !crash(&hits[m - 1]) {
        0.1
    } else {
        0.0
    };
    Some(shape + density * 0.5 + accent)
}

/// 形が変わるとみなす強さの下限
const SEAM_MIN: f64 = 0.3;

/// 譜面から、形が変わる所（A メロからサビなど）を探す。変わる前の 1 小節と後の 1 小節を、曲の順に
/// [`MAX_STOPS`] か所まで。近い（4 小節以内の）所は、はっきり変わる方だけ
pub fn find_seams(notes: &[DrumNote], measures: usize) -> Vec<Section> {
    let hits = hits_per_measure(notes, measures);
    let fills: Vec<bool> = (0..measures)
        .map(|m| fill_score(&hits, m).is_some())
        .collect();
    let mut cands: Vec<(usize, f64)> = (1..measures)
        .filter_map(|m| seam_score(&hits, &fills, m).map(|s| (m, s)))
        .filter(|&(_, s)| s >= SEAM_MIN)
        .collect();
    cands.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut taken: Vec<(Section, f64)> = Vec::new();
    for (m, s) in cands {
        if taken
            .iter()
            .all(|(t, _)| (t.last as i64 - m as i64).abs() > 4)
        {
            taken.push((Section::new(m as u32 - 1, m as u32), s));
        }
    }
    pick(taken)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_step_wraps() {
        for w in Way::ALL {
            assert_eq!(Way::from_key(w.key()), Some(w));
        }
        for p in Part::ALL {
            assert_eq!(Part::from_key(p.key()), Some(p));
        }
        for b in Build::ALL {
            assert_eq!(Build::from_key(b.key()), Some(b));
        }
        assert_eq!(Way::Climb.step(-1), Way::Fade);
        assert_eq!(Way::Fade.step(1), Way::Climb);
        assert_eq!(Way::Climb.step(1), Way::Part);
    }

    #[test]
    fn parts_split_hands_and_feet() {
        let hands = Part::Hands.mask();
        let feet = Part::Feet.mask();
        assert_eq!(hands & feet, 0);
        assert_eq!(hands | feet, lane_mask(DrumLane::ALL));
        assert!(Part::Toms.auto_lanes().contains(&DrumLane::SD));
        assert!(!Part::Toms.auto_lanes().contains(&DrumLane::FT));
    }

    #[test]
    fn demo_laps_pair_up() {
        assert_eq!(Way::Demo.laps_per_step(3), 4);
        assert_eq!(Way::Demo.laps_per_step(4), 4);
        assert_eq!(Way::Climb.laps_per_step(3), 3);
        assert_eq!(Way::Wait.laps_per_step(5), WAIT_LAPS);
        assert!(Way::Demo.demo_lap(1) && !Way::Demo.demo_lap(2) && Way::Demo.demo_lap(3));
        assert!(!Way::Climb.demo_lap(1));
    }

    #[test]
    fn build_doubles_up_to_the_whole_section() {
        assert_eq!(build_lengths(1), vec![1]);
        assert_eq!(build_lengths(4), vec![1, 2, 4]);
        assert_eq!(build_lengths(6), vec![1, 2, 4, 6]);
        let s = Section::new(10, 15);
        assert_eq!(build_section(s, 2, Build::Forward), Section::new(10, 11));
        assert_eq!(build_section(s, 2, Build::Backward), Section::new(14, 15));
        assert_eq!(build_section(s, 9, Build::Backward), s);
    }

    #[test]
    fn cutting_a_span_keeps_the_rest_in_place() {
        let span = Span {
            from_ms: 0.0,
            to_ms: 1000.0,
            at_ms: 500.0,
        };
        let v = cut_span(span, &[(200.0, 400.0), (800.0, 1200.0)], 0.5);
        assert_eq!(
            v,
            vec![
                Span {
                    from_ms: 0.0,
                    to_ms: 200.0,
                    at_ms: 500.0
                },
                Span {
                    from_ms: 400.0,
                    to_ms: 800.0,
                    at_ms: 1300.0
                },
            ]
        );
        assert_eq!(cut_span(span, &[], 1.0), vec![span]);
        assert!(cut_span(span, &[(-10.0, 2000.0)], 1.0).is_empty());
    }

    #[test]
    fn breaks_go_two_on_two_off() {
        let v: Vec<bool> = (0..8).map(break_muted).collect();
        assert_eq!(v, [false, false, true, true, false, false, true, true]);
    }

    fn pos(measure: u32, num: u32, den: u32) -> NotePos {
        NotePos { measure, num, den }
    }

    /// 4/4・120 BPM の小節 `m` の、16 分の位置 `step` の音符
    fn note(m: u32, step: u32, lane: DrumLane) -> DrumNote {
        let beat = f64::from(m * 4) + f64::from(step) / 4.0;
        DrumNote {
            ms: beat * 500.0,
            lane,
            wav: 1,
            at: NotePos::new(m, step, 16),
            beat,
        }
    }

    /// 16 分 16 個の文字列（`x` が音符）を、レーンごとに小節 `m` の音符にする
    fn bar(m: u32, rows: &[(DrumLane, &str)]) -> Vec<DrumNote> {
        rows.iter()
            .flat_map(|&(lane, s)| {
                s.chars()
                    .enumerate()
                    .filter(|&(_, c)| c == 'x')
                    .map(move |(i, _)| note(m, i as u32, lane))
            })
            .collect()
    }

    use DrumLane::{BD, CY, FT, HH, HT, LT, RD, SD};

    fn verse(m: u32) -> Vec<DrumNote> {
        bar(
            m,
            &[
                (HH, "x.x.x.x.x.x.x.x."),
                (SD, "....x.......x..."),
                (BD, "x.......x.x....."),
            ],
        )
    }

    fn chorus(m: u32) -> Vec<DrumNote> {
        bar(
            m,
            &[
                (CY, "x..............."),
                (RD, "..x.x.x.x.x.x.x."),
                (SD, "....x.......x..."),
                (BD, "x.x...x.x.x...x."),
            ],
        )
    }

    /// ビートの前半のまま、後半をタムにしたフィル
    fn tom_fill(m: u32) -> Vec<DrumNote> {
        bar(
            m,
            &[
                (HH, "x.x.x.x........."),
                (SD, "....x.......xx.."),
                (HT, "........xx......"),
                (LT, "..........xx...."),
                (FT, "..............xx"),
                (BD, "x.......x......."),
            ],
        )
    }

    /// 刻みを止めてスネアだけのフィル
    fn snare_fill(m: u32) -> Vec<DrumNote> {
        bar(m, &[(SD, "x.x.x.x.xxxxxxxx"), (BD, "x.......x.......")])
    }

    /// A メロ 8 小節（8 小節目はタムのフィル）→ サビ 8 小節（8 小節目はスネアのフィル）→ A メロ 8 小節
    /// （8 小節目は最初と同じフィル）→ サビ 4 小節
    fn song() -> (Vec<DrumNote>, usize) {
        let mut v = Vec::new();
        for m in 0..28 {
            v.extend(match m {
                7 | 23 => tom_fill(m),
                15 => snare_fill(m),
                0..8 | 16..24 => verse(m),
                _ => chorus(m),
            });
        }
        v.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        (v, 28)
    }

    #[test]
    fn fills_are_found_once_per_shape() {
        let (notes, n) = song();
        // 23 小節目は 7 小節目と同じ形なので入れない
        assert_eq!(
            find_fills(&notes, n),
            vec![Section::new(7, 7), Section::new(15, 15)]
        );
        // ビートだけの曲にはフィルが無い
        let plain: Vec<DrumNote> = (0..8).flat_map(verse).collect();
        assert!(find_fills(&plain, 8).is_empty());
        // タムで刻むビートはフィルではない
        let toms: Vec<DrumNote> = (0..8)
            .flat_map(|m| bar(m, &[(FT, "x.x.x.x.x.x.x.x."), (SD, "....x.......x...")]))
            .collect();
        assert!(find_fills(&toms, 8).is_empty());
    }

    #[test]
    fn seams_take_a_measure_on_each_side() {
        let (notes, n) = song();
        assert_eq!(
            find_seams(&notes, n),
            vec![
                Section::new(7, 8),
                Section::new(15, 16),
                Section::new(23, 24)
            ]
        );
        let plain: Vec<DrumNote> = (0..8).flat_map(verse).collect();
        assert!(find_seams(&plain, 8).is_empty());
    }

    #[test]
    fn stops_hold_up_to_five_and_shuffle_keeps_them_all() {
        let v: Vec<Section> = (0..7).map(|i| Section::new(i * 4, i * 4 + 1)).collect();
        let s = Stops::new(&v);
        assert_eq!(s.len(), MAX_STOPS);
        assert_eq!(s.as_slice(), &v[..MAX_STOPS]);
        assert_eq!(s.cover(), Some(Section::new(0, 17)));
        assert_eq!(Stops::default().cover(), None);
        let mut a: Vec<u32> = (0..10).collect();
        shuffle(&mut a, 42);
        let mut b: Vec<u32> = (0..10).collect();
        shuffle(&mut b, 42);
        assert_eq!(a, b);
        assert_ne!(a, (0..10).collect::<Vec<_>>());
        a.sort_unstable();
        assert_eq!(a, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn marks_go_clean_fair_weak() {
        use crate::practice::{BeatSlot, SpotTally};
        let slot = BeatSlot {
            measure: 3,
            beat: 1,
        };
        let attempt = |laps: &[&[(BeatSlot, DrumLane)]]| {
            let mut spots = SpotTally::default();
            for l in laps {
                spots.add_lap(l);
            }
            Attempt {
                index: 0,
                speed: 100,
                laps: laps.len() as u32,
                clean_laps: laps.iter().filter(|l| l.is_empty()).count() as u32,
                notes: 20,
                missed: laps.iter().map(|l| l.len() as u32).sum(),
                spots,
                weak_times: 3,
                passed: Some(true),
            }
        };
        assert_eq!(Mark::of(&attempt(&[&[], &[]])), Some(Mark::Clean));
        assert_eq!(Mark::of(&attempt(&[&[(slot, SD)], &[]])), Some(Mark::Fair));
        // ゆるい決まりでも、2 周とも同じ所なら ×
        assert_eq!(
            Mark::of(&attempt(&[&[(slot, SD)], &[(slot, SD)]])),
            Some(Mark::Weak)
        );
        assert_eq!(Mark::of(&attempt(&[])), None);
    }

    #[test]
    fn fading_goes_from_all_to_none() {
        assert_eq!(fade_share(1, 4), 1.0);
        assert!((fade_share(2, 4) - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(fade_share(4, 4), 0.0);
        assert_eq!(fade_share(1, 1), 0.0);
        // 隠す順番はばらけていて、同じ音符は同じ値
        let ranks: Vec<f64> = (0..200)
            .map(|i| fade_rank(pos(i / 16, i % 16, 16), DrumLane::SD))
            .collect();
        assert!(ranks.iter().all(|r| (0.0..1.0).contains(r)));
        let low = ranks.iter().filter(|&&r| r < 0.5).count();
        assert!((70..130).contains(&low), "{low}");
        assert_eq!(
            fade_rank(pos(3, 1, 4), DrumLane::HH),
            fade_rank(pos(3, 1, 4), DrumLane::HH)
        );
    }
}
