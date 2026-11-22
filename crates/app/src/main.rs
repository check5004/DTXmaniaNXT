//! DTXmaniaNXT 本体。
//!
//! - 曲のフォルダを探して選曲し、BGM を鳴らし、落ちてくる音符を叩いて判定する
//! - 曲の区間（小節の範囲）を選んで繰り返し練習できる。遊んだ記録から苦手な小節を出す
//! - 判定は MIDI の時刻と音の再生位置で行い、画面のフレームには左右されない
//! - 電子ドラムが無いときはキーボードで叩ける
//! - 設定（曲のフォルダ・補正値など）は config.toml に残す
//! - 画面は DTXManiaNX と同じ画像と配置で描く。画像と音はアプリに入れた NX のスキン（`assets/skin`）
//! - 起動するとタイトル。そこから選曲と設定へ進む
//! - Setup.exe で入れたときは、起動したときに GitHub のリリースから新しい版を落とし、終えるときに入れ替える
//! - Windows のリリース版はコンソールの窓を出さない。ログは設定のフォルダの dtxmanianxt.log にも書く（`logging`）

// 開発中（debug）は、ログが見えるようにコンソールを残す
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod band;
mod chart_sound;
mod config;
mod config_view;
mod fonts;
mod game;
mod gfx;
mod history;
mod inputs;
mod loading;
mod logging;
mod midi;
mod play_view;
mod practice;
mod practice_hud;
mod practice_popup;
mod result_training;
mod result_view;
mod select;
mod settings;
mod settings_ui;
mod sfx;
mod shutter;
mod skin;
mod songs;
mod stretch;
mod text;
mod time;
mod title_view;
mod ui;
mod update;
mod xa;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dtx::DrumLane;
use engine::input::{InputEvent, MidiMessage, MidiProfile, Normalizer};
use inputs::MidiMonitor;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::config::Config;
use crate::config_view::{
    ConfigAction, ConfigInfo, ConfigInput, ConfigView, ItemId, PlayPos, Setting,
};
use crate::game::{GaugeRule, HitSoundRule, Outcome, Play, Rules};
use crate::gfx::{Canvas, Gfx};
use crate::history::History;
use crate::loading::{Job, Loading};
use crate::midi::MidiIn;
use crate::play_view::PlayView;
use crate::practice::{PracticeOptions, PracticeSetup};
use crate::practice_popup::{Done, PopupInput, PopupMouse, PracticePopup};
use crate::result_view::{Page, ResultInfo, ResultView};
use crate::select::{PREVIEW_WAIT, SelectView};
use crate::sfx::{Se, Sfx};
use crate::shutter::Shutter;
use crate::songs::SongList;
use crate::time::host_us;
use crate::title_view::{Ending, TitleItem, TitleView};

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    egui_renderer: egui_wgpu::Renderer,
    /// 描く画像を続けて受け取れなかった回数
    timeouts: u32,
}

/// 描く画像をこの回数続けて受け取れなければ、画面への出力を作り直す
/// （1 回は 1 秒待つ。そのままにすると画面が止まったまま遊びだけ進む）
const TIMEOUTS_BEFORE_RECONFIGURE: u32 = 3;

impl Gpu {
    fn new(window: Arc<Window>) -> Result<Self> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .context("窓に描く準備（サーフェス）ができません")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .context("使える GPU が見つかりません")?;
        log::info!("GPU: {:?}", adapter.get_info().name);
        // NX のスキンには縦 4080 の画像などがあるので、GPU が扱える一番大きい画像の大きさを使う
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .context("GPU を使う準備ができません")?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("この GPU では窓に描けません")?;
        // egui は sRGB でない形式を前提に色を作るので、選べるならそちらにする
        let caps = surface.get_capabilities(&adapter);
        if let Some(f) = caps.formats.iter().find(|f| !f.is_srgb()) {
            config.format = *f;
        }
        // 遅れを小さくするため、待ち行列の深さを最小にする
        config.desired_maximum_frame_latency = 1;
        surface.configure(&device, &config);
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );
        Ok(Self {
            surface,
            device,
            queue,
            config,
            egui_renderer,
            timeouts: 0,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    fn render(&mut self, ctx: &egui::Context, mut output: egui::FullOutput) {
        // 字形などのテクスチャは、画面を描けないフレームでも必ず反映する（捨てると文字が出ない）
        let mut textures = std::mem::take(&mut output.textures_delta);
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, id, delta);
            }
        }
        let free: Vec<egui::TextureId> = textures.free.drain().collect();
        self.draw(ctx, output.shapes, output.pixels_per_point);
        for id in &free {
            self.egui_renderer.free_texture(id);
        }
    }

    fn draw(&mut self, ctx: &egui::Context, shapes: Vec<egui::epaint::ClippedShape>, ppp: f32) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.timeouts += 1;
                if self.timeouts >= TIMEOUTS_BEFORE_RECONFIGURE {
                    log::warn!("no frame for {} tries; reconfiguring", self.timeouts);
                    self.surface.configure(&self.device, &self.config);
                    self.timeouts = 0;
                }
                return;
            }
            other => {
                log::warn!("skip frame: {other:?}");
                self.surface.configure(&self.device, &self.config);
                return;
            }
        };
        self.timeouts = 0;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: ppp,
        };
        let jobs = ctx.tessellate(shapes, ppp);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let extra = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.02,
                            b: 0.05,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let mut pass = pass.forget_lifetime();
            self.egui_renderer.render(&mut pass, &jobs, &screen);
        }
        self.queue
            .submit(extra.into_iter().chain([encoder.finish()]));
        self.queue.present(frame);
    }
}

/// 曲を読むのに使う設定。読むスレッドへ渡せるように、設定から写しておく
struct LoadSettings {
    input_offset_ms: f64,
    drum_sounds: bool,
    /// 自動で叩くレーン、判定の幅、ゲージの決まりなど
    rules: Rules,
    /// MIDI のポートごとの名前と割り当て（[`MidiIn::ports`] と同じ並び）
    inputs: Vec<(String, MidiProfile)>,
}

impl LoadSettings {
    /// 曲を読む（時間がかかる）。`practice` があれば区間の練習
    fn load(self, path: &std::path::Path, practice: Option<PracticeOptions>) -> Result<Play> {
        let started = Instant::now();
        let mut p = match practice {
            Some(o) => {
                Play::load_practice(path, self.input_offset_ms, self.drum_sounds, &self.rules, o)?
            }
            None => Play::load(path, self.input_offset_ms, self.drum_sounds, &self.rules)?,
        };
        p.set_inputs(self.inputs);
        // 「叩いても音が出ない」と言われたとき、ログで分かるように
        log::info!(
            "loaded in {:.0}ms (hit sounds {})",
            started.elapsed().as_secs_f64() * 1000.0,
            if self.drum_sounds { "on" } else { "off" }
        );
        Ok(p)
    }
}

/// いまの画面
enum Screen {
    /// タイトル（動きは App の `title` に持つ。カーソルを覚えておくため）
    Title,
    /// 設定。`true` なら出たら選曲へ（F1 で開いたとき）、でなければタイトルへ
    Config(Box<ConfigView>, bool),
    /// 終わる前の画面
    End(Ending),
    /// 選曲（練習の決めごとのポップアップは App の `popup`）
    Select,
    /// 曲を読み込んでいる
    Loading(Box<Loading>),
    /// 遊んでいる（練習中も）。NX の演奏画面の動きと一緒に持つ
    Play(Box<Play>, Box<PlayView>),
    /// 結果。NX の結果の画面の動きと一緒に持つ
    Result(Box<Outcome>, Box<ResultView>),
}

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    midi: Option<MidiIn>,
    screen: Screen,
    songs: SongList,
    config: Config,
    /// プレイの記録
    history: History,
    /// 起動の引数で渡された譜面（ウィンドウができてから始める）
    pending: Option<PathBuf>,
    message: Option<String>,
    /// 選曲や結果の画面で、電子ドラムで操作するための入力（MIDI のポートごと）
    menu_inputs: Vec<Normalizer>,
    /// キーボードの割り当て（設定から作る）
    keys: Vec<(KeyCode, DrumLane)>,
    /// 届いた MIDI の見張り（設定の画面に出す最後の打撃と、ログ）
    monitor: MidiMonitor,
    /// MIDI の機器がつながったか外れたかを最後に調べた時刻
    ports_checked: Instant,
    /// 選曲の画面で最後に HH を叩いた時刻（2 回で難易度を変える）
    last_hh_us: Option<i64>,
    gfx: Gfx,
    /// 選曲画面の動き
    view: SelectView,
    /// 画面の効果音
    sfx: Sfx,
    /// タイトル画面の動き
    title: TitleView,
    /// 終わる（次のイベントでループを抜ける）
    quit: bool,
    /// 起動できなかったわけ（ループを抜けてから知らせる）
    startup_error: Option<anyhow::Error>,
    /// 画面の切り替えのシャッター（NX v2.0 `GitaDoraTransition`）
    shutter: Shutter,
    /// シャッターを閉じ終えたら移る先（閉じているあいだは操作を受け付けない）
    after_close: Option<AfterClose>,
    /// 選曲画面に重ねる、練習の決めごとのポップアップ
    popup: Option<Box<PracticePopup>>,
    /// 選曲の簡易設定か、演奏中の一時停止の板（設定の画面と同じ行）
    quick: Option<Box<ConfigView>>,
    /// 選曲の画面で最後に BD を叩いた時刻（2 回で簡易設定）
    last_bd_us: Option<i64>,
    /// 選曲画面のマウスホイールの、まだ 1 行に満たない分
    wheel: f32,
    /// 演奏中に設定と入力の情報を出す（I で切り替え）
    show_play_info: bool,
    /// 演奏中の短い知らせ（消す時刻）
    toast: Option<(String, Instant)>,
    /// 音符をすべて自動で叩く（開発用。`DTXMANIANXT_AUTOPLAY=1`。設定の自動演奏より優先）
    autoplay: bool,
    /// 新しい版を探して落とし、終えるときに入れ替える
    updater: update::Updater,
}

