//! 設定の項目の表。1 項目 1 件で、config.toml のキー・分類・日本語の名前・種類（ON/OFF・選択肢・数・まとめて切り替え）・
//! 選択肢か範囲と刻み・既定・説明・演奏中に変えてよいか を持つ。設定の画面はこの表から行を作る。
//! 値は config.toml の `[settings]` の表にキーごとに書く。書いていないもの、読めないものは既定。
//!
//! NX から移す設定はここに足していく（棚卸し: https://claude.ai/artifact/AH4xxgyx2hqybH6Q5Kb1rw ）。
//! 前からある設定（曲のフォルダ、補正、音量など）は `Config` の欄のまま。

use std::collections::BTreeMap;

use dtx::DrumLane;
use serde::{Deserialize, Serialize};

/// 分類（設定の画面を作り直すときの 8 つの分類。名前は [`Group::label`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// 演奏（演奏の決まり、自動で叩く、叩き分け、判定）
    Play,
    /// 譜面の見た目（流れ方、見えにくくする、ずれの表示、画面の部品）
    View,
    /// タイミング（補正）
    Timing,
    /// 音（音量、鳴らすもの）
    Sound,
    /// 入力機器
    Input,
    /// 曲
    Songs,
    /// 練習
    Practice,
    /// アプリ
    App,
}

impl Group {
    #[expect(dead_code, reason = "設定の画面を作り直すとき、分類の名前に使う")]
    pub fn label(self) -> &'static str {
        match self {
            Self::Play => "演奏",
            Self::View => "譜面の見た目",
            Self::Timing => "タイミング",
            Self::Sound => "音",
            Self::Input => "入力機器",
            Self::Songs => "曲",
            Self::Practice => "練習",
            Self::App => "アプリ",
        }
    }
}

/// 選択肢 1 つ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// config.toml に書く名前
    pub key: &'static str,
    /// 画面に出す名前
    pub label: &'static str,
}

/// 項目の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// ON / OFF
    Toggle { default: bool },
    /// 選択肢から 1 つ（Enter で次へ）
    #[expect(dead_code, reason = "次に移す設定（ゲージの減り方など）で使う")]
    Choice {
        choices: &'static [Choice],
        default: usize,
    },
    /// 数（Enter を押してから ↑↓ で `step` ずつ）
    #[expect(dead_code, reason = "次に移す設定（判定の幅など）で使う")]
    Number {
        min: i64,
        max: i64,
        step: i64,
        default: i64,
        unit: &'static str,
    },
    /// ほかの ON / OFF をまとめて切り替える。値は全部 ON なら ON、全部 OFF なら OFF、混ざっていれば「-」
    /// （上流の三つの状態の項目 `CItemThreeState`）
    All { members: &'static [&'static str] },
}

/// 設定の項目 1 つ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Def {
    /// config.toml の `[settings]` のキー
    pub key: &'static str,
    pub group: Group,
    /// 分類の中の小見出し（無ければ空）
    pub section: &'static str,
    /// 画面に出す名前
    pub name: &'static str,
    pub kind: Kind,
    /// 説明のパネルに出す文
    pub desc: &'static str,
    /// 演奏中に変えてよいか
    pub in_play: bool,
}

/// 自動演奏のレーン 1 つ
const fn auto_lane(key: &'static str, name: &'static str) -> Def {
    Def {
        key,
        group: Group::Play,
        section: AUTO_SECTION,
        name,
        kind: Kind::Toggle { default: false },
        desc: AUTO_LANE_DESC,
        in_play: false,
    }
}

const AUTO_SECTION: &str = "自動で叩く";

/// オートのまとめての切り替え
pub const AUTO_ALL: &str = "auto_all";

/// オートの項目のキーと、そのキーで自動で叩くレーン（上流 `bAutoPlay` の LC〜RD。HH は閉と開の両方）。
/// 並びは上流の設定画面と同じ
pub const AUTO_LANES: [(&str, &[DrumLane]); 11] = [
    ("auto_lc", &[DrumLane::LC]),
    ("auto_hh", &[DrumLane::HH, DrumLane::HHO]),
    ("auto_lp", &[DrumLane::LP]),
    ("auto_lbd", &[DrumLane::LBD]),
    ("auto_sd", &[DrumLane::SD]),
    ("auto_bd", &[DrumLane::BD]),
    ("auto_ht", &[DrumLane::HT]),
    ("auto_lt", &[DrumLane::LT]),
    ("auto_ft", &[DrumLane::FT]),
    ("auto_cy", &[DrumLane::CY]),
    ("auto_rd", &[DrumLane::RD]),
];

