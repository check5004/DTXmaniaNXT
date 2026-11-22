//! 設定の項目が NX v2.0（BocuD/DTXmaniaNX 20260121）の設定画面のどこにあったか。
//! 設定の画面の右の列に「NX v2.0 では: Drums ＞ HH Group」と出して、NX から来た人が探せるようにする。
//! 名前は NX の画面の英語のまま（System の中は Graphics・Audio・Gameplay の小分け）。
//!
//! ここに無い項目は何も出さない。NX に無い項目は [`NEW`] を返す。
//! 設定の表（[`crate::settings`]）に項目を足したら、ここにも足す。

use crate::config_view::{Drill, ItemId, Setting};

/// NX に無い項目
pub const NEW: &str = "無し（NXT で足した項目）";
/// Config.ini にはあるが、NX の設定の画面には無い項目
const HIT_RANGE: &str = "画面には無し（Config.ini の [HitRange]）";
const DTX_PATH: &str = "画面には無し（Config.ini の DTXPath）";
const INPUT_MAPPING: &str = "Drums ＞ Input Mapping";

/// 項目の、NX v2.0 での場所
pub fn of(id: ItemId) -> Option<&'static str> {
    Some(match id {
        ItemId::Value(s) => match s {
            Setting::MasterVolume => "System ＞ Audio ＞ MasterVolume",
            Setting::InputAdjust => "Drums ＞ InputAdjust",
            Setting::ScrollSpeed => "Drums ＞ ScrollSpeed",
            Setting::StageFailed => "System ＞ Gameplay ＞ StageFailed",
            Setting::HitSound => "Drums ＞ HitSound",
            Setting::ShowLagTime => "System ＞ Gameplay ＞ ShowLagTime",
            Setting::ShowLagTimeColor => "System ＞ Gameplay ＞ ShowLagTimeColor",
            Setting::ShowLagHitCount => "System ＞ Gameplay ＞ ShowLagHitCount",
            // 表示の補正、更新
            _ => NEW,
        },
        ItemId::Table(key) => return of_key(key),
        ItemId::Practice(d) => match d {
            Drill::Speed => "近いのは System ＞ Gameplay ＞ PlaySpeed",
            Drill::Pitch => "近いのは System ＞ Audio ＞ TimeStretch",
            _ => NEW,
        },
        ItemId::Lanes | ItemId::AssignAll | ItemId::Lane(_) | ItemId::ResetPads => INPUT_MAPPING,
        ItemId::DeviceMinVelocity(_) => "近いのは Drums ＞ Drum Hit Velocity（レーンごと）",
        ItemId::Device(_)
        | ItemId::DeviceState(_)
        | ItemId::DeviceUse(_)
        | ItemId::DevicePreset(_)
        | ItemId::DeviceChannel(_)
        | ItemId::DeviceReset(_)
        | ItemId::ReconnectMidi
        | ItemId::SongOffset
        | ItemId::ApplyAdvice => NEW,
        ItemId::Rescan => "System ＞ Reload Songs",
        ItemId::AddSongDir | ItemId::SongDir(_) => DTX_PATH,
        // 簡易設定・一時停止の操作の行（ほかのスレッドが足した行も、ここに足すまでは何も出さない）
        _ => return None,
    })
}