/// 演奏の画面が数えたスコアを、記録に残す形にする
fn play_score(f: &play_view::Finish, cleared: bool) -> records::PlayScore {
    records::PlayScore {
        score: f.score,
        rate: f.rate,
        progress: f.progress.clone(),
        cleared,
    }
}

/// 読み込みの画面を出しておく一番短い時間
const MIN_LOADING: Duration = Duration::from_millis(1500);

/// 演奏中の知らせを出しておく時間
const TOAST_FOR: Duration = Duration::from_millis(1500);
/// 譜面の音を切っているときに、演奏の始めに知らせを出しておく時間
const HIT_SOUND_NOTICE_FOR: Duration = Duration::from_secs(4);

/// 演奏中に譜面の音（叩いた音）を切り替えるキー。キーボードで叩くキー（A〜L、Q、Z）から離しておく
/// （前の M は J・K のすぐ下で、叩くつもりで押すと気づかないうちに切れてしまう）
const HIT_SOUND_KEY: KeyCode = KeyCode::F2;
/// [`HIT_SOUND_KEY`] の表示の名前
const HIT_SOUND_KEY_NAME: &str = "F2";

/// シャッターを閉じ終えたら移る先
enum AfterClose {
    Select,
    Title,
    /// 設定。`true` なら出たら選曲へ
    Config(bool),
    /// 演奏を一時停止の板の「演奏をやめる」でやめた。結果の画面へ（数はやめた時刻の曲の時刻 `at_ms` までで）
    Result {
        at_ms: f64,
    },
    /// 結果の画面から練習の決めごとへ（選曲画面に開く）
    Setup(PathBuf, Option<PracticeOptions>),
}

/// シャッターを開き始めるまでの待ち（v2 はフレームで数える。60 で割った時間にする）。
/// 選曲は 20 フレーム、設定は 2 フレーム、設定から出てタイトルへは 150 ミリ秒
const SELECT_OPEN_DELAY: Duration = Duration::from_millis(333);
const CONFIG_OPEN_DELAY: Duration = Duration::from_millis(33);
const CONFIG_LEAVE_OPEN_DELAY: Duration = Duration::from_millis(150);
/// 読み込みを Esc でやめてから閉じ始めるまで（v2：10 フレーム）
const LOADING_CANCEL_CLOSE_DELAY: Duration = Duration::from_millis(167);

/// MIDI の機器を開く（使わない設定にした機器は開かない）
fn open_midi(config: &Config) -> Option<MidiIn> {
    MidiIn::open(|ports| inputs::ports_to_open(config, ports))
        .map_err(|e| log::warn!("MIDI is off: {e:#}"))
        .ok()
}

/// MIDI の機器がつながったか外れたかを調べる間
const PORT_CHECK_EVERY: Duration = Duration::from_secs(2);

/// 2 回叩きと見なす間（上流と同じ 0.5 秒）
const DOUBLE_HIT_US: i64 = 500_000;

impl App {
    fn new(arg: Option<PathBuf>) -> Self {
        let mut config = Config::load();
        if config.take_over_nx() {
            config.save();
        }
        let midi = open_midi(&config);
        // 最初の音（タイトルの音）から設定の音量で鳴らす
        audio::set_master_volume(config.master_volume());
        let mut pending = None;
        // 引数がフォルダなら曲のフォルダに足し、譜面ならすぐ遊ぶ
        if let Some(p) = arg {
            if p.is_dir() {
                if config.add_song_dir(p) {
                    config.save();
                }
            } else {
                pending = Some(p);
            }
        }
        let mut songs = SongList::new(config.level);
        if !config.song_dirs.is_empty() {
            songs.rescan(config.song_dirs.clone());
        }
        let egui_ctx = egui::Context::default();
        let gfx = Gfx::new(&egui_ctx);
        let mut sfx = Sfx::new();
        sfx.load();
        let ports = midi.as_ref().map_or(&[][..], |m| m.ports.as_slice());
        let menu_inputs = inputs::normalizers(&config, ports);
        let keys = inputs::keyboard_map(&config);
        let history = History::open();
        songs.set_stats(history.chart_stats());
        let updater = update::Updater::start(config.auto_update);
        Self {
            window: None,
            gpu: None,
            egui_ctx,
            egui_state: None,
            midi,
            screen: Screen::Title,
            songs,
            config,
            history,
            pending,
            message: None,
            menu_inputs,
            keys,
            monitor: MidiMonitor::default(),
            ports_checked: Instant::now(),
            last_hh_us: None,
            gfx,
            view: SelectView::new(),
            sfx,
            title: TitleView::new(),
            quit: false,
            startup_error: None,
            shutter: Shutter::new(),
            after_close: None,
            popup: None,
            quick: None,
            last_bd_us: None,
            wheel: 0.0,
            show_play_info: false,
            toast: None,
            autoplay: std::env::var_os("DTXMANIANXT_AUTOPLAY").is_some_and(|v| v == "1"),
            updater,
        }
    }

    /// 選曲画面に戻る。v2 と同じく、入るたびにシャッターを開く（閉じていなければ閉じた形から）
    fn go_to_select(&mut self) {
        self.sfx.stop_title();
        self.popup = None;
        self.quick = None;
        self.screen = Screen::Select;
        self.view.enter();
        self.shutter.open(SELECT_OPEN_DELAY);
    }

    /// タイトルへ（白からのフェードから）
    fn go_to_title(&mut self) {
        self.message = None;
        self.screen = Screen::Title;
        self.title.enter();
    }

    /// 設定の画面を開く。`from_select` なら出たら選曲へ戻る。v2 と同じくシャッターを開いて入る
    fn open_config(&mut self, from_select: bool) {
        self.sfx.preview(None);
        self.quick = None;
        self.screen = Screen::Config(Box::default(), from_select);
        self.shutter.open(CONFIG_OPEN_DELAY);
    }

    /// シャッターを閉じ、閉じ終えたら `next` へ移る（閉じているあいだは操作を受け付けない）
    fn close_then(&mut self, next: AfterClose, delay: Duration) {
        if self.after_close.is_none() {
            self.after_close = Some(next);
            self.shutter.close(delay);
        }
    }

    /// シャッターを閉じ終えたので、決めておいた画面へ移る
    fn closed(&mut self, next: AfterClose) {
        match next {
            AfterClose::Select => self.go_to_select(),
            AfterClose::Title => {
                self.go_to_title();
                self.shutter.open(CONFIG_LEAVE_OPEN_DELAY);
            }
            AfterClose::Config(from_select) => self.open_config(from_select),
            AfterClose::Result { at_ms } => {
                self.finish_play(Some(at_ms));
                self.shutter.open(shutter::OPEN_DELAY);
            }
            AfterClose::Setup(path, previous) => self.open_setup(path, previous),
        }
    }

    /// 終わる前の画面を出す（終わりの音が鳴り終えたら閉じる）
    fn begin_end(&mut self) {
        let wait = self.sfx.play_end();
        self.screen = Screen::End(Ending::new(wait));
    }

    /// 割り当てが変わったので、メニューの操作に使う割り当てとキーボードの割り当てを作り直す
    fn refresh_pads(&mut self) {
        self.menu_inputs = inputs::normalizers(&self.config, self.ports());
        self.keys = inputs::keyboard_map(&self.config);
    }

    /// MIDI のポート（開いていないものも）
    fn ports(&self) -> &[midi::PortInfo] {
        self.midi.as_ref().map_or(&[][..], |m| m.ports.as_slice())
    }

    /// MIDI の機器を開き直す（つないだ機器や、使う設定を変えた機器を反映する）
    fn reopen_midi(&mut self) {
        // 先に閉じてから開き直す（同じポートを二重に開かない）
        self.midi = None;
        self.midi = open_midi(&self.config);
        self.monitor.reset();
        self.refresh_pads();
        if let Screen::Config(view, _) = &mut self.screen {
            view.devices_changed();
        }
    }

    /// 曲を遊んでいないときに、MIDI の機器がつながったか外れたかを時々調べ、変わっていれば開き直す。
    /// MIDI そのものが使えないとき（OS の MIDI を開けなかった）は調べない。設定の「Rescan Devices」で開き直せる
    fn check_ports(&mut self) {
        if self.midi.is_none()
            || self.ports_checked.elapsed() < PORT_CHECK_EVERY
            || !matches!(
                self.screen,
                Screen::Title | Screen::Config(..) | Screen::Select | Screen::Result(..)
            )
        {
            return;
        }
        self.ports_checked = Instant::now();
        let now = midi::list_ports();
        if self.midi.as_ref().is_some_and(|m| m.changed(&now)) {
            log::info!("MIDI devices changed");
            self.reopen_midi();
        }
    }