const AUTO_KEYS: [&str; 11] = {
    let mut keys = [""; 11];
    let mut i = 0;
    while i < AUTO_LANES.len() {
        keys[i] = AUTO_LANES[i].0;
        i += 1;
    }
    keys
};

const AUTO_LANE_DESC: &str = "ON にすると、このレーンの音符を自動で叩きます（音も鳴らします）。ほかのレーンの練習に。\n自動のレーンがあるプレイは、ベストの記録に入れません（ハイハットは閉と開の両方）。";

/// すべての項目（設定の画面の並び）
pub const ALL: &[Def] = &[
    Def {
        key: AUTO_ALL,
        group: Group::Play,
        section: AUTO_SECTION,
        name: "自動演奏（AutoPlay）",
        kind: Kind::All {
            members: &AUTO_KEYS,
        },
        desc: "ドラムのすべてのレーンの自動演奏を、まとめて ON / OFF にします。下の各レーンで 1 つずつも選べます。\n全部を自動にしたプレイは記録に残しません。",
        in_play: false,
    },
    auto_lane("auto_lc", "　左シンバル"),
    auto_lane("auto_hh", "　ハイハット"),
    auto_lane("auto_lp", "　左ペダル"),
    auto_lane("auto_lbd", "　左バスドラム"),
    auto_lane("auto_sd", "　スネア"),
    auto_lane("auto_bd", "　バスドラム"),
    auto_lane("auto_ht", "　ハイタム"),
    auto_lane("auto_lt", "　ロータム"),
    auto_lane("auto_ft", "　フロアタム"),
    auto_lane("auto_cy", "　右シンバル"),
    auto_lane("auto_rd", "　ライド"),
];

/// キーの項目。表に無いキーはプログラムの誤り
pub fn def(key: &str) -> &'static Def {
    ALL.iter()
        .find(|d| d.key == key)
        .unwrap_or_else(|| panic!("no setting named {key}"))
}

/// 分類 `group` の項目（表の並び）
pub fn in_group(group: Group) -> impl Iterator<Item = &'static Def> {
    ALL.iter().filter(move |d| d.group == group)
}

/// 項目の値（config.toml の `[settings]`）。書いていないキーは既定の値
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Values(BTreeMap<String, toml::Value>);

impl Values {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// ON / OFF の値。まとめての項目は、全部 ON なら true
    pub fn flag(&self, key: &str) -> bool {
        match def(key).kind {
            Kind::Toggle { default } => self
                .0
                .get(key)
                .and_then(toml::Value::as_bool)
                .unwrap_or(default),
            Kind::All { members } => members.iter().all(|m| self.flag(m)),
            _ => false,
        }
    }

    /// 数の値（範囲に収め、刻みにそろえる）
    pub fn number(&self, key: &str) -> i64 {
        let Kind::Number {
            min,
            max,
            step,
            default,
            ..
        } = def(key).kind
        else {
            return 0;
        };
        let v = self
            .0
            .get(key)
            .and_then(toml::Value::as_integer)
            .unwrap_or(default);
        snap(v, min, max, step)
    }

    /// 選択肢の何番目か（読めない名前なら既定）
    pub fn choice(&self, key: &str) -> usize {
        let Kind::Choice { choices, default } = def(key).kind else {
            return 0;
        };
        self.0
            .get(key)
            .and_then(toml::Value::as_str)
            .and_then(|s| choices.iter().position(|c| c.key == s))
            .unwrap_or(default)
    }

    /// 画面に出す値
    pub fn label(&self, key: &str) -> String {
        let on_off = |b: bool| (if b { "ON" } else { "OFF" }).to_owned();
        match def(key).kind {
            Kind::Toggle { .. } => on_off(self.flag(key)),
            Kind::Choice { choices, .. } => choices[self.choice(key)].label.to_owned(),
            Kind::Number { unit, .. } => format!("{}{unit}", self.number(key)),
            Kind::All { members } => {
                let on = members.iter().filter(|m| self.flag(m)).count();
                if on == members.len() {
                    on_off(true)
                } else if on == 0 {
                    on_off(false)
                } else {
                    "-".to_owned()
                }
            }
        }
    }

