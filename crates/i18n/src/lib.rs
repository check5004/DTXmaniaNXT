//! 画面に出す文の言語。英語（既定）と日本語の 2 つ。
//!
//! 文は使う所に英語と日本語を並べて書く（英語が先）。
//!
//! - [`t`]`("Back", "戻る")` … 今の言語の方（`&'static str`）
//! - [`tf!`]`("{n} bars", "{n} 小節")` … `format!` と同じ書き方で、今の言語の方（`String`）
//! - 表（設定の項目など）に置くときは [`tr`]`("Back", "戻る")`（`const fn`、[`Tr`]）。描くときに [`Tr::get`]
//!
//! 英語は日本語の直訳にしない。英語のゲームで普通に使う短い言い方にする。項目名・ボタン・見出しは
//! Title Case（"Scroll Speed"）、説明や知らせの文は普通の文。判定（PERFECT など）や難易度、
//! 遊び方の用語（HAZARD、Risky など）は両方で同じ綴り。
//!
//! 言語は [`set_lang`] で全体を変える（設定の「言語」）。テストは [`scoped`] でそのスレッドだけ変える。

use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};

/// 画面の言語
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Lang {
    #[default]
    En,
    Ja,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ja];

    /// 設定のファイルに書く名前
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ja => "ja",
        }
    }

    /// [`Lang::code`] から（読めなければ `None`）
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.code() == code)
    }

    /// その言語での言語の名前（どちらの言語の画面でも同じ綴りで出す）
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ja => "日本語",
        }
    }

    fn from_u8(v: u8) -> Self {
        if v == 1 { Lang::Ja } else { Lang::En }
    }

    fn to_u8(self) -> u8 {
        match self {
            Lang::En => 0,
            Lang::Ja => 1,
        }
    }
}

static GLOBAL: AtomicU8 = AtomicU8::new(0);

thread_local! {
    /// テストでそのスレッドだけ変えた言語（[`scoped`]）
    static LOCAL: Cell<Option<Lang>> = const { Cell::new(None) };
}

/// 今の言語
pub fn lang() -> Lang {
    LOCAL
        .with(Cell::get)
        .unwrap_or_else(|| Lang::from_u8(GLOBAL.load(Ordering::Relaxed)))
}

/// 言語を変える（どのスレッドにも効く）
pub fn set_lang(lang: Lang) {
    GLOBAL.store(lang.to_u8(), Ordering::Relaxed);
}

/// 今の言語が日本語か
pub fn is_ja() -> bool {
    lang() == Lang::Ja
}

/// このスレッドだけ言語を変え、戻り値を捨てると戻す（テスト用。テストは並んで走るので [`set_lang`] は使わない）
#[must_use = "捨てるとすぐ元の言語に戻る"]
pub fn scoped(lang: Lang) -> Scoped {
    Scoped(LOCAL.with(|l| l.replace(Some(lang))))
}

/// [`scoped`] の戻り値
pub struct Scoped(Option<Lang>);

impl Drop for Scoped {
    fn drop(&mut self) {
        LOCAL.with(|l| l.set(self.0));
    }
}

/// 英語と日本語の組。表に置く文（設定の項目の名前・説明・選択肢など）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tr {
    pub en: &'static str,
    pub ja: &'static str,
}

impl Tr {
    /// 今の言語の方
    pub fn get(self) -> &'static str {
        self.of(lang())
    }

    /// その言語の方
    pub fn of(self, lang: Lang) -> &'static str {
        match lang {
            Lang::En => self.en,
            Lang::Ja => self.ja,
        }
    }

    /// 両方とも空
    pub const fn is_empty(self) -> bool {
        self.en.is_empty() && self.ja.is_empty()
    }
}

/// 英語と日本語の組を作る（表に置く文）
pub const fn tr(en: &'static str, ja: &'static str) -> Tr {
    Tr { en, ja }
}

/// 両方とも空の組（見出しの無い小分けなど）
pub const EMPTY: Tr = tr("", "");

/// 今の言語の方を返す
pub fn t(en: &'static str, ja: &'static str) -> &'static str {
    tr(en, ja).get()
}

/// 数と単位。英語は 1 のとき単数にする。`count(3, "loop", "loops", "周")` は "3 loops" / "3 周"
pub fn count<N>(n: N, one: &str, many: &str, ja: &str) -> String
where
    N: std::fmt::Display + Copy + PartialEq + From<u8>,
{
    match lang() {
        Lang::En if n == N::from(1) => format!("{n} {one}"),
        Lang::En => format!("{n} {many}"),
        Lang::Ja => format!("{n} {ja}"),
    }
}

/// `format!` の形で今の言語の方を作る。`tf!("{n} bars", "{n} 小節")`、`tf!("{} / {}", "{} / {}", a, b)`。
/// 引数は両方の文で同じものを使う
#[macro_export]
macro_rules! tf {
    ($en:literal, $ja:literal $(, $($arg:tt)*)?) => {
        if $crate::is_ja() {
            format!($ja $(, $($arg)*)?)
        } else {
            format!($en $(, $($arg)*)?)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_unless_set() {
        let _en = scoped(Lang::En);
        assert_eq!(t("Back", "戻る"), "Back");
        assert_eq!(tr("Back", "戻る").get(), "Back");
        let n = 3;
        assert_eq!(tf!("{n} bars", "{n} 小節"), "3 bars");
        assert_eq!(count(1usize, "loop", "loops", "周"), "1 loop");
        assert_eq!(count(4u32, "loop", "loops", "周"), "4 loops");
        let _ja = scoped(Lang::Ja);
        assert_eq!(count(1i64, "loop", "loops", "周"), "1 周");
    }

    #[test]
    fn scoped_switches_only_this_thread_and_restores() {
        {
            let _ja = scoped(Lang::Ja);
            assert_eq!(t("Back", "戻る"), "戻る");
            assert_eq!(tf!("{} of {}", "{} / {}", 1, 2), "1 / 2");
            let other = std::thread::spawn(lang).join().unwrap();
            assert_eq!(other, Lang::from_u8(GLOBAL.load(Ordering::Relaxed)));
        }
        let _en = scoped(Lang::En);
        assert_eq!(lang(), Lang::En);
    }

    #[test]
    fn codes_round_trip() {
        for l in Lang::ALL {
            assert_eq!(Lang::from_code(l.code()), Some(l));
            assert_eq!(Lang::from_u8(l.to_u8()), l);
        }
        assert_eq!(Lang::from_code("fr"), None);
    }
}
