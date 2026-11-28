//! 場面が変わった直後の入力を受け付けない門。
//!
//! 電子ドラムで遊ぶ人は、結果の画面などで何気なく叩き続けることが多い。前の画面で叩いた続きが
//! 次の画面に効くと、もう一度や練習が勝手に始まってしまう。そこで場面（画面と、重ねている練習メニュー・板）が
//! 変わったら少しのあいだパッドを受け付けず、そのあいだに叩いたら、叩くのを止めるまで延ばす。
//! Esc も同じにして、連打しても 1 画面ずつしか戻らない（選曲やタイトルの先、アプリの終了まで届かない）。

use std::time::{Duration, Instant};

/// 場面が変わってから、パッドと Esc を受け付けない時間（結果の画面のパッドは `RESULT_SETTLE`）
pub const SCENE_SETTLE: Duration = Duration::from_millis(500);
pub const RESULT_SETTLE: Duration = Duration::from_millis(1000);
/// 受け付けないあいだに叩いたら、そこからこれだけ叩くのを止めるまで受け付けない
pub const KEEP_QUIET: Duration = Duration::from_millis(400);

/// 今の場面。`screen` は画面の種類、`menu` は選曲に重ねる練習メニュー、`quick` は簡易設定・一時停止の板
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scene<K> {
    pub screen: K,
    pub menu: bool,
    pub quick: bool,
    /// 結果の画面（パッドを受け付けない時間を長くする）
    pub result: bool,
}

#[derive(Debug)]
pub struct InputGate<K> {
    scene: Option<Scene<K>>,
    pads_from: Instant,
    esc_from: Instant,
}

impl<K: Copy + PartialEq> InputGate<K> {
    pub fn new(now: Instant) -> Self {
        Self {
            scene: None,
            pads_from: now,
            esc_from: now,
        }
    }

    /// 今の場面を見せる。変わっていれば、少しのあいだ受け付けない。
    /// 板（一時停止・簡易設定）を開け閉めしただけなら Esc は止めない（Esc を続けて 2 回で演奏をやめられるように）
    pub fn observe(&mut self, scene: Scene<K>, now: Instant) {
        let Some(before) = self.scene.replace(scene) else {
            return;
        };
        if before == scene {
            return;
        }
        let settle = if scene.result {
            RESULT_SETTLE
        } else {
            SCENE_SETTLE
        };
        self.pads_from = self.pads_from.max(now + settle);
        if before.screen != scene.screen || before.menu != scene.menu {
            self.esc_from = self.esc_from.max(now + SCENE_SETTLE);
        }
    }

    /// パッドを受け付けてよいか。まだなら、そこから [`KEEP_QUIET`] 延ばす
    pub fn pad(&mut self, now: Instant) -> bool {
        pass(&mut self.pads_from, now)
    }

    /// Esc を受け付けてよいか。まだなら、そこから [`KEEP_QUIET`] 延ばす
    pub fn esc(&mut self, now: Instant) -> bool {
        pass(&mut self.esc_from, now)
    }
}

fn pass(from: &mut Instant, now: Instant) -> bool {
    if now < *from {
        *from = (*from).max(now + KEEP_QUIET);
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(screen: u8) -> Scene<u8> {
        Scene {
            screen,
            menu: false,
            quick: false,
            result: false,
        }
    }

    fn ms(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn hits_right_after_a_change_do_nothing_until_you_stop() {
        let t0 = Instant::now();
        let mut g = InputGate::new(t0);
        g.observe(scene(0), t0);
        assert!(g.pad(t0));
        g.observe(scene(1), ms(t0, 1000));
        // 叩き続けている（250ms ごと）あいだは受け付けない
        for i in 0..8 {
            assert!(!g.pad(ms(t0, 1100 + i * 250)), "hit {i}");
        }
        // 最後に叩いてから 400ms 止めると受け付ける
        assert!(!g.pad(ms(t0, 2850 + 350)));
        assert!(g.pad(ms(t0, 3200 + 400)));
        // 叩かずに待っていれば 0.5 秒で受け付ける
        g.observe(scene(2), ms(t0, 5000));
        assert!(g.pad(ms(t0, 5500)));
    }

    #[test]
    fn the_result_screen_waits_longer() {
        let t0 = Instant::now();
        let mut g = InputGate::new(t0);
        g.observe(scene(0), t0);
        g.observe(
            Scene {
                result: true,
                ..scene(1)
            },
            t0,
        );
        // 0.5 秒では受け付けない（ここで叩いたので、さらに 400ms 止めるまで）
        assert!(!g.pad(ms(t0, 600)));
        assert!(!g.pad(ms(t0, 900)));
        assert!(g.pad(ms(t0, 1300)));
    }

    #[test]
    fn esc_twice_quits_from_the_pause_but_mashing_stops_at_the_next_screen() {
        let t0 = Instant::now();
        let mut g = InputGate::new(t0);
        let play = scene(0);
        g.observe(play, t0);
        // 1 回目の Esc で一時停止の板が開く。板を開いただけでは Esc は止めない
        assert!(g.esc(ms(t0, 3000)));
        g.observe(
            Scene {
                quick: true,
                ..play
            },
            ms(t0, 3000),
        );
        assert!(g.esc(ms(t0, 3120)));
        // 2 回目でやめて選曲へ。続けて押した Esc は、止めるまで受け付けない
        g.observe(scene(1), ms(t0, 3500));
        for i in 0..10 {
            assert!(!g.esc(ms(t0, 3550 + i * 120)), "press {i}");
        }
        assert!(g.esc(ms(t0, 3550 + 9 * 120 + 400)));
    }

    #[test]
    fn opening_a_panel_holds_the_pads_but_not_esc() {
        let t0 = Instant::now();
        let mut g = InputGate::new(t0);
        let select = scene(1);
        g.observe(select, t0);
        // RD で簡易設定を開いた直後の RD・CY は効かない（ライドを刻んでも区間練習へ進まない）
        g.observe(
            Scene {
                quick: true,
                ..select
            },
            ms(t0, 1000),
        );
        assert!(!g.pad(ms(t0, 1250)));
        assert!(g.esc(ms(t0, 1250)));
    }
}