    /// 値を 1 段変える。ON / OFF は切り替え、選択肢は `delta` だけ先へ（端で回る）、数は `delta` 刻み。
    /// まとめての項目は、全部 ON なら全部 OFF に、そうでなければ全部 ON にする
    pub fn change(&mut self, key: &str, delta: i32) {
        match def(key).kind {
            Kind::Toggle { .. } => {
                let v = !self.flag(key);
                self.set(key, toml::Value::Boolean(v));
            }
            Kind::Choice { choices, .. } => {
                let n = choices.len() as i64;
                let i = (self.choice(key) as i64 + i64::from(delta)).rem_euclid(n) as usize;
                self.set(key, toml::Value::String(choices[i].key.to_owned()));
            }
            Kind::Number { min, max, step, .. } => {
                let v = snap(self.number(key) + i64::from(delta) * step, min, max, step);
                self.set(key, toml::Value::Integer(v));
            }
            Kind::All { members } => {
                let v = !self.flag(key);
                for m in members {
                    self.set(m, toml::Value::Boolean(v));
                }
            }
        }
    }

    fn set(&mut self, key: &str, v: toml::Value) {
        self.0.insert(key.to_owned(), v);
    }

    /// 自動で叩くレーン
    pub fn auto_lanes(&self) -> Vec<DrumLane> {
        AUTO_LANES
            .iter()
            .filter(|(key, _)| self.flag(key))
            .flat_map(|(_, lanes)| lanes.iter().copied())
            .collect()
    }
}

/// `v` を `min`〜`max` に収め、`min` からの `step` 刻みにそろえる
fn snap(v: i64, min: i64, max: i64, step: i64) -> i64 {
    let step = step.max(1);
    let v = v.clamp(min, max);
    min + (v - min) / step * step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_members_exist() {
        for (i, d) in ALL.iter().enumerate() {
            assert!(
                ALL[..i].iter().all(|e| e.key != d.key),
                "duplicate key {}",
                d.key
            );
            if let Kind::All { members } = d.kind {
                for m in members {
                    assert!(matches!(def(m).kind, Kind::Toggle { .. }), "{m}");
                }
            }
            if let Kind::Choice { choices, default } = d.kind {
                assert!(default < choices.len(), "{}", d.key);
            }
            if let Kind::Number {
                min, max, default, ..
            } = d.kind
            {
                assert!((min..=max).contains(&default), "{}", d.key);
            }
        }
        for (key, _) in AUTO_LANES {
            def(key);
        }
    }

    #[test]
    fn auto_all_switches_every_lane() {
        let mut v = Values::default();
        assert!(v.auto_lanes().is_empty());
        assert_eq!(v.label(AUTO_ALL), "OFF");
        v.change("auto_bd", 1);
        assert_eq!(v.auto_lanes(), vec![DrumLane::BD]);
        assert_eq!(v.label(AUTO_ALL), "-");
        // 混ざっているときは全部 ON に
        v.change(AUTO_ALL, 1);
        assert_eq!(v.label(AUTO_ALL), "ON");
        assert_eq!(v.auto_lanes().len(), 12);
        v.change(AUTO_ALL, 1);
        assert!(v.auto_lanes().is_empty());
        v.change("auto_hh", 1);
        assert_eq!(v.auto_lanes(), vec![DrumLane::HH, DrumLane::HHO]);
    }

    #[test]
    fn values_round_trip_through_toml() {
        let mut v = Values::default();
        v.change("auto_sd", 1);
        let text = toml::to_string(&v).unwrap();
        assert_eq!(text.trim(), "auto_sd = true");
        let back: Values = toml::from_str(&text).unwrap();
        assert_eq!(back, v);
        // 型の違う値は既定に
        let odd: Values = toml::from_str("auto_sd = 3\n").unwrap();
        assert!(!odd.flag("auto_sd"));
    }

    #[test]
    fn numbers_snap_to_their_steps() {
        assert_eq!(snap(17, 0, 100, 5), 15);
        assert_eq!(snap(-3, 0, 100, 5), 0);
        assert_eq!(snap(130, 0, 100, 5), 100);
        assert_eq!(snap(7, 1, 9, 2), 7);
    }
}