    /// レーンを叩くキー
    fn key_lane(&self, code: KeyCode) -> Option<DrumLane> {
        self.keys.iter().find(|(k, _)| *k == code).map(|(_, l)| *l)
    }

    /// カーソルを動かす
    fn move_cursor(&mut self, delta: isize) {
        if self.songs.len() == 0 {
            return;
        }
        self.songs.move_by(delta);
        self.view.moved(delta);
        self.sfx.play(Se::Move);
    }

    /// 曲を始める。読み込みの画面から
    fn start(&mut self, path: PathBuf) {
        self.open_loading(path, None);
    }

    /// 読み込みの画面を開く。`practice` があれば区間の練習
    fn open_loading(&mut self, path: PathBuf, practice: Option<PracticeOptions>) {
        // やり直しなどで途中の曲を捨てるときも、そこまでの記録は残す
        self.save_unfinished();
        let label = self.songs.chart_label(&path);
        // 音が無いときも、読み込みの画面が一瞬で消えないように少しは出す
        let wait = self.sfx.play_loading().max(MIN_LOADING);
        // 曲は別のスレッドで読む（画面のスレッドを止めない。読み込み中も Esc がすぐ効く）
        let settings = self.load_settings();
        let job = {
            let path = path.clone();
            Job::spawn("chart loader", move || settings.load(&path, practice))
        };
        self.screen = Screen::Loading(Box::new(Loading::new(&path, &label, wait, job)));
    }

    /// 曲を読むのに使う設定（読むスレッドへ渡す）
    fn load_settings(&self) -> LoadSettings {
        LoadSettings {
            input_offset_ms: self.config.input_offset_ms,
            drum_sounds: self.config.drum_sounds,
            rules: self.rules(),
            inputs: self
                .ports()
                .iter()
                .map(|p| p.name.clone())
                .zip(inputs::profiles(&self.config, self.ports()))
                .collect(),
        }
    }

    /// 判定の決まり（設定の表から）
    fn rules(&self) -> Rules {
        let v = &self.config.settings;
        Rules {
            // 開発用の DTXMANIANXT_AUTOPLAY=1 なら全部
            auto: if self.autoplay {
                DrumLane::ALL.to_vec()
            } else {
                v.auto_lanes()
            },
            windows: v.hit_windows(false),
            pedal_windows: v.hit_windows(true),
            tight: v.flag(settings::TIGHT),
            gauge: GaugeRule {
                miss_factor: v.damage_factor(),
                risky: v.number(settings::RISKY) as u32,
                hazard: v.flag(settings::HAZARD),
            },
            groups: v.lane_groups(),
            hit_sounds: HitSoundRule {
                pad_first: v.pad_sound_first(),
                muting_lp: v.flag(settings::MUTING_LP),
            },
        }
    }

    /// 曲をこのスレッドで読む（時間がかかる。演奏中のやり直しで使う）
    fn load_play(&self, path: &std::path::Path, practice: Option<PracticeOptions>) -> Result<Play> {
        let mut p = self.load_settings().load(path, practice)?;
        self.add_weak_band(&mut p);
        Ok(p)
    }

    /// 練習なら、苦手な小節の帯を記録から付ける（記録は画面のスレッドで読む）
    fn add_weak_band(&self, p: &mut Play) {
        if p.practice.is_none() {
            return;
        }
        let band = self
            .history
            .weak_map(&p.path, p.notes_per_measure(), None)
            .weakness;
        if let Some(pr) = p.practice.as_mut() {
            pr.band = band;
        }
    }

    /// 読み込んだ曲を鳴らし始め、演奏の画面にする
    fn enter_play(&mut self, mut play: Play) {
        let label = self.songs.chart_label(&play.path);
        play.begin();
        let lag = play_view::LagDigits::from_config(&self.config);
        let looks = play_view::Looks::from_settings(&self.config.settings);
        let view = PlayView::new(&play, &label, lag, looks);
        self.message = None;
        self.quick = None;
        // 譜面の音を切ったままだと、叩いても音が出ないわけが分からない。始めにしばらく知らせる
        if !play.drum_sounds {
            self.toast_for(
                format!("叩いた音: 鳴らさない（{HIT_SOUND_KEY_NAME} で切り替え）"),
                HIT_SOUND_NOTICE_FOR,
            );
        }
        self.screen = Screen::Play(Box::new(play), Box::new(view));
    }

    /// 演奏中のやり直し。読み込みの画面を出さずにすぐ始める
    fn restart(&mut self, path: PathBuf, practice: Option<PracticeOptions>) {
        self.save_unfinished();
        // 前の曲の音を止めてから読む
        self.screen = Screen::Select;
        match self.load_play(&path, practice) {
            Ok(p) => self.enter_play(p),
            Err(e) => self.load_failed(&e),
        }
    }

    fn load_failed(&mut self, e: &anyhow::Error) {
        log::error!("{e:#}");
        self.sfx.stop_loading();
        self.message = Some(format!("読み込めませんでした: {e:#}"));
        self.go_to_select();
    }

    /// 読み込みの画面を進める。読み終えた曲を受け取り、音が鳴り終えたら演奏へ
    fn pump_loading(&mut self) {
        // Esc でやめてシャッターを閉じているあいだは、読み終えても演奏へ移らない
        if self.after_close.is_some() {
            return;
        }
        let Screen::Loading(l) = &mut self.screen else {
            return;
        };
        match l.take_result() {
            Some(Ok(mut p)) => {
                self.add_weak_band(&mut p);
                if let Screen::Loading(l) = &mut self.screen {
                    l.play = Some(Box::new(p));
                }
            }
            Some(Err(e)) => {
                self.load_failed(&e);
                return;
            }
            None => {}
        }
        if let Screen::Loading(l) = &mut self.screen
            && l.done()
            && let Some(p) = l.play.take()
        {
            self.enter_play(*p);
        }
    }

    /// ゲージが尽きたら途中で終わるか（設定に無ければ上流と同じく終わる）
    fn stage_failed_enabled(&self) -> bool {
        self.config.stage_failed()
    }

    /// 演奏中に設定を変えたことを知らせる
    fn toast(&mut self, text: String) {
        self.toast_for(text, TOAST_FOR);
    }

    /// 演奏中に `time` のあいだ知らせを出す
    fn toast_for(&mut self, text: String, time: Duration) {
        self.toast = Some((text, Instant::now() + time));
    }

    /// 練習の決めごとのポップアップを選曲画面に開く。`previous` があればその区間とつなぎ方を選んでおく
    fn open_setup(&mut self, path: PathBuf, previous: Option<PracticeOptions>) {
        if !matches!(self.screen, Screen::Select) {
            self.save_unfinished();
            self.go_to_select();
        }
        match PracticeSetup::load(&path, &self.history, previous, self.config.practice_prefs()) {
            Ok(s) => {
                self.message = None;
                self.popup = Some(Box::new(PracticePopup::new(s)));
            }
            Err(e) => {
                log::error!("{e:#}");
                self.message = Some(format!("読み込めませんでした: {e:#}"));
            }
        }
    }

    /// ポップアップを操作する
    fn popup_input(&mut self, input: PopupInput) {
        let Some(p) = &mut self.popup else {
            return;
        };
        let r = p.input(input);
        self.popup_response(r);
    }

    /// ポップアップの操作の結果：音を鳴らし、決めごとを覚え、閉じるか練習を始める
    fn popup_response(&mut self, r: practice_popup::Response) {
        let Some(p) = &self.popup else {
            return;
        };
        if let Some(se) = r.se {
            self.sfx.play(se);
        }
        // 区間のほかの決めごとは次の練習でも使う
        let prefs = p.setup.prefs();
        if prefs != self.config.practice_prefs() {
            self.config.set_practice_prefs(prefs);
            self.config.save();
        }
        match r.done {
            Some(Done::Start(o)) => {
                let path = p.setup.path.clone();
                self.popup = None;
                self.sfx.preview(None);
                self.start_practice(path, o);
            }
            Some(Done::Close) => self.popup = None,
            None => {}
        }
    }

    /// 区間の練習を始める
    fn start_practice(&mut self, path: PathBuf, options: PracticeOptions) {
        self.open_loading(path, Some(options));
    }

    /// 曲（練習）を終える。結果の画面へ。`at_ms`（曲の時刻）があればそこでやめたことにする
    fn finish_play(&mut self, at_ms: Option<f64>) {
        self.quick = None;
        let Screen::Play(p, view) = &self.screen else {
            return;
        };
        let finish = view.finish(p, at_ms.unwrap_or_else(|| p.now_ms()));
        let practice = p.practice.is_some();
        // Esc でやめたときも結果は出す（記録は「最後まで遊んでいない」として残す）
        let cleared = p.finished();
        let session = self.history.save(p, Some(play_score(&finish, cleared)));
        let stats = self.history.chart_stats();
        // New Record と前回のベストのバーは、今回より前のプレイと比べる。自動で叩くレーンがあるときと、
        // 早送りか巻き戻しをしたときは比べない
        let best = if p.has_auto() || p.skipped() {
            None
        } else {
            self.history.best_score(&p.path, session)
        };
        let mut outcome = p.outcome();
        if !practice {
            let weak = self.history.weak_map(
                &outcome.path,
                &outcome.notes_per_measure,
                Some(&outcome.results),
            );
            outcome.overcome = self.history.check_overcome(
                &outcome.path,
                outcome.started_at,
                &outcome.notes_per_measure,
                &outcome.results,
                &weak,
            );
            outcome.weak = Some(weak);
            outcome.tendencies = self.history.tendencies();
        }
        let lag_counts = self
            .config
            .show_lag_hit_count
            .then_some(self.config.show_lag_time_color == 1);
        let view = ResultView::new(ResultInfo {
            title: outcome.title.clone(),
            artist: outcome.artist.clone(),
            preimage: p.preimage.clone(),
            level: outcome.level,
            tally: outcome.tally.clone(),
            notes: outcome.notes,
            practice,
            cleared: cleared && !practice,
            finish,
            best,
            lag_counts,
        });
        self.screen = Screen::Result(Box::new(outcome), Box::new(view));
        self.songs.set_stats(stats);
    }

