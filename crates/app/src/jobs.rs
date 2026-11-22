//! 別のスレッドでする仕事の列（曲名の文字を画像にする、ジャケットを読む、譜面の混み具合を数える）。
//!
//! 後から頼んだものから先に片付け、しばらく頼み直されなかった仕事は始める前に捨てる。
//! 選曲画面を速く送ったとき、通り過ぎた曲の仕事に手間取って、今見えている曲が後回しにならないように。
//! 呼ぶ側は要るあいだ毎フレーム頼み直す（頼み直すと一番先になる）。

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

struct State<K> {
    /// まだ始めていない仕事（最後が一番先）
    queue: Vec<K>,
    closed: bool,
}

struct Shared<K> {
    state: Mutex<State<K>>,
    wake: Condvar,
}

impl<K> Shared<K> {
    fn lock(&self) -> MutexGuard<'_, State<K>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 仕事の列。`K` は仕事（同じものは 1 度だけする）、`R` は結果
pub struct Jobs<K, R> {
    shared: Arc<Shared<K>>,
    done: Receiver<(K, R)>,
    /// 頼んでまだ結果の届いていない仕事と、最後に頼まれた時（呼ぶ側の数え方。フレームの番号など）
    pending: HashMap<K, u64>,
}

impl<K, R> Jobs<K, R>
where
    K: Hash + Eq + Clone + Send + 'static,
    R: Send + 'static,
{
    /// `threads` 本のスレッドで `work` をする
    pub fn new(name: &str, threads: usize, work: impl Fn(&K) -> R + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                queue: Vec::new(),
                closed: false,
            }),
            wake: Condvar::new(),
        });
        let (tx, done) = channel();
        let work = Arc::new(work);
        for i in 0..threads.max(1) {
            let shared = shared.clone();
            let tx = tx.clone();
            let work = work.clone();
            std::thread::Builder::new()
                .name(format!("{name} {i}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let mut st = shared.lock();
                            loop {
                                if st.closed {
                                    return;
                                }
                                if let Some(j) = st.queue.pop() {
                                    break j;
                                }
                                st = shared.wake.wait(st).unwrap_or_else(PoisonError::into_inner);
                            }
                        };
                        let r = work(&job);
                        if tx.send((job, r)).is_err() {
                            return;
                        }
                    }
                })
                .expect("cannot start a worker thread");
        }
        Self {
            shared,
            done,
            pending: HashMap::new(),
        }
    }

    /// 頼む。もう頼んであれば、まだ要ることを覚えて一番先にする。`now` は頼んだ時
    pub fn request(&mut self, key: &K, now: u64) {
        let mut st = self.shared.lock();
        if let Some(t) = self.pending.get_mut(key) {
            *t = now;
            // 始める前なら一番先へ（始めていれば何もしない）
            if let Some(i) = st.queue.iter().position(|j| j == key)
                && i + 1 != st.queue.len()
            {
                let j = st.queue.remove(i);
                st.queue.push(j);
            }
            return;
        }
        self.pending.insert(key.clone(), now);
        st.queue.push(key.clone());
        drop(st);
        self.shared.wake.notify_one();
    }

    /// 頼んでいて、まだ結果が届いていない
    #[cfg(test)]
    pub fn is_pending(&self, key: &K) -> bool {
        self.pending.contains_key(key)
    }

    /// 届いた結果を受け取る。`stale` より前から頼み直されていない、まだ始めていない仕事は捨てる
    pub fn poll(&mut self, stale: u64) -> Vec<(K, R)> {
        let mut out = Vec::new();
        for (k, r) in self.done.try_iter() {
            self.pending.remove(&k);
            out.push((k, r));
        }
        let mut st = self.shared.lock();
        let pending = &mut self.pending;
        st.queue.retain(|k| {
            let keep = pending.get(k).is_some_and(|&t| t >= stale);
            if !keep {
                pending.remove(k);
            }
            keep
        });
        out
    }
}

impl<K, R> Drop for Jobs<K, R> {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Sender;
    use std::time::{Duration, Instant};

    /// 結果が `n` 個届くまで待つ
    fn wait_for<K, R>(jobs: &mut Jobs<K, R>, n: usize, stale: u64) -> Vec<(K, R)>
    where
        K: Hash + Eq + Clone + Send + 'static,
        R: Send + 'static,
    {
        let start = Instant::now();
        let mut out = Vec::new();
        while out.len() < n {
            out.extend(jobs.poll(stale));
            assert!(start.elapsed() < Duration::from_secs(5), "timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        out
    }

    /// 最初の仕事で止めておける仕事の列（止めているあいだに頼んだ順を確かめる）
    fn gated() -> (Jobs<u32, u32>, Sender<()>) {
        let (open, gate) = channel::<()>();
        let gate = Mutex::new(gate);
        let jobs = Jobs::new("test", 1, move |&k: &u32| {
            if k == 0 {
                let _ = gate.lock().unwrap().recv();
            }
            k * 10
        });
        (jobs, open)
    }

    #[test]
    fn does_each_job_once() {
        let mut jobs = Jobs::new("test", 2, |&k: &u32| k * 2);
        for k in 0..20 {
            jobs.request(&k, 0);
            jobs.request(&k, 0);
        }
        let mut got = wait_for(&mut jobs, 20, 0);
        got.sort_unstable();
        assert_eq!(got, (0..20).map(|k| (k, k * 2)).collect::<Vec<_>>());
        assert!(!jobs.is_pending(&3));
    }

    #[test]
    fn latest_request_goes_first() {
        let (mut jobs, open) = gated();
        jobs.request(&0, 0);
        // 0 を始めるまで待つ
        while !jobs.shared.lock().queue.is_empty() {
            std::thread::sleep(Duration::from_millis(1));
        }
        for k in 1..=3 {
            jobs.request(&k, 0);
        }
        // 頼み直した 1 が一番先、次に後から頼んだ 3
        jobs.request(&1, 0);
        open.send(()).unwrap();
        let got: Vec<u32> = wait_for(&mut jobs, 4, 0)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(got, [0, 1, 3, 2]);
    }

    #[test]
    fn stale_jobs_are_dropped() {
        let (mut jobs, open) = gated();
        jobs.request(&0, 0);
        while !jobs.shared.lock().queue.is_empty() {
            std::thread::sleep(Duration::from_millis(1));
        }
        jobs.request(&1, 1);
        jobs.request(&2, 5);
        // 1 は 3 より前から頼まれていないので捨てる（始めている 0 は捨てない）
        assert!(jobs.poll(3).is_empty());
        assert!(!jobs.is_pending(&1));
        assert!(jobs.is_pending(&0));
        open.send(()).unwrap();
        let got: Vec<u32> = wait_for(&mut jobs, 2, 3)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(got, [0, 2]);
        // 捨てたものは頼み直せばする
        jobs.request(&1, 9);
        assert_eq!(wait_for(&mut jobs, 1, 9), [(1, 10)]);
    }
}
