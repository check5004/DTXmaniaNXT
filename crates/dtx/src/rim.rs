//! スネアのリムの音符の見分け。
//!
//! DTX にはリムのレーンが無い（上流 DTXManiaNX にも仕組みは無い）。譜面の作者はスネアのレーンに
//! リムショットやクロススティックの音（`Rim_37_92.ogg`、`SideStick_37_92.ogg`、`037_Rimshot.ogg ;Snare Rim` など）を置いて表す。
//! そこで、音のファイル名と `#WAVzz` の行の注釈（DTXCreator が書く音の名前）から見当をつける。
//! 名前からはリムショットとクロススティックを分けられないことが多い（GM の 37 番はサイドスティックなのに
//! `037_Rimshot.ogg` と名付けたものや、`037_Rimshot.ogg ;CROSS STICK` もある）ので、どちらも「リム」とまとめる。

use std::collections::HashSet;

use crate::lane::DrumLane;
use crate::parse::Chart;
use crate::timing::DrumNote;

/// 音の名前がリムの音らしいか。大文字と小文字、全角と半角のカタカナを問わない。
/// `rim`（`trim`・`prim`・`grim` の一部は除く）、`stick`（サイドスティック・クロススティック・`Xstick`）、
/// カタカナのリム・スティックを探す
pub fn is_rim_sound_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    let rim = lower.match_indices("rim").any(|(i, _)| {
        !matches!(
            lower[..i].chars().next_back(),
            Some('t' | 'p' | 'g' | 'b' | 'c')
        )
    });
    rim || lower.contains("stick")
        || ["リム", "ﾘﾑ", "スティック", "ｽﾃｨｯｸ"]
            .iter()
            .any(|k| name.contains(k))
}

impl Chart {
    /// `#WAVzz` がリムの音らしいか（ファイル名か注釈で見分ける。[`is_rim_sound_name`]）
    pub fn is_rim_wav(&self, id: u16) -> bool {
        self.wav(id).is_some_and(is_rim_sound_name)
            || self.wav_comment(id).is_some_and(is_rim_sound_name)
    }

    /// スネアの音符に使われている音のうち、リムの音らしいものの番号
    pub fn rim_wavs(&self, notes: &[DrumNote]) -> HashSet<u16> {
        notes
            .iter()
            .filter(|n| n.lane == DrumLane::SD)
            .map(|n| n.wav)
            .filter(|&w| self.is_rim_wav(w))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rim_names() {
        // ユーザーの曲にあった名前
        for name in [
            "Rim_37_92.ogg",
            "SideStick_37_92.ogg",
            "037_Rimshot.ogg",
            "RimShot.ogg",
            "038_Snare_rimclick.ogg",
            "Stick.ogg",
            "Snare_Xstick2.wav",
            "YUMEYUME-SIDESTICK.XA",
            "rim_cut.wav",
            "Snare Rim",
            "CROSS STICK",
            "スネア リム",
            "ｻｲﾄﾞｽﾃｨｯｸ",
            "SnareRim.wav",
        ] {
            assert!(is_rim_sound_name(name), "{name}");
        }
        for name in [
            "Snare_38_92.ogg",
            "038_snare_lim_mn.ogg",
            "040_sn_rs_.ogg",
            "snare_trim.wav",
            "prime.ogg",
            "grim.wav",
            "brim.wav",
            "crimson.ogg",
            "HNG-SNARE3.ogg",
            "スネア",
            "",
        ] {
            assert!(!is_rim_sound_name(name), "{name}");
        }
    }

    #[test]
    fn rim_wavs_of_snare_notes() {
        let chart = crate::load(
            "#BPM 120\n#WAV01 snare.ogg\n#WAV02 rim.ogg\n#WAV03 snare2.ogg ;Snare Rim\n#WAV04 stick.ogg\n\
             #00112: 01020300\n#00113: 04\n"
                .as_bytes(),
        );
        let notes = crate::TempoMap::build(&chart).drum_notes(&chart);
        let rims = chart.rim_wavs(&notes);
        // 04 はリムの名前でもバスドラムの音符なので入れない
        assert_eq!(rims, HashSet::from([2, 3]));
    }
}