    /// 遊んでいる途中の曲があれば、そこまでの記録を残す（STAGE FAILED や、途中でやめたとき）
    fn save_unfinished(&mut self) {
        if let Screen::Play(p, view) = &self.screen {
            let now = view.frozen_ms().unwrap_or_else(|| p.now_ms());
            let finish = view.finish(p, now);
            self.history.save(p, Some(play_score(&finish, false)));
            self.songs.set_stats(self.history.chart_stats());
        }
    }

    fn add_song_dir(&mut self, dir: PathBuf) {
        if self.config.add_song_dir(dir) {
            self.config.save();
        }
        self.songs.rescan(self.config.song_dirs.clone());
    }

    /// 届いた MIDI をすべて処理し、時間を進める
    fn pump(&mut self) {
        self.songs.poll();
        self.check_ports();
        let mut assigned = None;
        let closing = self.after_close.is_some();
        // 届いた MIDI を読み、見張りに見せてから、いまの画面へ渡す
        let received: Vec<(midi::RawMidi, Vec<MidiMessage>)> = match &self.midi {
            Some(midi) => midi
                .drain()
                .map(|raw| {
                    let msgs = MidiMessage::parse_all(&raw.bytes);
                    for m in &msgs {
                        self.monitor.observe(
                            raw.port,
                            midi.name(raw.port),
                            m,
                            self.menu_inputs.get(raw.port).map(Normalizer::profile),
                        );
                    }
                    (raw, msgs)
                })
                .collect(),
            None => Vec::new(),
        };
        let paused_panel = self.quick.is_some();
        match &mut self.screen {
            // シャッターを閉じているあいだは叩いても何もしない（演奏をやめたあとの音符も数えない）
            _ if closing => {}
            // 一時停止の板を開いているあいだは、パッドで板を操作する
            Screen::Play(play, _) if !paused_panel => {
                for (raw, msgs) in &received {
                    play.on_midi(raw, msgs);
                }
            }
            Screen::Config(view, _) if view.wants_raw() => {
                // パッドの割り当て：叩いたノートを、叩いた機器の割り当てにする
                let hit = received.iter().find_map(|(raw, msgs)| {
                    msgs.iter().find_map(|m| match *m {
                        MidiMessage::NoteOn { note, .. } => Some((raw.port, note)),
                        _ => None,
                    })
                });
                if let Some((port, note)) = hit {
                    assigned = Some(view.pad_hit(port, note));
                }
            }
            _ => {
                let mut hits: Vec<(DrumLane, i64)> = Vec::new();
                for (raw, msgs) in &received {
                    let Some(input) = self.menu_inputs.get_mut(raw.port) else {
                        continue;
                    };
                    for &m in msgs {
                        if let Some(InputEvent::Hit {
                            lane: Some(lane),
                            host_us,
                            ..
                        }) = input.feed(m, raw.arrival_us)
                        {
                            hits.push((lane, host_us));
                        }
                    }
                }
                for (lane, at) in hits {
                    self.on_menu_pad(lane, at);
                }
            }
        }
        if let Screen::Config(view, _) = &mut self.screen
            && !closing
            && let Some(r) = view.tick()
        {
            assigned.get_or_insert(r);
        }
        if let Some(r) = assigned {
            self.config_response(r);
        }
        self.pump_loading();
        self.pump_title();
        if let Screen::Result(_, view) = &mut self.screen {
            for se in view.due_sounds() {
                self.sfx.play(se);
            }
        }
        let stage_failed = self.stage_failed_enabled();
        let mut failed_out = false;
        let mut finished = false;
        if let Screen::Play(play, view) = &mut self.screen
            && !closing
        {
            if view.frozen_ms().is_some() {
                // STAGE FAILED の途中は譜面を進めない
                failed_out = view.failure_done();
            } else {
                // 判定の線より上に見えている時間
                play.view_ahead_ms = play_view::view_ahead_ms(self.config.scroll_px_per_ms);
                play.update();
                view.feed(play);
                if stage_failed && play.practice.is_none() && !play.skipped() && view.gauge_empty()
                {
                    // 上流と同じく、音を止めて STAGE FAILED を出し、選曲に戻る（記録は残す）
                    view.start_failure(play.now_ms() + self.config.visual_offset_ms);
                    play.silence();
                    self.sfx.play(Se::StageFailed);
                } else {
                    finished = play.finished();
                }
            }
        }
        if failed_out {
            self.save_unfinished();
            self.go_to_select();
        } else if finished {
            self.finish_play(None);
        }
        if self.shutter.is_closed()
            && let Some(next) = self.after_close.take()
        {
            self.closed(next);
        }
        if matches!(self.screen, Screen::Select) && self.after_close.is_none() {
            // 選んでから少し待ってプレビューの音を鳴らす（上流 CActSelectPresound）
            let preview = if self.view.settled_for(&self.songs, PREVIEW_WAIT) {
                self.view.preview_path(&self.songs)
            } else {
                None
            };
            self.sfx.preview(preview);
        } else {
            self.sfx.preview(None);
        }
        self.sfx.poll();
    }

    /// タイトル、設定、終わりの画面を進める
    fn pump_title(&mut self) {
        match &mut self.screen {
            Screen::Title => {
                if self.title.take_title_sound() {
                    self.sfx.play_title();
                }
            }
            Screen::End(e) if e.done() => self.quit = true,
            _ => {}
        }
    }

    fn title_move(&mut self, delta: i32) {
        if self.title.move_by(delta) {
            self.sfx.play(Se::Move);
        }
    }

    fn title_decide(&mut self) {
        match self.title.decide() {
            Some(TitleItem::GameStart) => {
                // 上流と同じく、GAME START の音が無ければ決定の音
                let se = if self.sfx.has(Se::GameStart) {
                    Se::GameStart
                } else {
                    Se::Decide
                };
                self.sfx.play(se);
                // v2 と同じく、タイトルの音を止めてシャッターを閉じる
                self.sfx.stop_title();
                self.close_then(AfterClose::Select, Duration::ZERO);
            }
            Some(TitleItem::Config) => {
                self.sfx.play(Se::Decide);
                self.sfx.stop_title();
                self.close_then(AfterClose::Config(false), Duration::ZERO);
            }
            Some(TitleItem::Exit) => {
                self.sfx.play(Se::Decide);
                self.begin_end();
            }
            None => {}
        }
    }

