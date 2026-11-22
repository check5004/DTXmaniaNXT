//! testdata/charts の譜面を読み、手で計算した正解と比べる。

use std::collections::BTreeMap;
use std::path::PathBuf;

use dtx::AutoChannel;
use serde::Deserialize;

#[derive(Deserialize)]
struct Expected {
    title: String,
    level: Option<f64>,
    notes: Vec<ExpectedNote>,
    /// 自動で鳴らす音（省略できる）
    auto: Option<Vec<ExpectedAuto>>,
    /// `#WAVzz` の番号ごとの音量と左右（省略できる）
    volume: Option<BTreeMap<String, u8>>,
    pan: Option<BTreeMap<String, i8>>,
    /// 空打ちの音と見えない音符（省略できる）
    empty_hits: Option<Vec<ExpectedLaneSound>>,
    hidden: Option<Vec<ExpectedLaneSound>>,
}

#[derive(Deserialize)]
struct ExpectedAuto {
    ms: f64,
    wav: String,
    kind: String,
    cut_ms: Option<f64>,
}

#[derive(Deserialize)]
struct ExpectedLaneSound {
    ms: f64,
    lane: String,
    wav: String,
}

fn wav(id: &str) -> u16 {
    dtx::parse_base36x2(id).unwrap()
}

fn kind(c: AutoChannel) -> String {
    match c {
        AutoChannel::Bgm => "BGM".to_owned(),
        AutoChannel::Se(ch) => format!("SE{ch:02X}"),
        AutoChannel::Drum(ch) => format!("DRUM{ch:02X}"),
        AutoChannel::Guitar => "GUITAR".to_owned(),
        AutoChannel::Bass => "BASS".to_owned(),
    }
}

#[derive(Deserialize)]
struct ExpectedNote {
    ms: f64,
    lane: String,
    /// 小節の番号（省略できる）
    measure: Option<u32>,
    /// 小節の中の位置。約分した分数 `"1/4"`、頭は `"0"`（省略できる）
    pos: Option<String>,
}

/// `"3/8"` や `"0"` を (分子, 分母) にする
fn fraction(s: &str) -> (u32, u32) {
    match s.split_once('/') {
        Some((n, d)) => (n.trim().parse().unwrap(), d.trim().parse().unwrap()),
        None => (s.trim().parse().unwrap(), 1),
    }
}

fn charts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/charts")
}

#[test]
fn shared_test_charts() {
    let mut checked = 0;
    for entry in std::fs::read_dir(charts_dir()).expect("testdata/charts") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("dtx") {
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let expected: Expected =
            serde_json::from_slice(&std::fs::read(path.with_extension("expected.json")).unwrap())
                .unwrap();

        let chart = dtx::load(&std::fs::read(&path).unwrap());
        assert!(chart.warnings.is_empty(), "{name}: {:?}", chart.warnings);
        assert_eq!(chart.title(), expected.title, "{name}: title");
        if let Some(level) = expected.level {
            assert!(
                (chart.drum_level().unwrap() - level).abs() < 1e-9,
                "{name}: level"
            );
        }

        let notes = dtx::TempoMap::build(&chart).drum_notes(&chart);
        let got: Vec<(f64, &str)> = notes.iter().map(|n| (n.ms, n.lane.name())).collect();
        let want: Vec<(f64, &str)> = expected
            .notes
            .iter()
            .map(|n| (n.ms, n.lane.as_str()))
            .collect();
        assert_eq!(
            got.len(),
            want.len(),
            "{name}: note count\n got: {got:?}\nwant: {want:?}"
        );
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!(
                (g.0 - w.0).abs() < 0.01 && g.1 == w.1,
                "{name}: note {i}: got {g:?}, want {w:?}"
            );
        }
        for (i, (n, w)) in notes.iter().zip(&expected.notes).enumerate() {
            if let Some(m) = w.measure {
                assert_eq!(n.at.measure, m, "{name}: note {i}: measure");
            }
            if let Some(pos) = &w.pos {
                assert_eq!((n.at.num, n.at.den), fraction(pos), "{name}: note {i}: pos");
            }
        }
        let tempo = dtx::TempoMap::build(&chart);
        if let Some(want) = &expected.auto {
            let got: Vec<(f64, u16, String, Option<f64>)> = tempo
                .auto_sounds(&chart)
                .iter()
                .map(|a| (a.ms, a.wav, kind(a.channel), a.cut_ms))
                .collect();
            let want: Vec<(f64, u16, String, Option<f64>)> = want
                .iter()
                .map(|a| (a.ms, wav(&a.wav), a.kind.clone(), a.cut_ms))
                .collect();
            assert_eq!(got, want, "{name}: auto sounds");
        }
        for (id, v) in expected.volume.iter().flatten() {
            assert_eq!(chart.wav_volume(wav(id)), *v, "{name}: volume {id}");
        }
        for (id, v) in expected.pan.iter().flatten() {
            assert_eq!(chart.wav_pan(wav(id)), *v, "{name}: pan {id}");
        }
        for (got, want, what) in [
            (
                tempo.empty_hit_sounds(&chart),
                &expected.empty_hits,
                "empty hits",
            ),
            (tempo.hidden_sounds(&chart), &expected.hidden, "hidden"),
        ] {
            let Some(want) = want else { continue };
            let got: Vec<(f64, &str, u16)> =
                got.iter().map(|s| (s.ms, s.lane.name(), s.wav)).collect();
            let want: Vec<(f64, &str, u16)> = want
                .iter()
                .map(|s| (s.ms, s.lane.as_str(), wav(&s.wav)))
                .collect();
            assert_eq!(got, want, "{name}: {what}");
        }
        checked += 1;
    }
    assert!(checked >= 4, "only {checked} charts found");
}
