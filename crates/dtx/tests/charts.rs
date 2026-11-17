//! testdata/charts の譜面を読み、手で計算した正解と比べる。

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize)]
struct Expected {
    title: String,
    level: Option<f64>,
    notes: Vec<ExpectedNote>,
}

#[derive(Deserialize)]
struct ExpectedNote {
    ms: f64,
    lane: String,
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
        checked += 1;
    }
    assert!(checked >= 4, "only {checked} charts found");
}