    fn on_title_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::ArrowUp => self.title_move(-1),
            KeyCode::ArrowDown => self.title_move(1),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.title_decide(),
            // 上流と同じく Esc で終わる
            KeyCode::Escape => self.begin_end(),
            _ => {}
        }
    }

    /// 設定の画面の操作（v2 と同じく、シャッターが開ききるまでは受け付けない）
    fn config_input(&mut self, input: ConfigInput) {
        if self.shutter.animating() {
            return;
        }
        let Screen::Config(view, _) = &mut self.screen else {
            return;
        };
        let info = config_info(&self.config, &self.midi, &self.monitor, &self.songs);
        let rows = config_view::rows(view.page(), &info);
        let r = view.input(input, &rows);
        self.config_response(r);
    }

    fn config_response(&mut self, r: config_view::Response) {
        if let Some(se) = r.se {
            self.sfx.play(se);
        }
        if let Some(a) = r.action {
            self.config_action(a);
        }
    }

    fn config_action(&mut self, action: ConfigAction) {
        match action {
            ConfigAction::Set(id, v) => self.config_set(id, v),
            ConfigAction::Run(id) => self.config_run(id),
            ConfigAction::TogglePad(lane, port, note) => {
                let ports = self.ports().to_vec();
                let added = inputs::toggle_note(&mut self.config, &ports, port, note, lane);
                let device = self
                    .midi
                    .as_ref()
                    .map_or("MIDI", |m| m.name(port))
                    .to_owned();
                log::info!(
                    "{} note {note} of {device} {} {}",
                    if added { "assigned" } else { "removed" },
                    if added { "to" } else { "from" },
                    lane.name()
                );
                self.config.save();
                self.refresh_pads();
                self.config_toast(if added {
                    format!(
                        "ノート {note} を{}に足しました（{device}）",
                        inputs::lane_label(lane)
                    )
                } else {
                    format!(
                        "ノート {note} を{}から外しました（{device}）",
                        inputs::lane_label(lane)
                    )
                });
            }
            ConfigAction::ToggleKey(lane, code) => {
                let added = inputs::toggle_key(&mut self.config, code, lane);
                self.config.save();
                self.refresh_pads();
                let key = inputs::key_label(code);
                self.config_toast(if added {
                    format!("キー {key} を{}に足しました", inputs::lane_label(lane))
                } else {
                    format!("キー {key} を{}から外しました", inputs::lane_label(lane))
                });
            }
            ConfigAction::AssignPad(lane, port, note) => {
                let ports = self.ports().to_vec();
                inputs::assign_note(&mut self.config, &ports, port, note, lane);
                log::info!(
                    "assigned note {note} of {} to {}",
                    self.midi.as_ref().map_or("MIDI", |m| m.name(port)),
                    lane.name()
                );
                self.config.save();
                self.refresh_pads();
            }
            ConfigAction::AssignKey(lane, code) => {
                inputs::set_key(&mut self.config, code, Some(lane));
                self.config.save();
                self.refresh_pads();
            }
            ConfigAction::ClearLane(lane) => {
                let ports = self.ports().to_vec();
                inputs::clear_lane(&mut self.config, &ports, lane);
                self.config.save();
                self.refresh_pads();
                self.config_toast(format!(
                    "{}の割り当てを全部外しました",
                    inputs::lane_label(lane)
                ));
            }
            ConfigAction::Leave => {
                self.message = None;
                let next = if matches!(self.screen, Screen::Config(_, true)) {
                    AfterClose::Select
                } else {
                    AfterClose::Title
                };
                self.close_then(next, Duration::ZERO);
            }
        }
    }

    /// 設定の画面の下に短い知らせを出す
    fn config_toast(&mut self, text: String) {
        if let Some(q) = &mut self.quick {
            q.toast(text);
        } else if let Screen::Config(view, _) = &mut self.screen {
            view.toast(text);
        }
    }

    /// 簡易設定の板の項目を作るのに要るもの（選んでいる曲、演奏中なら曲の位置）
    fn quick_info(&self) -> ConfigInfo<'_> {
        let mut info = config_info(&self.config, &self.midi, &self.monitor, &self.songs);
        info.song = self.songs.selected_chart().is_some();
        if let Screen::Play(p, _) = &self.screen {
            info.play = Some(PlayPos {
                practice: p.practice.is_some(),
                at_ms: p.now_ms(),
                end_ms: p.end_ms,
            });
        }
        info
    }

    /// 選曲の簡易設定（BD 2 回）を開く
    fn open_quick(&mut self) {
        if self.quick.is_some() || self.popup.is_some() {
            return;
        }
        self.sfx.play(Se::Decide);
        self.quick = Some(Box::new(ConfigView::panel(config_view::Page::Quick)));
    }

    /// 演奏を一時停止して、一時停止の板を開く（Esc・Space・Pause）
    fn open_pause(&mut self) {
        let Screen::Play(play, _) = &mut self.screen else {
            return;
        };
        if !play.paused() {
            play.toggle_pause();
        }
        self.sfx.play(Se::Decide);
        self.quick = Some(Box::new(ConfigView::panel(config_view::Page::Pause)));
    }

    /// 一時停止の板を閉じて、演奏を続ける
    fn resume_play(&mut self) {
        self.quick = None;
        if let Screen::Play(play, _) = &mut self.screen
            && play.paused()
        {
            play.toggle_pause();
        }
    }

    /// 簡易設定の板の操作
    fn quick_input(&mut self, input: ConfigInput) {
        let Some(page) = self.quick.as_ref().map(|q| q.page()) else {
            return;
        };
        let rows = config_view::rows(page, &self.quick_info());
        let Some(q) = &mut self.quick else {
            return;
        };
        let r = q.input(input, &rows);
        if let Some(se) = r.se {
            self.sfx.play(se);
        }
        if let Some(a) = r.action {
            self.quick_action(a);
        }
    }

    fn quick_action(&mut self, action: ConfigAction) {
        match action {
            ConfigAction::Set(ItemId::Seek, v) => {
                // 上流 SkipForward / SkipBackward。飛んだプレイは記録に残さない
                if let Screen::Play(play, _) = &mut self.screen {
                    let d = v as f64 * 1000.0 - play.now_ms();
                    play.seek(d);
                }
            }
            ConfigAction::Set(id, v) => {
                self.config_set(id, v);
                // 演奏が持っている値も合わせる
                if let Screen::Play(play, _) = &mut self.screen {
                    play.input_offset_ms = self.config.input_offset_ms;
                    play.drum_sounds = self.config.drum_sounds;
                }
            }
            ConfigAction::Run(ItemId::Resume) | ConfigAction::Leave => {
                if matches!(self.screen, Screen::Play(..)) {
                    self.resume_play();
                } else {
                    self.quick = None;
                }
            }
            ConfigAction::Run(ItemId::Retry) => {
                if let Screen::Play(play, _) = &self.screen {
                    let path = play.path.clone();
                    let practice = play.practice.as_ref().map(|p| p.restart_options());
                    self.quick = None;
                    self.restart(path, practice);
                }
            }
            ConfigAction::Run(ItemId::Quit) => {
                if let Screen::Play(play, _) = &self.screen {
                    // Esc でやめていたときと同じく、シャッターを閉じてから結果へ
                    let at_ms = play.now_ms();
                    self.quick = None;
                    self.close_then(AfterClose::Result { at_ms }, Duration::ZERO);
                }
            }
            ConfigAction::Run(ItemId::PracticeSong) => {
                self.quick = None;
                self.select_action(ui::SelectAction::Practice);
            }
            ConfigAction::Run(ItemId::AllSettings) => {
                self.quick = None;
                self.sfx.preview(None);
                self.close_then(AfterClose::Config(true), Duration::ZERO);
            }
            ConfigAction::Run(id) => self.config_run(id),
            // 簡易設定の板には割り当ての行が無い
            _ => {}
        }
    }

    /// 簡易設定・一時停止の板を開いているときのキー
    fn on_quick_key(&mut self, code: KeyCode) {
        let paused = matches!(self.screen, Screen::Play(..));
        let input = match code {
            KeyCode::ArrowUp => ConfigInput::Up,
            KeyCode::ArrowDown => ConfigInput::Down,
            KeyCode::ArrowLeft => ConfigInput::Left,
            KeyCode::ArrowRight => ConfigInput::Right,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => ConfigInput::Decide,
            KeyCode::Escape | KeyCode::Backspace => ConfigInput::Cancel,
            KeyCode::Pause if paused => {
                self.resume_play();
                return;
            }
            KeyCode::KeyR if paused => {
                self.quick_action(ConfigAction::Run(ItemId::Retry));
                return;
            }
            KeyCode::KeyP if !paused => {
                self.quick_action(ConfigAction::Run(ItemId::PracticeSong));
                return;
            }
            KeyCode::F1 if !paused => {
                self.sfx.play(Se::Decide);
                self.quick_action(ConfigAction::Run(ItemId::AllSettings));
                return;
            }
            _ => return,
        };
        self.quick_input(input);
    }

    /// 設定の画面で値を決めた（ON / OFF は 1 と 0、選択肢は何番目か）
    fn config_set(&mut self, id: ItemId, v: i64) {
        match id {
            ItemId::Value(s) => {
                config_view::set(s, &mut self.config, v);
                if s == Setting::MasterVolume {
                    audio::set_master_volume(self.config.master_volume());
                }
                self.config.save();
            }
            ItemId::Table(key) => {
                self.config.settings.set_raw(key, v);
                self.config.save();
            }
            ItemId::Practice(d) => {
                config_view::set_drill(d, &mut self.config, v);
                self.config.save();
            }
            ItemId::DeviceUse(port) => {
                let ports = self.ports().to_vec();
                if let Some(d) = inputs::device_mut(&mut self.config, &ports, port) {
                    d.disabled = v == 0;
                    self.config.save();
                    self.reopen_midi();
                }
            }
            ItemId::DevicePreset(port) => {
                let ports = self.ports().to_vec();
                let preset = engine::input::Preset::ALL
                    [v.clamp(0, engine::input::Preset::ALL.len() as i64 - 1) as usize];
                if let Some(d) = inputs::device_mut(&mut self.config, &ports, port) {
                    d.preset = preset.name().to_owned();
                    self.config.save();
                    self.refresh_pads();
                }
            }
            ItemId::DeviceChannel(port) => {
                let ports = self.ports().to_vec();
                if let Some(d) = inputs::device_mut(&mut self.config, &ports, port) {
                    // 0 はすべて、1〜16
                    d.channel = v.clamp(0, 16) as u8;
                    self.config.save();
                    self.refresh_pads();
                }
            }
            ItemId::DeviceMinVelocity(port) => {
                let ports = self.ports().to_vec();
                if let Some(d) = inputs::device_mut(&mut self.config, &ports, port) {
                    d.min_velocity = v.clamp(0, 127) as u8;
                    self.config.save();
                    self.refresh_pads();
                }
            }
            _ => {}
        }
    }

    /// 設定の画面の動かす項目
    fn config_run(&mut self, id: ItemId) {
        match id {
            ItemId::AddSongDir => {
                if let Some(dir) = ui::pick_folder("曲のフォルダを選ぶ") {
                    self.add_song_dir(dir);
                }
            }
            ItemId::SongDir(i) => {
                if i < self.config.song_dirs.len() {
                    self.config.song_dirs.remove(i);
                    self.config.save();
                    self.songs.rescan(self.config.song_dirs.clone());
                    self.config_toast("曲のフォルダを一覧から外しました".to_owned());
                }
            }
            ItemId::Rescan => {
                if !self.config.song_dirs.is_empty() {
                    self.songs.rescan(self.config.song_dirs.clone());
                }
            }
            ItemId::ReconnectMidi => self.reopen_midi(),
            ItemId::ResetPads => {
                inputs::reset_all(&mut self.config);
                self.config.save();
                self.refresh_pads();
                self.config_toast("割り当てを既定に戻しました".to_owned());
            }
            ItemId::DeviceReset(port) => {
                let ports = self.ports().to_vec();
                inputs::reset_device(&mut self.config, &ports, port);
                self.config.save();
                self.refresh_pads();
                self.config_toast("この機器の割り当てをプリセットに戻しました".to_owned());
            }
            _ => {}
        }
    }

    fn on_key(&mut self, code: KeyCode) {
        if self.after_close.is_some() {
            return;
        }
        let at = host_us();
        let lane = self.key_lane(code);
        match &mut self.screen {
            Screen::Title => self.on_title_key(code),
            Screen::Config(view, _) => {
                if let Some(r) = view.key_hit(code) {
                    self.config_response(r);
                    return;
                }
                let input = match code {
                    KeyCode::ArrowUp => ConfigInput::Up,
                    KeyCode::ArrowDown => ConfigInput::Down,
                    KeyCode::ArrowLeft => ConfigInput::Left,
                    KeyCode::ArrowRight => ConfigInput::Right,
                    KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => ConfigInput::Decide,
                    KeyCode::Escape | KeyCode::Backspace => ConfigInput::Cancel,
                    _ => return,
                };
                self.config_input(input);
            }
            Screen::End(_) => {}
            Screen::Loading(_) => {
                // 上流と同じく Esc で読み込みをやめて選曲に戻る（v2：少し待ってシャッターを閉じる）
                if code == KeyCode::Escape {
                    self.sfx.stop_loading();
                    self.close_then(AfterClose::Select, LOADING_CANCEL_CLOSE_DELAY);
                }
            }
            Screen::Play(..) if self.quick.is_some() => self.on_quick_key(code),
            Screen::Play(play, _) => {
                if let Some(lane) = lane {
                    play.on_key(lane, at);
                    return;
                }
                self.on_play_key(code);
            }
            Screen::Select => self.on_select_key(code),
            Screen::Result(_, view) => match code {
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape => {
                    self.result_action(ui::ResultAction::Back);
                }
                KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                    view.turn();
                    self.sfx.play(Se::Change);
                }
                KeyCode::KeyR => self.result_action(ui::ResultAction::Retry),
                KeyCode::KeyP => self.result_action(ui::ResultAction::Practice(None)),
                _ => {}
            },
        }
    }

    fn on_play_key(&mut self, code: KeyCode) {
        let Screen::Play(play, view) = &mut self.screen else {
            return;
        };
        let failing = view.frozen_ms().is_some();
        let toast = match code {
            // 一時停止して、一時停止の板を開く（上流 Pause。やめるのは板の「演奏をやめる」から）。
            // STAGE FAILED を出しているあいだは止めない（出し終えると選曲に戻る）
            KeyCode::Escape | KeyCode::Space | KeyCode::Pause => {
                if !failing {
                    self.open_pause();
                }
                return;
            }
            KeyCode::KeyR => {
                let path = play.path.clone();
                let practice = play.practice.as_ref().map(|p| p.restart_options());
                self.restart(path, practice);
                return;
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight if play.practice.is_some() => {
                // 区間を 1 小節ずらして始め直す
                let d = if code == KeyCode::ArrowLeft { -1 } else { 1 };
                let path = play.path.clone();
                if let Some(mut o) = play.practice.as_ref().map(|p| p.restart_options()) {
                    o.section = o.section.shifted(d, play.notes_per_measure().len());
                    self.restart(path, Some(o));
                }
                return;
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                // 上流 SkipForward / SkipBackward
                let width = self.config.settings.number(settings::SKIP_MS) as f64;
                let d = if code == KeyCode::ArrowLeft {
                    -width
                } else {
                    width
                };
                let Some(to) = play.seek(d) else {
                    return;
                };
                format!(
                    "{} {:.0} 秒へ（このプレイは記録に残しません）",
                    if d < 0.0 { "巻き戻し" } else { "早送り" },
                    to.max(0.0) / 1000.0
                )
            }
            KeyCode::Comma | KeyCode::Period if play.practice.is_some() => {
                play.nudge_speed(if code == KeyCode::Comma { -1 } else { 1 });
                let speed = play.practice.as_ref().map_or(100, |p| p.ladder().speed());
                format!("次の周から速度 {speed}%")
            }
            HIT_SOUND_KEY => {
                play.drum_sounds = !play.drum_sounds;
                self.config.drum_sounds = play.drum_sounds;
                self.config.save();
                format!(
                    "叩いた音を{}",
                    if play.drum_sounds {
                        "鳴らす"
                    } else {
                        "鳴らさない"
                    }
                )
            }
            KeyCode::Minus | KeyCode::Equal => {
                let d = if code == KeyCode::Minus { -1.0 } else { 1.0 };
                play.input_offset_ms += d;
                self.config.input_offset_ms = play.input_offset_ms;
                self.config.save();
                format!("入力の補正 {:+.0}ms", play.input_offset_ms)
            }
            KeyCode::BracketLeft | KeyCode::BracketRight => {
                let d = if code == KeyCode::BracketLeft {
                    -1.0
                } else {
                    1.0
                };
                self.config.visual_offset_ms += d;
                self.config.save();
                format!("表示の補正 {:+.0}ms", self.config.visual_offset_ms)
            }
            KeyCode::ArrowUp | KeyCode::ArrowDown => {
                let d = if code == KeyCode::ArrowUp {
                    0.05
                } else {
                    -0.05
                };
                self.config.scroll_px_per_ms = (self.config.scroll_px_per_ms + d).clamp(0.1, 3.0);
                self.config.save();
                format!("譜面の速さ {:.2}", self.config.scroll_px_per_ms)
            }
            KeyCode::KeyI => {
                self.show_play_info = !self.show_play_info;
                return;
            }
            _ => return,
        };
        self.toast(toast);
    }

    /// 選曲画面のマウス：ホイールで動かし、曲バーを押すとそこへ動く（選んでいる曲なら決める）
    fn select_mouse(&mut self, mouse: SelectMouse) {
        if !matches!(self.screen, Screen::Select)
            || self.after_close.is_some()
            || self.quick.is_some()
        {
            self.wheel = 0.0;
            return;
        }
        if let Some(p) = &mut self.popup {
            self.wheel = 0.0;
            let r = p.mouse(mouse.popup);
            self.popup_response(r);
            return;
        }
        self.wheel += mouse.wheel;
        let steps = self.wheel.trunc();
        if steps != 0.0 {
            self.wheel -= steps;
            // 上へ回すと前の曲
            self.move_cursor(-steps as isize);
        }
        match mouse.clicked_row {
            Some(0) => self.select_action(ui::SelectAction::Enter),
            Some(d) => self.move_cursor(d),
            None => {}
        }
        if let Some(d) = mouse.clicked_sort {
            self.change_sort(d);
        }
    }

    fn on_select_key(&mut self, code: KeyCode) {
        if self.quick.is_some() {
            self.on_quick_key(code);
            return;
        }
        if self.popup.is_some() {
            self.on_popup_key(code);
            return;
        }
        // 簡易設定（パッドなら BD を 2 回）
        if code == KeyCode::F2 {
            self.open_quick();
            return;
        }
        if code == KeyCode::F1 {
            self.sfx.play(Se::Decide);
            self.sfx.preview(None);
            self.close_then(AfterClose::Config(true), Duration::ZERO);
            return;
        }
        let delta = match code {
            KeyCode::ArrowUp => -1,
            KeyCode::ArrowDown => 1,
            KeyCode::PageUp => -10,
            KeyCode::PageDown => 10,
            _ => 0,
        };
        if delta != 0 {
            self.move_cursor(delta);
            return;
        }
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                self.select_action(ui::SelectAction::Enter)
            }
            KeyCode::Backspace => self.select_action(ui::SelectAction::Back),
            KeyCode::Escape => {
                self.sfx.play(Se::Cancel);
                if self.songs.back() {
                    self.view.jumped();
                } else {
                    // 上流と同じく一番上で Esc ならタイトルへ（v2 はすぐ切り替え、タイトルが白から出る）
                    self.sfx.preview(None);
                    self.go_to_title();
                }
            }
            // v2 と同じく ← → で並べ替え、R でランダムに選ぶ。難易度は Tab（パッドの HH 2 回）
            KeyCode::ArrowLeft => self.change_sort(-1),
            KeyCode::ArrowRight => self.change_sort(1),
            KeyCode::KeyR => self.pick_random(),
            KeyCode::Tab => self.cycle_level(),
            KeyCode::F5 => self.select_action(ui::SelectAction::Rescan),
            KeyCode::KeyP => self.select_action(ui::SelectAction::Practice),
            _ => {}
        }
    }

    /// ポップアップを開いているときのキー
    fn on_popup_key(&mut self, code: KeyCode) {
        let input = match code {
            KeyCode::ArrowUp => PopupInput::Up,
            KeyCode::ArrowDown => PopupInput::Down,
            KeyCode::ArrowLeft => PopupInput::Left,
            KeyCode::ArrowRight => PopupInput::Right,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => PopupInput::Decide,
            KeyCode::Escape | KeyCode::Backspace => PopupInput::Cancel,
            KeyCode::Comma | KeyCode::Period => {
                if let Some(p) = &mut self.popup {
                    p.setup
                        .nudge_speed(if code == KeyCode::Comma { -1 } else { 1 });
                    self.popup_response(practice_popup::Response {
                        se: Some(Se::Move),
                        done: None,
                    });
                }
                return;
            }
            _ => return,
        };
        self.popup_input(input);
    }

    /// 選曲と結果の画面での電子ドラムの操作（上流と同じ割り当て）。
    /// HT で上、LT で下、CY か RD で決定、LC で戻る、HH を 2 回で難易度
    fn on_menu_pad(&mut self, lane: DrumLane, at_us: i64) {
        if self.after_close.is_some() {
            return;
        }
        if self.quick.is_some() && matches!(self.screen, Screen::Select | Screen::Play(..)) {
            // 設定の画面と同じパッドの役目
            let input = match lane {
                DrumLane::HT => ConfigInput::Up,
                DrumLane::LT => ConfigInput::Down,
                DrumLane::SD => ConfigInput::Left,
                DrumLane::FT => ConfigInput::Right,
                DrumLane::CY | DrumLane::RD => ConfigInput::Decide,
                DrumLane::LC => ConfigInput::Cancel,
                _ => return,
            };
            self.quick_input(input);
            return;
        }
        match &self.screen {
            Screen::Title => match lane {
                DrumLane::HT => self.title_move(-1),
                DrumLane::LT => self.title_move(1),
                DrumLane::CY | DrumLane::RD => self.title_decide(),
                _ => {}
            },
            Screen::Config(..) => {
                // パッドの役目は設定の画面の下に出す（HT 上、LT 下、SD 左、FT 右、CY・RD 決定、LC 戻る）
                let input = match lane {
                    DrumLane::HT => ConfigInput::Up,
                    DrumLane::LT => ConfigInput::Down,
                    DrumLane::SD => ConfigInput::Left,
                    DrumLane::FT => ConfigInput::Right,
                    DrumLane::CY | DrumLane::RD => ConfigInput::Decide,
                    DrumLane::LC => ConfigInput::Cancel,
                    _ => return,
                };
                self.config_input(input);
            }
            Screen::End(_) => {}
            Screen::Select => {
                if self.popup.is_some() {
                    let input = match lane {
                        DrumLane::HT => PopupInput::Up,
                        DrumLane::LT => PopupInput::Down,
                        DrumLane::CY | DrumLane::RD => PopupInput::Decide,
                        DrumLane::LC => PopupInput::Cancel,
                        _ => return,
                    };
                    self.popup_input(input);
                    return;
                }
                match lane {
                    DrumLane::HT => self.move_cursor(-1),
                    DrumLane::LT => self.move_cursor(1),
                    // v2 と同じく SD と FT で並べ替え
                    DrumLane::SD => self.change_sort(-1),
                    DrumLane::FT => self.change_sort(1),
                    DrumLane::CY | DrumLane::RD => self.select_action(ui::SelectAction::Enter),
                    // 一番上で LC を叩いてもアプリは終わらない
                    DrumLane::LC => self.select_action(ui::SelectAction::Back),
                    DrumLane::HH | DrumLane::HHO => {
                        if self.last_hh_us.is_some_and(|t| at_us - t <= DOUBLE_HIT_US) {
                            self.last_hh_us = None;
                            self.cycle_level();
                        } else {
                            self.last_hh_us = Some(at_us);
                        }
                    }
                    // BD を 2 回で簡易設定
                    DrumLane::BD => {
                        if self.last_bd_us.is_some_and(|t| at_us - t <= DOUBLE_HIT_US) {
                            self.last_bd_us = None;
                            self.open_quick();
                        } else {
                            self.last_bd_us = Some(at_us);
                        }
                    }
                    _ => {}
                }
            }
            Screen::Result(..) => match lane {
                DrumLane::CY | DrumLane::RD | DrumLane::LC => {
                    self.result_action(ui::ResultAction::Back);
                }
                // 選曲と同じく HT と LT で動かす
                DrumLane::HT | DrumLane::LT => self.turn_result_page(),
                _ => {}
            },
            Screen::Loading(_) | Screen::Play(..) => {}
        }
    }

    /// 結果の画面のページを切り替える
    fn turn_result_page(&mut self) {
        if let Screen::Result(_, view) = &mut self.screen {
            view.turn();
            self.sfx.play(Se::Change);
        }
    }

    /// 難易度をひとつ上へ（v2 と同じく、BASIC〜MASTER はそれぞれの声で知らせる）
    fn cycle_level(&mut self) {
        let before = self.songs.level;
        self.songs.cycle_level();
        if self.songs.level != before {
            self.sfx.play(Se::level(self.songs.level));
        }
        self.remember_level();
    }

    /// 並べ替えを `delta` 個となりへ（v2 の ← → とパッドの SD・FT、アイコンを押す）
    fn change_sort(&mut self, delta: isize) {
        if delta == 0 {
            return;
        }
        let kind = self.songs.sort().step(delta);
        self.songs.set_sort(kind);
        self.view.sorted(delta);
        self.sfx.play(Se::Move);
        self.sfx.play(Se::Sort(kind));
    }

    /// 今いる BOX の中から 1 つを選ぶ（v2 の R）
    fn pick_random(&mut self) {
        if self.songs.len() == 0 {
            return;
        }
        self.songs.pick_random();
        self.view.jumped();
        self.sfx.play(Se::Move);
    }

    fn remember_level(&mut self) {
        if self.config.level != self.songs.level {
            self.config.level = self.songs.level;
            self.config.save();
        }
    }

    fn select_action(&mut self, action: ui::SelectAction) {
        use ui::SelectAction as A;
        match action {
            A::Enter => {
                let depth = self.songs.breadcrumbs().len();
                match self.songs.enter() {
                    Some(path) => {
                        // v2 と同じく、すぐ読み込みの画面へ切り替える
                        self.sfx.play(Se::Decide);
                        self.sfx.preview(None);
                        self.start(path);
                    }
                    None => {
                        self.view.jumped();
                        let left = self.songs.breadcrumbs().len() < depth;
                        self.sfx.play(if left { Se::Cancel } else { Se::Decide });
                    }
                }
            }
            A::Back => {
                if self.songs.back() {
                    self.view.jumped();
                    self.sfx.play(Se::Cancel);
                }
            }
            A::Rescan => {
                if !self.config.song_dirs.is_empty() {
                    self.songs.rescan(self.config.song_dirs.clone());
                }
            }
            A::Practice => {
                // 上流のソートのポップアップと同じく、選曲画面の上に開く
                if let Some(path) = self.songs.selected_chart() {
                    self.sfx.play(Se::Decide);
                    self.open_setup(path, None);
                }
            }
        }
    }

    fn result_action(&mut self, action: ui::ResultAction) {
        if self.after_close.is_some() {
            return;
        }
        let Screen::Result(o, _) = &self.screen else {
            return;
        };
        let path = o.path.clone();
        let options = o.practice.as_ref().map(|p| p.options);
        match action {
            ui::ResultAction::Back => {
                // 上流と同じく戻る音を鳴らして選曲へ（v2：シャッターを閉じてから）
                self.sfx.play(Se::Cancel);
                self.close_then(AfterClose::Select, Duration::ZERO);
            }
            ui::ResultAction::Retry => match options {
                Some(o) => self.start_practice(path, o),
                None => self.start(path),
            },
            ui::ResultAction::SetInputOffset(ms) => {
                self.config.input_offset_ms = ms;
                self.config.save();
            }
            ui::ResultAction::Practice(section) => {
                let previous = match section {
                    Some(section) => {
                        let p = self.config.practice_prefs();
                        Some(PracticeOptions {
                            section,
                            style: p.style,
                            rule: p.rule,
                            speed: p.speed,
                            pitch: p.pitch,
                        })
                    }
                    None => options,
                };
                self.close_then(AfterClose::Setup(path, previous), Duration::ZERO);
            }
        }
    }

    fn on_drop(&mut self, path: PathBuf) {
        // シャッターを閉じているあいだは次の画面が決まっているので、曲を始めない
        if self.after_close.is_some() {
            return;
        }
        if path.is_dir() {
            self.add_song_dir(path);
        } else if ui::is_chart_file(&path) {
            self.start(path);
        } else {
            self.message = Some(format!(
                "DTX ファイルか曲のフォルダをドロップしてください: {}",
                path.display()
            ));
        }
    }

    fn redraw(&mut self) {
        let (Some(window), Some(state)) = (self.window.clone(), &mut self.egui_state) else {
            return;
        };
        let input = state.take_egui_input(&window);
        let ports = self
            .midi
            .as_ref()
            .map(MidiIn::open_names)
            .unwrap_or_default();
        let mut mouse = SelectMouse::default();
        let mut result_actions = Vec::new();
        let toast = self
            .toast
            .as_ref()
            .filter(|(_, until)| Instant::now() < *until)
            .map(|(t, _)| t.clone());
        let scanning = self.songs.scanning().is_some();
        self.title.set_update(self.updater.ready());
        self.shutter.update(Instant::now());
        let egui_ctx = self.egui_ctx.clone();
        let output = egui_ctx.run_ui(input, |ui| {
            self.draw_screen(
                ui,
                &ports,
                &toast,
                scanning,
                &mut mouse,
                &mut result_actions,
            );
            // シャッターはどの画面のどの窓よりも上に
            let ctx = ui.ctx().clone();
            let top = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Debug,
                egui::Id::unique("shutter"),
            ));
            self.shutter
                .draw(&Canvas::overlay(&top, ctx.content_rect()), &mut self.gfx);
        });
        if let Some(state) = &mut self.egui_state {
            state.handle_platform_output(&window, output.platform_output.clone());
        }
        if let Some(gpu) = &mut self.gpu {
            gpu.render(&self.egui_ctx, output);
        }
        self.select_mouse(mouse);
        for a in result_actions {
            self.result_action(a);
        }
    }

    /// 今の画面を描く
    fn draw_screen(
        &mut self,
        ui: &mut egui::Ui,
        ports: &[String],
        toast: &Option<String>,
        scanning: bool,
        mouse: &mut SelectMouse,
        result_actions: &mut Vec<ui::ResultAction>,
    ) {
        match &mut self.screen {
            Screen::Title => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                self.title.draw(&c, &mut self.gfx, scanning);
            }
            Screen::Config(view, _) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                let info = config_info(&self.config, &self.midi, &self.monitor, &self.songs);
                let rows = config_view::rows(view.page(), &info);
                view.draw(&c, &mut self.gfx, &rows, &info);
                if let Some(m) = &self.message {
                    ui::message_overlay(&ctx, m);
                }
            }
            Screen::End(e) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                e.draw(&c, &mut self.gfx);
            }
            Screen::Select => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                self.view.draw(&c, &mut self.gfx, &self.songs);
                if let Some(p) = &self.popup {
                    p.draw(&c, &mut self.gfx);
                }
                if let Some(q) = &mut self.quick {
                    let mut info =
                        config_info(&self.config, &self.midi, &self.monitor, &self.songs);
                    info.song = self.songs.selected_chart().is_some();
                    let rows = config_view::rows(q.page(), &info);
                    q.draw_panel(&c, &mut self.gfx, &rows, &info);
                }
                if let Some(m) = &self.message {
                    ui::message_overlay(&ctx, m);
                }
                *mouse = SelectMouse::read(&ctx, &c);
            }
            Screen::Loading(l) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                l.draw(&c, &mut self.gfx);
            }
            Screen::Play(play, view) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                let now = view
                    .frozen_ms()
                    .unwrap_or_else(|| play.now_ms() + self.config.visual_offset_ms);
                view.draw(&c, &mut self.gfx, play, now, self.config.scroll_px_per_ms);
                ui::draw_play_overlay(
                    &ctx,
                    &c,
                    &ui::PlayOverlay {
                        play,
                        config: &self.config,
                        midi_ports: ports,
                        show_info: self.show_play_info,
                    },
                );
                if let Some(t) = toast {
                    ui::play_toast(&ctx, &c, t);
                }
                if let Some(q) = &mut self.quick {
                    let mut info =
                        config_info(&self.config, &self.midi, &self.monitor, &self.songs);
                    info.play = Some(PlayPos {
                        practice: play.practice.is_some(),
                        at_ms: play.now_ms(),
                        end_ms: play.end_ms,
                    });
                    let rows = config_view::rows(q.page(), &info);
                    q.draw_panel(&c, &mut self.gfx, &rows, &info);
                }
            }
            Screen::Result(o, view) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                view.draw_background(&c, &mut self.gfx);
                match view.page {
                    Page::Summary => view.draw_summary(&c, &mut self.gfx, o),
                    Page::Analysis => {
                        view.draw_analysis_hint(&c, &mut self.gfx);
                        *result_actions = ui::draw_result(ui, &mut self.gfx, o, &self.config);
                    }
                }
                // フェードは egui の窓より上に
                let top = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::unique("result fade"),
                ));
                view.draw_fade(&Canvas::overlay(&top, ctx.content_rect()));
            }
        }
    }
}