/// 設定の表（[`crate::settings`]）のキーの、NX v2.0 での場所
fn of_key(key: &str) -> Option<&'static str> {
    Some(match key {
        // System ＞ Graphics
        "display_mode" => "System ＞ Graphics ＞ Fullscreen",
        "vsync" => "System ＞ Graphics ＞ VSyncWait",
        "avi" => "System ＞ Graphics ＞ AVI",
        "movie_alpha" => "System ＞ Graphics ＞ LaneAlpha",
        // System ＞ Audio
        "chip_volume" => "System ＞ Audio ＞ ChipVolume",
        "auto_volume" => "System ＞ Audio ＞ AutoVolume",
        "bgm_sound" => "System ＞ Audio ＞ BGM Sound",
        "bgm_adjust" => "System ＞ Audio ＞ BGMAdjust",
        // System ＞ Gameplay
        "damage_level" => "System ＞ Gameplay ＞ DamageLevel",
        "risky" => "System ＞ Gameplay ＞ Risky",
        "metronome" => "System ＞ Gameplay ＞ Metronome",
        "auto_save_result" => "System ＞ Gameplay ＞ AutoSaveResult",
        // Drums
        "auto_all" => "Drums ＞ AutoPlay (All)",
        "auto_lc" => "Drums ＞ LeftCymbal",
        "auto_hh" => "Drums ＞ HiHat",
        "auto_lp" => "Drums ＞ LeftPedal",
        "auto_lbd" => "Drums ＞ LBassDrum",
        "auto_sd" => "Drums ＞ Snare",
        "auto_bd" => "Drums ＞ BassDrum",
        "auto_ht" => "Drums ＞ HighTom",
        "auto_lt" => "Drums ＞ LowTom",
        "auto_ft" => "Drums ＞ FloorTom",
        "auto_cy" => "Drums ＞ Cymbal",
        "auto_rd" => "Drums ＞ Ride",
        "hid_sud" => "Drums ＞ HID-SUD",
        "dark" => "Drums ＞ Dark",
        "lane_disp" => "Drums ＞ LaneDisp",
        "judge_line_disp" => "Drums ＞ JudgeLineDisp",
        "lane_flush" => "Drums ＞ LaneFlush",
        "attack_effect" => "Drums ＞ AttackEffect",
        "reverse" => "Drums ＞ Reverse",
        "judge_position" => "Drums ＞ JudgePosition",
        "combo_disp" => "Drums ＞ Combo",
        "lane_type" => "Drums ＞ LaneType",
        "rd_position" => "Drums ＞ RDPosition",
        "hazard" => "Drums ＞ HAZARD",
        "tight" => "Drums ＞ Tight",
        "hh_group" => "Drums ＞ HH Group",
        "ft_group" => "Drums ＞ FT Group",
        "cy_group" => "Drums ＞ CY Group",
        "bd_group" => "Drums ＞ BD Group",
        "cymbal_free" => "Drums ＞ CymbalFree",
        "hh_priority" => "Drums ＞ HH Priority",
        "ft_priority" => "Drums ＞ FT Priority",
        "cy_priority" => "Drums ＞ CY Priority",
        "min_combo" => "Drums ＞ D-MinCombo",
        "hho_graphics" => "Drums ＞ HHOGraphics",
        "lbd_graphics" => "Drums ＞ LBDGraphics",
        "judge_line_pos" => "Drums ＞ JudgeLinePos",
        "shutter_in" => "Drums ＞ ShutterInPos",
        "shutter_out" => "Drums ＞ ShutterOutPos",
        "muting_lp" => "Drums ＞ Muting LP",
        // 画面には無い
        "skip_ms" => "画面には無し（Config.ini の SkipTimeMs）",
        "hit_perfect" | "hit_great" | "hit_good" | "hit_poor" | "pedal_hit_perfect"
        | "pedal_hit_great" | "pedal_hit_good" | "pedal_hit_poor" => HIT_RANGE,
        // NXT で足した
        // （metronome_volume と se_volume は音量ミキサーのスレッドが足す予定のキー）
        "hit_error_bar" | "rim_graphics" | "metronome_volume" | "se_volume" => NEW,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drum_and_system_items_point_where_nx_had_them() {
        assert_eq!(
            of(ItemId::Table("hh_priority")),
            Some("Drums ＞ HH Priority")
        );
        assert_eq!(
            of(ItemId::Table("display_mode")),
            Some("System ＞ Graphics ＞ Fullscreen")
        );
        assert_eq!(of(ItemId::Value(Setting::Update)), Some(NEW));
        assert_eq!(of(ItemId::Table("hit_good")), Some(HIT_RANGE));
        assert_eq!(of(ItemId::Table("no_such_key")), None);
        assert_eq!(of(ItemId::Resume), None);
    }
}
