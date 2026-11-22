//! スネアの打ち分け（ヘッドとリム）。
//!
//! 譜面のスネアの音符は、リムの音らしい音（[`dtx::Chart::rim_wavs`]）ならリムの音符、ほかは普通の音符とする。
//! 叩いた所は電子ドラムが送るノート番号の叩き方（[`Articulation`]）で分かる。決まりは次のとおり。
//!
//! | 譜面の音符 | ヘッド | リム（リムショット） | クロススティック |
//! | --- | --- | --- | --- |
//! | 普通の音符 | 合っている | 合っている | 違う（ヘッドで叩く） |
//! | リムの音符 | 違う（リムで叩く） | 合っている | 合っている |
//!
//! 普通の音符をリムショット（ヘッドとリムを同時に）で叩くのは、アクセントでよくやるので合っているとする。
//! リムの音符は、名前からはリムショットかクロススティックか分からないことが多いので、どちらでも合っているとする。
//! 判定（PERFECT など）は変えない。違う所を叩いたことを知らせ、結果で数えるだけに使う。

use crate::input::Articulation;

/// スネアのどこで叩く音符か / 叩いた音を鳴らすならどちらの音か
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stroke {
    Head,
    Rim,
}

impl Stroke {
    /// 音符の種類（リムの音らしい音ならリム）
    pub fn of_note(rim: bool) -> Self {
        if rim { Self::Rim } else { Self::Head }
    }

    /// 叩いた所が、音符 `self` に合っていないなら、叩くべきだった方（`self`）。合っていれば `None`
    pub fn wrong_for(self, hit: Articulation) -> Option<Self> {
        let ok = match self {
            Self::Head => hit != Articulation::CrossStick,
            Self::Rim => matches!(hit, Articulation::Rim | Articulation::CrossStick),
        };
        (!ok).then_some(self)
    }

    /// 叩いた所で鳴らす音の種類。音符に合っていればその音符の種類、違っていれば叩いた所の種類
    /// （リムの音符をヘッドで叩いたらヘッドの音、普通の音符をクロススティックで叩いたらリムの音）
    pub fn sound_for(self, hit: Articulation) -> Self {
        match self.wrong_for(hit) {
            None => self,
            Some(Self::Head) => Self::Rim,
            Some(Self::Rim) => Self::Head,
        }
    }
}

/// 1 回の演奏のスネアの打ち分けの数
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StrokeTally {
    /// リムの音符を、電子ドラムで叩いた数（キーボードと自動は数えない）と、そのうち合っていた数
    pub rim_hit: u32,
    pub rim_ok: u32,
    /// 普通の音符を、電子ドラムで叩いた数と、そのうち合っていた数
    pub head_hit: u32,
    pub head_ok: u32,
    /// リムの音符をヘッドで叩いた小節（小さい順、重なりなし）
    pub rim_missed_measures: Vec<u32>,
}

impl StrokeTally {
    /// スネアの音符を 1 つ数える。`measure` は音符の小節
    pub fn add(&mut self, note: Stroke, hit: Articulation, measure: u32) {
        let ok = note.wrong_for(hit).is_none();
        match note {
            Stroke::Rim => {
                self.rim_hit += 1;
                if ok {
                    self.rim_ok += 1;
                } else if let Err(i) = self.rim_missed_measures.binary_search(&measure) {
                    self.rim_missed_measures.insert(i, measure);
                }
            }
            Stroke::Head => {
                self.head_hit += 1;
                if ok {
                    self.head_ok += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Articulation::*;

    #[test]
    fn rules() {
        assert_eq!(Stroke::Head.wrong_for(Head), None);
        assert_eq!(Stroke::Head.wrong_for(Rim), None);
        assert_eq!(Stroke::Head.wrong_for(CrossStick), Some(Stroke::Head));
        assert_eq!(Stroke::Rim.wrong_for(Head), Some(Stroke::Rim));
        assert_eq!(Stroke::Rim.wrong_for(Rim), None);
        assert_eq!(Stroke::Rim.wrong_for(CrossStick), None);
        // スネアにシンバルのパッドを割り当てたときなど（ヘッドと同じ扱い）
        assert_eq!(Stroke::Rim.wrong_for(Edge), Some(Stroke::Rim));
        assert_eq!(Stroke::Head.wrong_for(Bow), None);
    }

    #[test]
    fn sounds_follow_where_you_hit() {
        assert_eq!(Stroke::Rim.sound_for(Head), Stroke::Head);
        assert_eq!(Stroke::Rim.sound_for(CrossStick), Stroke::Rim);
        assert_eq!(Stroke::Head.sound_for(CrossStick), Stroke::Rim);
        // リムショットは普通の音符の音のまま
        assert_eq!(Stroke::Head.sound_for(Rim), Stroke::Head);
    }

    #[test]
    fn tally() {
        let mut t = StrokeTally::default();
        t.add(Stroke::Rim, Rim, 3);
        t.add(Stroke::Rim, Head, 12);
        t.add(Stroke::Rim, Head, 5);
        t.add(Stroke::Rim, Head, 12);
        t.add(Stroke::Head, Head, 1);
        t.add(Stroke::Head, CrossStick, 1);
        assert_eq!((t.rim_hit, t.rim_ok, t.head_hit, t.head_ok), (4, 1, 2, 1));
        assert_eq!(t.rim_missed_measures, vec![5, 12]);
    }
}