/// 設定の画面の項目を作るのに要るもの
fn config_info<'a>(
    config: &'a Config,
    midi: &Option<MidiIn>,
    monitor: &MidiMonitor,
    songs: &SongList,
) -> ConfigInfo<'a> {
    let ports = midi.as_ref().map_or(&[][..], |m| m.ports.as_slice());
    let profiles = inputs::profiles(config, ports);
    let devices = ports
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let d = inputs::device(config, ports, i);
            config_view::DeviceRow {
                name: p.name.clone(),
                id: p.id.clone(),
                open: p.open,
                error: p.error.clone(),
                disabled: d.is_some_and(|d| d.disabled),
                preset: inputs::preset_for(d, &p.name),
                default_preset: engine::input::Preset::detect(midi::base_name(&p.name)),
                channel: profiles[i].channel.map_or(0, |c| c + 1),
                min_velocity: profiles[i].min_velocity,
                changes: d.map_or(config.pad_notes.len(), |d| d.notes.len()),
                last: monitor.last(i).map(str::to_owned),
            }
        })
        .collect();
    ConfigInfo {
        config,
        lanes: inputs::lane_sources(config, ports),
        devices,
        scanning: songs.scanning(),
        song_count: songs.library.songs,
        song: false,
        play: None,
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("DTXmaniaNXT")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.startup_error = Some(anyhow::Error::new(e).context("窓を開けません"));
                event_loop.exit();
                return;
            }
        };
        match Gpu::new(window.clone()) {
            Ok(gpu) => self.gpu = Some(gpu),
            Err(e) => {
                self.startup_error = Some(e.context("GPU を使い始められません"));
                event_loop.exit();
                return;
            }
        }
        fonts::install_japanese(&self.egui_ctx);
        let max_texture_side = self
            .gpu
            .as_ref()
            .map(|g| g.device.limits().max_texture_dimension_2d as usize);
        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            max_texture_side,
        ));
        window.request_redraw();
        self.window = Some(window);
        if let Some(path) = self.pending.take() {
            self.start(path);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let (Some(window), Some(state)) = (&self.window, &mut self.egui_state) {
            let _ = state.on_window_event(window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                self.save_unfinished();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::DroppedFile(path) => self.on_drop(path),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && let PhysicalKey::Code(code) = event.physical_key
                {
                    // 押し続けたときの繰り返しは、設定の画面の矢印だけ（上流のキーの反復と同じ。左右は値を続けて変える）
                    let repeat_ok = matches!(self.screen, Screen::Config(..))
                        && matches!(
                            code,
                            KeyCode::ArrowUp
                                | KeyCode::ArrowDown
                                | KeyCode::ArrowLeft
                                | KeyCode::ArrowRight
                        );
                    if !event.repeat || repeat_ok {
                        self.on_key(code);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.pump();
                self.redraw();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
        if self.quit {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // 叩いた音をすぐ鳴らすため、描画を待たずに MIDI を処理する
        self.pump();
        // 描画の頼みはここでも出す。描画の中で次を頼むだけだと、描画の知らせが 1 度でも来なかったとき
        // 画面が止まったまま遊びだけ進む（頼みが重なっても描画は 1 回）
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        if self.quit {
            event_loop.exit();
        }
    }
}

/// 選曲画面で、この描画のあいだにあったマウスの操作
#[derive(Default)]
struct SelectMouse {
    /// ホイールを回した行数（上が正）
    wheel: f32,
    /// 押した曲の帯が、選んでいる曲から何本先か
    clicked_row: Option<isize>,
    /// 押した並べ替えのアイコンが、選んでいるものから何個先か
    clicked_sort: Option<isize>,
    /// 練習のポップアップへのマウスの操作
    popup: PopupMouse,
}

impl SelectMouse {
    fn read(ctx: &egui::Context, c: &Canvas) -> Self {
        ctx.input(|i| {
            let wheel = i
                .events
                .iter()
                .map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * 10.0,
                        egui::MouseWheelUnit::Point => delta.y / 40.0,
                    },
                    _ => 0.0,
                })
                .sum();
            let clicked = i
                .pointer
                .primary_clicked()
                .then(|| i.pointer.interact_pos())
                .flatten()
                .map(|p| c.to_nx(p));
            let clicked_row = clicked.and_then(SelectView::row_at);
            let clicked_sort = clicked.and_then(SelectView::sort_at);
            let popup = PopupMouse {
                pressed: i
                    .pointer
                    .primary_pressed()
                    .then(|| i.pointer.interact_pos())
                    .flatten()
                    .map(|p| c.to_nx(p)),
                down: i.pointer.primary_down(),
                hover: i.pointer.hover_pos().map(|p| c.to_nx(p)),
            };
            Self {
                wheel,
                clicked_row,
                clicked_sort,
                popup,
            }
        })
    }
}

fn main() -> ExitCode {
    // インストールなどで Velopack から呼ばれたときは、ここで仕事をして終わる
    // （ログのファイルより先に。前回の遊んだときのログを押し出さないように）
    update::run_hooks();
    let log = logging::init();
    // 主のスレッドの panic は、ログに書いたうえで（`logging`）ここで受け、終わることを知らせる
    let error = match std::panic::catch_unwind(run) {
        Ok(Ok(())) => return ExitCode::SUCCESS,
        Ok(Err(e)) => {
            log::error!("{e:#}");
            format!("{e:#}")
        }
        Err(_) => logging::main_panic().unwrap_or_else(|| "panic".to_owned()),
    };
    logging::tell_user(&error, log.as_deref());
    ExitCode::FAILURE
}

fn run() -> Result<()> {
    let _ = host_us();
    let arg = std::env::args_os().nth(1).map(PathBuf::from);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(arg);
    event_loop.run_app(&mut app)?;
    app.updater.apply_on_exit(app.config.auto_update);
    match app.startup_error.take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
