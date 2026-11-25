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

mod app_icon;
mod audio;
mod band;
mod capture;
mod chart_picture;
mod chart_sound;
mod config;
mod config_view;
mod credits;
mod drill;
mod drill_hud;
mod drill_menu;
mod drop_view;
mod editor;
mod fonts;
mod frames;
mod game;
mod gfx;
mod history;
mod inputs;
mod jobs;
mod lag_view;
mod lane_layout;
mod loading;
mod logging;
mod midi;
mod module_link;
mod module_probe;
mod movie;
mod nx_import;
mod perf;
mod play_view;
mod practice;
mod practice_hud;
mod practice_menu;
mod progress;
mod records_view;
mod replay;
mod replay_view;
mod result_drill;
mod result_training;
mod result_view;
mod rim_look;
mod section_sound;
mod select;
mod settings;
mod settings_ui;
mod sfx;
mod shutter;
mod skin;
mod song_adder;
mod songs;
mod stretch;
mod takeover;
mod text;
mod time;
mod title_view;
mod ui;
mod update;
mod update_view;
mod window_mode;
mod xa;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dtx::DrumLane;
use engine::input::{InputEvent, MidiMessage, MidiProfile, Normalizer};
use i18n::{t, tf};
use inputs::MidiMonitor;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::chart_sound::AutoMix;
use crate::config::Config;
use crate::config_view::{
    ConfigAction, ConfigInfo, ConfigInput, ConfigView, Drill, ItemId, PlayPos, Setting,
};
use crate::game::{GaugeRule, HitSoundRule, Outcome, Play, Rules};
use crate::gfx::{Canvas, Gfx};
use crate::history::History;
use crate::lag_view::LagResult;
use crate::loading::{Job, Loading};
use crate::midi::MidiIn;
use crate::play_view::PlayView;
use crate::practice::{PracticeOptions, PracticeSetup};
use crate::practice_menu::{Done, Finished, MenuInput, MenuMouse, PracticeMenu, Spot};
use crate::records_view::{RecordsAction, RecordsFrom, RecordsInput, RecordsView};
use crate::replay_view::{Back, ListDone, ListInput, ReplayList, ReplayRun};
use crate::result_view::{Page, ResultInfo, ResultView};
use crate::section_sound::SectionAudio;
use crate::select::{PREVIEW_WAIT, SelectView};
use crate::settings::{DisplayMode, Vsync};
use crate::sfx::{Se, Sfx};
use crate::shutter::Shutter;
use crate::songs::SongList;
use crate::time::host_us;
use crate::title_view::{Ending, TitleItem, TitleView};
use crate::update_view::Corner;
use crate::window_mode::MonitorPick;

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    egui_renderer: egui_wgpu::Renderer,
    /// 描く画像を続けて受け取れなかった回数
    timeouts: u32,
    /// 前のフレームで、描く画像（画面の空き）を待った時間
    waited: Duration,
    /// 次に描く画面を PNG にして残す先（F12 と、記録を更新した結果の画面）
    capture: Option<PathBuf>,
    /// この GPU と窓で使える出し方（垂直同期の低遅延が使えるか）
    present_modes: Vec<wgpu::PresentMode>,
}

/// 描く画像をこの回数続けて受け取れなければ、画面への出力を作り直す
/// （1 回は 1 秒待つ。そのままにすると画面が止まったまま遊びだけ進む）
const TIMEOUTS_BEFORE_RECONFIGURE: u32 = 3;

impl Gpu {
    fn new(window: Arc<Window>, vsync: Vsync) -> Result<Self> {
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).context(t(
            "Cannot prepare the window for drawing (surface)",
            "窓に描く準備（サーフェス）ができません",
        ))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .context(t("No usable GPU found", "使える GPU が見つかりません"))?;
        log::info!("GPU: {:?}", adapter.get_info().name);
        // NX のスキンには縦 4080 の画像などがあるので、GPU が扱える一番大きい画像の大きさを使う
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .context(t("Cannot set up the GPU", "GPU を使う準備ができません"))?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context(t(
                "This GPU cannot draw to the window",
                "この GPU では窓に描けません",
            ))?;
        // egui は sRGB でない形式を前提に色を作るので、選べるならそちらにする
        let caps = surface.get_capabilities(&adapter);
        if let Some(f) = caps.formats.iter().find(|f| !f.is_srgb()) {
            config.format = *f;
        }
        // 遅れを小さくするため、待ち行列の深さを最小にする
        config.desired_maximum_frame_latency = 1;
        config.present_mode = present_mode(vsync, &caps.present_modes);
        log::info!("present modes: {:?}", caps.present_modes);
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
            waited: Duration::ZERO,
            capture: None,
            present_modes: caps.present_modes,
        })
    }

    /// 垂直同期（上流の VSyncWait）を切り替える
    fn set_vsync(&mut self, vsync: Vsync) {
        let mode = present_mode(vsync, &self.present_modes);
        if self.config.present_mode != mode {
            self.config.present_mode = mode;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// 描く。画面を残したら、その先を返す
    fn render(&mut self, ctx: &egui::Context, mut output: egui::FullOutput) -> Option<PathBuf> {
        // 字形などのテクスチャは、画面を描けないフレームでも必ず反映する（捨てると文字が出ない）
        let mut textures = std::mem::take(&mut output.textures_delta);
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, id, delta);
            }
        }
        let free: Vec<egui::TextureId> = textures.free.drain().collect();
        let saved = self.draw(ctx, output.shapes, output.pixels_per_point);
        for id in &free {
            self.egui_renderer.free_texture(id);
        }
        saved
    }

    fn draw(
        &mut self,
        ctx: &egui::Context,
        shapes: Vec<egui::epaint::ClippedShape>,
        ppp: f32,
    ) -> Option<PathBuf> {
        let asked = Instant::now();
        let got = self.surface.get_current_texture();
        self.waited = asked.elapsed();
        let frame = match got {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            wgpu::CurrentSurfaceTexture::Occluded => return None,
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.timeouts += 1;
                if self.timeouts >= TIMEOUTS_BEFORE_RECONFIGURE {
                    log::warn!("no frame for {} tries; reconfiguring", self.timeouts);
                    self.surface.configure(&self.device, &self.config);
                    self.timeouts = 0;
                }
                return None;
            }
            other => {
                log::warn!("skip frame: {other:?}");
                self.surface.configure(&self.device, &self.config);
                return None;
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
        // 画面を残すときは、同じ絵を読み出せる画像にも描く（窓の画像は読み出せないことがある）
        let capture = self.capture.take().map(|path| {
            let shot = capture::Shot::new(
                &self.device,
                self.config.format,
                self.config.width,
                self.config.height,
            );
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("capture"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: shot.view(),
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                let mut pass = pass.forget_lifetime();
                self.egui_renderer.render(&mut pass, &jobs, &screen);
            }
            shot.copy(&mut encoder);
            (shot, path)
        });
        self.queue
            .submit(extra.into_iter().chain([encoder.finish()]));
        self.queue.present(frame);
        let (shot, path) = capture?;
        shot.save(&self.device, path.clone());
        Some(path)
    }
}

/// 垂直同期の選び方に当たる出し方（選べなければ wgpu が近いものにする）。
/// 低遅延は Mailbox（モニターに合わせたまま、待たずに一番新しい絵を出す）で、使えなければ ON と同じ
fn present_mode(vsync: Vsync, available: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    match vsync {
        Vsync::LowLatency if available.contains(&wgpu::PresentMode::Mailbox) => {
            wgpu::PresentMode::Mailbox
        }
        Vsync::On | Vsync::LowLatency => wgpu::PresentMode::AutoVsync,
        Vsync::Off => wgpu::PresentMode::AutoNoVsync,
    }
}

/// 曲を読むのに使う設定。読むスレッドへ渡せるように、設定から写しておく
struct LoadSettings {
    input_offset_ms: f64,
    /// この曲の補正（記録から）
    song_offset_ms: f64,
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
        p.song_offset_ms = self.song_offset_ms;
        // 「叩いても音が出ない」と言われたとき、ログで分かるように
        log::info!(
            "loaded in {:.0}ms (hit sounds {})",
            started.elapsed().as_secs_f64() * 1000.0,
            if self.drum_sounds { "on" } else { "off" }
        );
        Ok(p)
    }
}

/// 補正の行に出す、この曲の補正とおすすめ（簡易設定・一時停止・設定の画面を開いたときに記録から出す）
struct SongTiming {
    /// この曲の譜面（難易度すべて）。曲を選んでいなければ空
    charts: Vec<PathBuf>,
    /// この曲の補正（ミリ秒）
    offset_ms: f64,
    advice: engine::lag::Advice,
}

impl SongTiming {
    fn info(&self) -> config_view::TimingInfo {
        config_view::TimingInfo {
            song_ms: (!self.charts.is_empty()).then(|| self.offset_ms.round() as i64),
            advice: self.advice,
        }
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
    /// 選曲（練習メニューは App の `menu`）
    Select,
    /// 曲を読み込んでいる
    Loading(Box<Loading>),
    /// 遊んでいる（練習中も）。NX の演奏画面の動きと一緒に持つ
    Play(Box<Play>, Box<PlayView>),
    /// 結果。NX の結果の画面の動きと一緒に持つ
    Result(Box<Outcome>, Box<ResultView>),
    /// 記録したプレイのリプレイ
    Replay(Box<ReplayRun>),
    /// 基礎練習の画面（タイトルの Drills から）
    Drills(Box<drill_menu::DrillMenu>),
    /// 上達の記録
    Records(Box<RecordsView>),
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
    /// 選曲画面に重ねる、練習メニュー
    menu: Option<Box<PracticeMenu>>,
    /// 練習メニューに出した曲と苦手な所（練習から戻ったとき、同じ並びで出す）
    menu_spots: Option<(PathBuf, Vec<Spot>)>,
    /// 練習メニューで選んでいる所の音と、その具合（メニューに出す）
    section_audio: SectionAudio,
    menu_sound: Option<practice_menu::Sound>,
    /// 選曲の簡易設定か、演奏中の一時停止の板（設定の画面と同じ行）
    quick: Option<Box<ConfigView>>,
    /// 選曲画面に重ねる、選んでいる曲のプレイの一覧（リプレイ）
    replays: Option<Box<ReplayList>>,
    /// 補正の行に出す、この曲の補正とおすすめ
    timing: Option<SongTiming>,
    /// 選曲画面のマウスホイールの、まだ 1 行に満たない分
    wheel: f32,
    /// 演奏中に設定と入力の情報を出す（I で切り替え）
    show_play_info: bool,
    /// 演奏中の短い知らせ（消す時刻）
    toast: Option<(String, Instant)>,
    /// ドロップされた zip や曲のフォルダを、曲のフォルダへ写す係
    adder: song_adder::Adder,
    /// 続けてドロップされた分の結果（全部終わったら 1 つの知らせにまとめ、曲を探し直す）
    added: Vec<song_adder::Outcome>,
    /// 探し直したあと選曲で移る、追加した曲（譜面か曲のフォルダ）
    jump_to: Option<PathBuf>,
    /// 追加したあとの探し直しを頼んだ（探し終えたら `reveal_ready`）
    rescanning_for_add: bool,
    /// 追加したあとの探し直しが終わった。選曲（かタイトル）にいれば `jump_to` へ移る
    reveal_ready: bool,
    /// ウィンドウに重ねているファイル（ドロップする前。重ねているあいだ画面に案内を出す）
    drop_hover: Vec<PathBuf>,
    /// 音符をすべて自動で叩く（開発用。`DTXMANIANXT_AUTOPLAY=1`。設定の自動演奏より優先）
    autoplay: bool,
    /// 新しい版を確かめ、頼まれたら落とし、終えるときか再起動で入れ替える
    updater: update::Updater,
    /// 「再起動して入れ替える」を選んだ（入れ替える版と時刻）。「更新しています」を出してから閉じる
    applying: Option<(String, Instant)>,
    /// 更新の印の回る輪の起点
    update_since: Instant,
    /// FPS と遅延の表示（設定か F9）
    perf: perf::PerfView,
    /// 押している Alt などのキー（Alt+Enter で全画面）
    modifiers: winit::keyboard::ModifiersState,
    /// 窓に当てた画面の出し方・垂直同期・出すモニター・ウィンドウの大きさ（変わったら当て直す）
    applied_window: Option<WindowWant>,
    /// ウィンドウのときの位置と大きさ（全画面やボーダーレスから戻るときに使う）
    window_placement: Option<window_mode::Placement>,
    /// ボーダーレスの窓をモニターに合わせ直す（DPI の違うモニターへ移ると、winit が窓の大きさを変えるので）
    refit_window: bool,
    /// つながっているモニター（設定の画面の「出すモニター」の選択肢。設定の画面にいるあいだ時々調べ直す）
    monitor_list: Vec<window_mode::MonitorInfo>,
    monitors_checked: Instant,
    /// 窓がキーボードを受けている（ほかの窓を見ているあいだ描く回数を下げるか決める）
    focused: bool,
    /// フレームレートに上限があるとき、次に描く時刻
    next_frame: Instant,
    /// 他の DTXMania からの引き継ぎ（見つけたもの、読んだ設定）
    import: takeover::Import,
}

/// 窓に当てる設定
#[derive(Debug, Clone, PartialEq)]
struct WindowWant {
    mode: DisplayMode,
    vsync: Vsync,
    monitor: Option<MonitorPick>,
    /// ウィンドウの大きさ（前回のままなら `None`）
    size: Option<(u32, u32)>,
}

impl WindowWant {
    fn of(config: &Config) -> Self {
        let v = &config.settings;
        Self {
            mode: v.display_mode(),
            vsync: v.vsync(),
            monitor: config.monitor.clone(),
            size: v.window_size(),
        }
    }
}

/// ほかの窓を見ているあいだ（下げる設定のとき）の、1 秒に描く回数
const BACKGROUND_FPS: u32 = 30;

/// 1 フレームの間（上限が無ければ `None`）。設定の上限 `limit` と、ほかの窓を見ているあいだ下げるとき
/// （`background`。演奏と練習の最中は呼ぶ側で false にする）の低い方
fn frame_interval(limit: Option<u32>, background: bool) -> Option<Duration> {
    let fps = match (limit, background) {
        (Some(f), true) => Some(f.min(BACKGROUND_FPS)),
        (None, true) => Some(BACKGROUND_FPS),
        (f, false) => f,
    };
    fps.map(|f| Duration::from_secs_f64(1.0 / f64::from(f.max(1))))
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
/// 曲を追加した（できなかった）知らせを出しておく時間
const ADD_TOAST_FOR: Duration = Duration::from_secs(5);
/// 譜面の音を切っているときに、演奏の始めに知らせを出しておく時間
const HIT_SOUND_NOTICE_FOR: Duration = Duration::from_secs(4);

/// 演奏中に譜面の音（叩いた音）を切り替えるキー。キーボードで叩くキー（A〜L、Q、Z）から離しておく
/// （前の M は J・K のすぐ下で、叩くつもりで押すと気づかないうちに切れてしまう）
const HIT_SOUND_KEY: KeyCode = KeyCode::F2;
/// [`HIT_SOUND_KEY`] の表示の名前
const HIT_SOUND_KEY_NAME: &str = "F2";

/// FPS と遅延の表示を切り替えるキー（どの画面でも。出さない → 小さく → 詳しく）
const MONITOR_KEY: KeyCode = KeyCode::F9;

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
    /// 結果の画面や一時停止の板から練習メニューへ（選曲画面に開く）。今の決めごとと、終えた練習
    Setup(PathBuf, Option<PracticeOptions>, Option<Finished>),
    /// 結果の画面かプレイの一覧から、記録したプレイ（セッションの番号）のリプレイへ
    Replay(i64),
    /// リプレイを閉じて、来た画面へ
    LeaveReplay,
    /// 基礎練習の画面へ
    Drills,
    /// 基礎練習を始める
    Drill(engine::drill::Run),
    /// 上達の記録へ。どこから開いたかと、選んでおく譜面
    Records(RecordsFrom, Option<PathBuf>),
}

/// リプレイの一覧に出すプレイの数
const REPLAY_LIST_LIMIT: usize = 50;

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

/// 「再起動して入れ替える」を選んでから閉じるまで、「更新しています」を出しておく時間
const APPLY_SHOW: Duration = Duration::from_millis(1500);

/// MIDI の機器がつながったか外れたかを調べる間
const PORT_CHECK_EVERY: Duration = Duration::from_secs(2);

/// 設定の画面にいるあいだ、モニターがつながったか外れたかを調べる間
const MONITOR_CHECK_EVERY: Duration = Duration::from_secs(2);

/// 設定の音量（全体と、ミキサーの系統ごと）を出力に渡す。鳴っている音にもすぐ効く
fn apply_volumes(config: &Config) {
    audio::set_master_volume(config.master_volume());
    for (bus, key) in audio::Bus::ALL.into_iter().zip(settings::VOLUMES) {
        audio::set_bus_volume(bus, config.settings.number(key) as u8);
    }
}

/// 2 回叩きと見なす間（上流と同じ 0.5 秒）
const DOUBLE_HIT_US: i64 = 500_000;

impl App {
    fn new(arg: Option<PathBuf>) -> Self {
        let mut config = Config::load();
        i18n::set_lang(config.lang());
        if config.take_over_nx() {
            config.save();
        }
        let midi = open_midi(&config);
        // 最初の音（タイトルの音）から設定の音量で鳴らす
        apply_volumes(&config);
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
        let updater = update::Updater::start(config.update_mode());
        let mut title = TitleView::new();
        // 前と違う版で起動したら（更新したら）、タイトルで知らせる
        if let Some(previous) = config.remember_version(update::current_version()) {
            config.save_unless_broken();
            title.set_updated(update::current_version(), previous);
        }
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
            title,
            quit: false,
            startup_error: None,
            shutter: Shutter::new(),
            after_close: None,
            menu: None,
            menu_spots: None,
            section_audio: SectionAudio::default(),
            menu_sound: None,
            quick: None,
            replays: None,
            timing: None,
            wheel: 0.0,
            show_play_info: false,
            toast: None,
            adder: song_adder::Adder::default(),
            added: Vec::new(),
            jump_to: None,
            rescanning_for_add: false,
            reveal_ready: false,
            drop_hover: Vec::new(),
            autoplay: std::env::var_os("DTXMANIANXT_AUTOPLAY").is_some_and(|v| v == "1"),
            updater,
            applying: None,
            update_since: Instant::now(),
            perf: perf::PerfView::default(),
            modifiers: winit::keyboard::ModifiersState::empty(),
            applied_window: None,
            window_placement: None,
            refit_window: false,
            monitor_list: Vec::new(),
            monitors_checked: Instant::now(),
            focused: true,
            next_frame: Instant::now(),
            import: takeover::Import::start(),
        }
    }

    /// FPS と遅延の表示を次の大きさにし、設定に残す（F9）
    fn cycle_monitor(&mut self) {
        let next = perf::Mode::from_index(self.config.settings.choice(settings::MONITOR)).next();
        self.config
            .settings
            .set_raw(settings::MONITOR, next.index() as i64);
        self.config.save();
        self.perf.set_mode(next);
        log::info!("monitor: {next:?}");
    }

    /// FPS と遅延の表示へ、今の音の出力の遅れと、演奏で叩いた時刻を渡す
    fn feed_perf(&mut self) {
        let (output, hits) = match &mut self.screen {
            Screen::Play(play, _) => (
                play.audio.as_ref().and_then(audio::AudioOut::latency),
                play.take_hit_timings(),
            ),
            _ => (self.sfx.latency(), Vec::new()),
        };
        self.perf.feed(output, hits);
    }

    /// 画面の出し方・垂直同期・出すモニター・ウィンドウの大きさを設定に合わせる（設定の画面で変えたときも、ここで当てる）
    fn apply_window_settings(&mut self) {
        let want = WindowWant::of(&self.config);
        if self.applied_window.as_ref() == Some(&want) && !self.refit_window {
            return;
        }
        if let Some(w) = &self.window {
            let target = window_mode::target(w, want.monitor.as_ref());
            match &self.applied_window {
                // 出し方は同じで、モニターが変わったか、DPI が変わって合わせ直す
                Some(a) if a.mode == want.mode => {
                    if a.monitor != want.monitor || self.refit_window {
                        window_mode::move_to(w, want.mode, target.as_ref());
                    }
                }
                applied => window_mode::apply(
                    w,
                    applied.as_ref().map(|a| a.mode),
                    want.mode,
                    &mut self.window_placement,
                    target.as_ref(),
                ),
            }
            // ウィンドウの大きさを選んだ。ウィンドウならその場で、ほかはウィンドウに戻ったときに
            let resized = self
                .applied_window
                .as_ref()
                .is_some_and(|a| a.size != want.size);
            if let (true, Some(size)) = (resized, want.size) {
                if want.mode == DisplayMode::Window {
                    window_mode::resize(w, size);
                } else if let Some(p) = &mut self.window_placement {
                    (p.w, p.h, p.maximized) = (size.0, size.1, false);
                }
            }
        }
        self.refit_window = false;
        if let Some(gpu) = &mut self.gpu {
            gpu.set_vsync(want.vsync);
        }
        let mode = want.mode;
        self.applied_window = Some(want);
        if self.config.remember_full_mode(mode) {
            self.config.save();
        }
    }

    /// ウィンドウの位置と大きさを、次に起動したときのために残す（終わるとき）
    fn remember_window(&mut self) {
        let Some(w) = &self.window else {
            return;
        };
        let now = if self.config.settings.display_mode() == DisplayMode::Window {
            window_mode::Placement::of(w)
        } else {
            self.window_placement
        };
        if let Some(p) = now
            && self.config.window != Some(p)
        {
            self.config.window = Some(p);
            self.config.save();
        }
    }

    /// 今の 1 フレームの間（上限が無ければ `None`）
    fn frame_interval(&self) -> Option<Duration> {
        let v = &self.config.settings;
        let playing = matches!(
            self.screen,
            Screen::Play(..) | Screen::Loading(..) | Screen::Replay(_)
        );
        frame_interval(
            v.fps_limit(),
            !self.focused && !playing && v.background_low(),
        )
    }

    /// 描き終えた。上限があれば次に描く時刻を決め、無ければすぐ次を頼む
    fn after_frame(&mut self) {
        match self.frame_interval() {
            None => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            Some(iv) => {
                let now = Instant::now();
                let next = self.next_frame + iv;
                // 遅れたら追いつこうとせず、今から数え直す
                self.next_frame = if next > now { next } else { now };
            }
        }
    }

    /// つながっているモニターを調べ直す（設定の画面の選択肢。`now` でなければ間をあける）
    fn refresh_monitors(&mut self, now: bool) {
        if !now && self.monitors_checked.elapsed() < MONITOR_CHECK_EVERY {
            return;
        }
        self.monitors_checked = Instant::now();
        if let Some(w) = &self.window {
            self.monitor_list = window_mode::monitors(w);
        }
    }

    /// Alt+Enter と F11：ウィンドウと、最後に使った全画面の出し方（全画面かボーダーレス）を行き来して、
    /// 設定にも残す（上流と同じく次に開いたときもそのまま）
    fn toggle_fullscreen(&mut self) {
        let next = match self.config.settings.display_mode() {
            DisplayMode::Window => self.config.full_mode(),
            _ => DisplayMode::Window,
        };
        self.config.settings.set_display_mode(next);
        self.config.save();
        self.apply_window_settings();
    }

    /// F12：次に描く画面を「ピクチャ」に残す（上流の Capture キー）
    fn request_screenshot(&mut self) {
        let (Some(gpu), Some(path)) = (&mut self.gpu, capture::screenshot_path()) else {
            return;
        };
        gpu.capture = Some(path);
    }

    /// 記録を更新した結果の画面を、譜面の隣に残す（上流の AutoSaveResult）
    fn capture_new_record(&mut self) {
        let Screen::Result(o, view) = &mut self.screen else {
            return;
        };
        if !self.config.settings.flag(settings::AUTO_SAVE_RESULT) {
            return;
        }
        if let (Some(rank), Some(gpu)) = (view.record_capture_due(), &mut self.gpu) {
            gpu.capture = Some(capture::result_path(&o.path, rank));
        }
    }

    /// 選曲画面に戻る。v2 と同じく、入るたびにシャッターを開く（閉じていなければ閉じた形から）
    fn go_to_select(&mut self) {
        self.sfx.stop_title();
        self.menu = None;
        self.quick = None;
        self.replays = None;
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
        let chart = if from_select {
            self.songs.selected_chart()
        } else {
            None
        };
        self.load_timing(chart.as_deref());
        self.refresh_monitors(true);
        // タイトルで「見つかりました」と知らせていたら、引き継ぐ画面から始める（一度だけ）
        let view = if !from_select && self.title.found() {
            self.title.set_found(None);
            self.config.import_offered = true;
            self.config.save();
            let cursor = self.app_row(ItemId::Import);
            ConfigView::starting_at(config_view::Cat::App, cursor, config_view::Page::Import)
        } else {
            ConfigView::new()
        };
        self.screen = Screen::Config(Box::new(view), from_select);
        self.shutter.open(CONFIG_OPEN_DELAY);
    }

    /// 設定の「アプリ」の一覧で `id` の行が何行目か（無ければ 0）
    fn app_row(&self, id: ItemId) -> usize {
        let mut info = config_info(
            &self.config,
            &self.midi,
            &self.monitor,
            &self.songs,
            &self.monitor_list,
        );
        info.import = Some(self.import.info());
        config_view::rows(config_view::Page::Cat(config_view::Cat::App), &info)
            .iter()
            .position(|r| r.id == Some(id))
            .unwrap_or(0)
    }

    /// 他の DTXMania の設定を読んで、引き継ぐ項目の一覧を開く
    fn import_open(&mut self, source: &nx::Source) {
        log::info!(
            "read {} ({:?}) at {}",
            source.title(),
            source.family,
            source.found.config.display()
        );
        let plan = takeover::plan(source, &self.config, self.ports());
        self.import.plan = Some(plan);
        self.import.error = None;
        if let Screen::Config(view, _) = &mut self.screen {
            view.open_page(config_view::Page::ImportItems);
        }
    }

    /// 指定したもの（ショートカット・exe・フォルダ・設定のファイル）から読む
    fn import_from_path(&mut self, path: &std::path::Path) {
        match takeover::open(path) {
            Ok(source) => self.import_open(&source),
            Err(e) => {
                log::info!("cannot import from {}: {e}", path.display());
                self.import.error = Some(e);
                if let Screen::Config(view, _) = &mut self.screen
                    && view.page() != config_view::Page::Import
                {
                    view.open_page(config_view::Page::Import);
                }
            }
        }
    }

    /// 選んだ項目を引き継ぐ（前の設定を残してから）
    fn import_apply(&mut self) {
        let Some(plan) = self.import.plan.take() else {
            return;
        };
        if plan.selected() == 0 {
            self.import.plan = Some(plan);
            self.config_toast(t("Nothing is checked", "引き継ぐ項目を選んでいません").to_owned());
            return;
        }
        if !takeover::save_backup(&self.config) {
            self.import.plan = Some(plan);
            self.config_toast(
                t(
                    "Couldn't save your current settings, so nothing was imported",
                    "今の設定を残せなかったので、引き継ぎませんでした",
                )
                .to_owned(),
            );
            return;
        }
        let ports = self.ports().to_vec();
        let applied = takeover::apply(&plan, &mut self.config, &ports);
        self.config.import_offered = true;
        self.config.save();
        log::info!("imported {} items from {}", applied.count, plan.title);
        apply_volumes(&self.config);
        if applied.pads {
            self.reopen_midi();
        }
        if applied.songs {
            self.songs.rescan(self.config.song_dirs.clone());
        }
        self.import.backup = takeover::backup_time();
        let cursor = self.app_row(ItemId::ImportUndo);
        if let Screen::Config(view, _) = &mut self.screen {
            view.back_to_category(cursor);
        }
        self.config_toast(tf!(
            "Imported {} items from {}",
            "{1} から {0} 項目を引き継ぎました",
            applied.count,
            plan.product
        ));
    }

    /// 引き継ぐ前の設定に戻す
    fn import_undo(&mut self) {
        let Some(before) = takeover::load_backup() else {
            self.import.backup = None;
            self.config_toast(
                t(
                    "Couldn't read the settings from before the import",
                    "引き継ぐ前の設定を読めませんでした",
                )
                .to_owned(),
            );
            return;
        };
        let songs_changed = before.song_dirs != self.config.song_dirs;
        // 窓の位置と大きさは今のまま（引き継ぎでは変えないので、戻すと窓の覚えだけが古くなる）
        let window = self.config.window;
        self.config = before;
        self.config.window = window;
        self.config.import_offered = true;
        self.config.save();
        takeover::remove_backup();
        self.import.backup = None;
        i18n::set_lang(self.config.lang());
        apply_volumes(&self.config);
        self.updater.set_mode(self.config.update_mode());
        self.reopen_midi();
        if songs_changed {
            self.songs.rescan(self.config.song_dirs.clone());
        }
        self.config_toast(
            t(
                "Put your settings back to before the import",
                "引き継ぐ前の設定に戻しました",
            )
            .to_owned(),
        );
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
            AfterClose::Select => {
                self.close_menu();
                self.go_to_select();
            }
            AfterClose::Title => {
                self.go_to_title();
                self.shutter.open(CONFIG_LEAVE_OPEN_DELAY);
            }
            AfterClose::Config(from_select) => self.open_config(from_select),
            AfterClose::Result { at_ms } => {
                self.finish_play(Some(at_ms));
                self.shutter.open(shutter::OPEN_DELAY);
            }
            AfterClose::Setup(path, previous, finished) => {
                self.open_setup(path, previous, finished)
            }
            AfterClose::Replay(session) => self.open_replay(session),
            AfterClose::LeaveReplay => self.leave_replay(),
            AfterClose::Records(from, focus) => self.open_records(from, focus.as_deref()),
            AfterClose::Drills => self.open_drills(),
            AfterClose::Drill(run) => {
                self.start_drill(run);
                self.shutter.open(shutter::OPEN_DELAY);
            }
        }
    }

    /// 上達の記録の画面を開く（v2 の設定と同じく、シャッターを開いて入る）
    fn open_records(&mut self, from: RecordsFrom, focus: Option<&Path>) {
        self.sfx.preview(None);
        self.quick = None;
        self.close_menu();
        let data = self.history.progress();
        let view = RecordsView::new(from, data, &self.songs, focus);
        self.screen = Screen::Records(Box::new(view));
        self.shutter.open(CONFIG_OPEN_DELAY);
    }

    /// 上達の記録の画面の操作（シャッターが開ききるまでは受け付けない）
    fn records_input(&mut self, input: RecordsInput) {
        if self.shutter.animating() {
            return;
        }
        let Screen::Records(view) = &mut self.screen else {
            return;
        };
        let from = view.from;
        match view.input(input) {
            RecordsAction::None => {}
            RecordsAction::Moved => self.sfx.play(Se::Move),
            RecordsAction::Changed => self.sfx.play(Se::Change),
            RecordsAction::Pick(path) => {
                // 選曲で、この譜面の曲と難易度を選んでおく（一覧に無ければ今の場所のまま）
                self.sfx.play(Se::Decide);
                if !self.songs.reveal(&path) {
                    log::info!("not in the song list: {}", path.display());
                }
                self.close_then(AfterClose::Select, Duration::ZERO);
            }
            RecordsAction::Back => {
                self.sfx.play(Se::Cancel);
                let next = match from {
                    RecordsFrom::Title => AfterClose::Title,
                    RecordsFrom::Select => AfterClose::Select,
                };
                self.close_then(next, Duration::ZERO);
            }
        }
    }

    /// 基礎練習の画面を開く（遊んでいる途中なら、そこまでの記録を残してから）。カーソルとテンポは前のまま
    fn open_drills(&mut self) {
        self.save_unfinished();
        self.sfx.stop_title();
        self.sfx.preview(None);
        self.quick = None;
        self.message = None;
        let menu =
            drill_menu::DrillMenu::new(&self.config.drill_prefs(), self.history.drill_records());
        self.screen = Screen::Drills(Box::new(menu));
        self.shutter.open(CONFIG_OPEN_DELAY);
    }

    /// 基礎練習の画面の操作
    fn drills_input(&mut self, input: drill_menu::Input) {
        let Screen::Drills(m) = &mut self.screen else {
            return;
        };
        let r = m.input(input);
        let prefs = r.changed.then(|| m.prefs());
        if let Some(se) = r.se {
            self.sfx.play(se);
        }
        if let Some(p) = prefs {
            self.config.set_drill_prefs(p);
            self.config.save();
        }
        match r.done {
            Some(drill_menu::Done::Start(run)) => {
                self.close_then(AfterClose::Drill(run), Duration::ZERO);
            }
            Some(drill_menu::Done::Back) => self.close_then(AfterClose::Title, Duration::ZERO),
            None => {}
        }
    }

    /// 基礎練習の譜面を作る（すぐできるので、読み込みの画面は出さない）
    fn load_drill(&self, run: engine::drill::Run) -> Result<Play> {
        let path = drill::chart_path(run.drill);
        let s = self.load_settings(&path);
        let mut p = Play::load_drill(run, s.input_offset_ms, s.drum_sounds, &s.rules)?;
        p.set_inputs(s.inputs);
        Ok(p)
    }

    /// 基礎練習を始める（やり直しも）。遊んでいる途中なら、そこまでの記録を残す
    fn start_drill(&mut self, run: engine::drill::Run) {
        self.save_unfinished();
        self.quick = None;
        // 前の演奏の音を止めてから作る
        self.screen = Screen::Title;
        match self.load_drill(run) {
            Ok(p) => self.enter_play(p),
            Err(e) => {
                log::error!("{e:#}");
                self.open_drills();
                self.message = Some(tf!("Could not start: {e:#}", "始められませんでした: {e:#}"));
            }
        }
    }

    /// 演奏中の曲（練習と基礎練習も）を、同じ決めごとではじめからやり直す
    fn restart_current(&mut self) {
        let Screen::Play(play, _) = &self.screen else {
            return;
        };
        if let Some(d) = &play.drill {
            let run = d.run;
            self.start_drill(run);
            return;
        }
        let path = play.path.clone();
        let practice = play.practice.as_ref().map(|p| p.restart_options());
        self.restart(path, practice);
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
                Screen::Title
                    | Screen::Config(..)
                    | Screen::Select
                    | Screen::Result(..)
                    | Screen::Drills(_)
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

    /// カーソルを動かす（帯が動いて、半分を過ぎたところでカーソルが動く。音は送ったときに鳴らす）
    fn move_cursor(&mut self, delta: isize) {
        if self.songs.len() == 0 {
            return;
        }
        if self.view.push(delta) {
            self.sfx.play(Se::Move);
        }
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
        let settings = self.load_settings(&path);
        let job = {
            let path = path.clone();
            Job::spawn("chart loader", move || settings.load(&path, practice))
        };
        self.screen = Screen::Loading(Box::new(Loading::new(&path, &label, wait, job)));
    }

    /// 曲を読むのに使う設定（読むスレッドへ渡す）
    fn load_settings(&self, path: &std::path::Path) -> LoadSettings {
        LoadSettings {
            input_offset_ms: self.config.input_offset_ms,
            song_offset_ms: self.history.song_offset(path),
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
            // 左バスドラムに入力が無ければ、バスドラムで取れるように
            groups: v.lane_groups().for_inputs(
                inputs::lane_sources(&self.config, self.ports()).contains_key(&DrumLane::LBD),
            ),
            hit_sounds: HitSoundRule {
                pad_first: v.pad_sound_first(),
                muting_lp: v.flag(settings::MUTING_LP),
            },
            auto_mix: AutoMix {
                bgm: v.flag(settings::BGM_SOUND),
                shift_ms: v.number(settings::BGM_ADJUST) as f64,
            },
            metronome: v.flag(settings::METRONOME),
        }
    }

    /// 曲をこのスレッドで読む（時間がかかる。演奏中のやり直しで使う）
    fn load_play(&self, path: &std::path::Path, practice: Option<PracticeOptions>) -> Result<Play> {
        let mut p = self.load_settings(path).load(path, practice)?;
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
                tf!(
                    "Hit sounds: off ({HIT_SOUND_KEY_NAME} to toggle)",
                    "叩いた音: 鳴らさない（{HIT_SOUND_KEY_NAME} で切り替え）"
                ),
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
        self.message = Some(tf!("Could not load: {e:#}", "読み込めませんでした: {e:#}"));
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

    /// 練習メニューを選曲画面に開く。`previous` があればその区間とつなぎ方を選んでおく。
    /// `finished` は今終えた練習（クリアしたら次の苦手な所を選んでおく）
    fn open_setup(
        &mut self,
        path: PathBuf,
        previous: Option<PracticeOptions>,
        finished: Option<Finished>,
    ) {
        if !matches!(self.screen, Screen::Select) {
            self.save_unfinished();
            self.go_to_select();
        }
        match PracticeSetup::load(&path, &self.history, previous, self.config.practice_prefs()) {
            Ok(s) => {
                self.message = None;
                // 同じ曲の練習から戻ってきたなら、苦手な所は前と同じ並びで出す
                let spots = self
                    .menu_spots
                    .take()
                    .filter(|(p, _)| *p == path)
                    .map(|(_, v)| v);
                let menu = PracticeMenu::new(s, spots, finished);
                self.menu_spots = Some((path.clone(), menu.spots().to_vec()));
                self.menu = Some(Box::new(menu));
                self.sfx.preview(None);
                let auto = self.rules().auto_mix;
                self.section_audio.prepare(&path, self.sfx.rate(), auto);
            }
            Err(e) => {
                log::error!("{e:#}");
                self.message = Some(tf!("Could not load: {e:#}", "読み込めませんでした: {e:#}"));
            }
        }
    }

    /// 練習メニューを操作する
    fn menu_input(&mut self, input: MenuInput) {
        let Some(m) = &mut self.menu else {
            return;
        };
        let r = m.input(input);
        self.menu_response(r);
    }

    /// 練習メニューの操作の結果：音を鳴らし、決めごとを覚え、閉じるか練習を始める
    fn menu_response(&mut self, r: practice_menu::Response) {
        let Some(m) = &self.menu else {
            return;
        };
        if let Some(se) = r.se {
            self.sfx.play(se);
        }
        // 区間のほかの決めごとは次の練習でも使う
        let prefs = m.setup.prefs();
        if prefs != self.config.practice_prefs() {
            self.config.set_practice_prefs(prefs);
            self.config.save();
        }
        match r.done {
            Some(Done::Start(o)) => {
                let path = m.setup.path.clone();
                self.menu = None;
                // 曲の音は残す（練習から戻ったら、すぐ流せるように）
                self.section_audio.stop(&mut self.sfx);
                self.start_practice(path, o);
            }
            Some(Done::Close) => self.close_menu(),
            None => {}
        }
    }

    /// 練習メニューを閉じて選曲に戻る
    fn close_menu(&mut self) {
        self.menu = None;
        self.menu_spots = None;
        self.section_audio.forget(&mut self.sfx);
    }

    /// 結果の画面の HH（キーボードは V）：今遊んだプレイのリプレイへ
    fn result_replay(&mut self) {
        if self.after_close.is_some() {
            return;
        }
        let Screen::Result(o, _) = &self.screen else {
            return;
        };
        let Some(session) = o
            .session_id
            .filter(|_| o.practice.is_none() && o.drill.is_none())
        else {
            return;
        };
        self.sfx.play(Se::Decide);
        self.close_then(AfterClose::Replay(session), Duration::ZERO);
    }

    /// 選んでいる曲のプレイの一覧（リプレイ）を選曲画面に開く
    fn open_replay_list(&mut self) {
        let Some(path) = self.songs.selected_chart() else {
            return;
        };
        let entries = self.history.plays(&path, REPLAY_LIST_LIMIT);
        let hash = std::fs::read(&path).ok().map(|b| records::hash_bytes(&b));
        let label = self.songs.chart_label(&path);
        let title = match self.songs.selected_song() {
            Some(s) if !label.is_empty() => format!("{} / {label}", s.title),
            Some(s) => s.title.clone(),
            None => label,
        };
        self.sfx.play(Se::Decide);
        self.sfx.preview(None);
        self.replays = Some(Box::new(ReplayList::new(title, entries, hash)));
    }

    /// プレイの一覧を操作する
    fn replay_list_input(&mut self, input: ListInput) {
        let Some(list) = &mut self.replays else {
            return;
        };
        let (se, done) = list.input(input);
        if let Some(se) = se {
            self.sfx.play(se);
        }
        match done {
            Some(ListDone::Play(session)) => {
                self.close_then(AfterClose::Replay(session), Duration::ZERO);
            }
            Some(ListDone::Close) => self.replays = None,
            None => {}
        }
    }

    /// 記録したプレイのリプレイを開く（シャッターを閉じたあと。譜面と音を読むあいだ画面は止まる）。
    /// 閉じたら、結果の画面から来たならその結果へ、一覧から来たなら一覧へ戻る
    fn open_replay(&mut self, session: i64) {
        let back = match std::mem::replace(&mut self.screen, Screen::Select) {
            Screen::Result(o, v) => Some(Back::Result(o, v)),
            other => {
                self.screen = other;
                self.replays.take().map(Back::List)
            }
        };
        let Some(back) = back else {
            self.go_to_select();
            return;
        };
        match self.load_replay(session) {
            Ok((mut play, when)) => {
                let label = self.songs.chart_label(&play.path);
                play.begin();
                let lag = play_view::LagDigits::from_config(&self.config);
                let looks = play_view::Looks::from_settings(&self.config.settings);
                let view = PlayView::new(&play, &label, lag, looks);
                self.quick = None;
                let run = ReplayRun::new(Box::new(play), Box::new(view), back, &when);
                self.screen = Screen::Replay(Box::new(run));
                self.shutter.open(shutter::OPEN_DELAY);
            }
            Err(e) => {
                log::error!("{e:#}");
                self.return_to(back);
                self.toast(tf!(
                    "Could not open the replay: {e:#}",
                    "リプレイを開けませんでした: {e:#}"
                ));
            }
        }
    }

    /// 記録から、プレイ `session` を遊んだときの決まりで読み直す。返すのは再生する曲と、遊んだ日時
    fn load_replay(&self, session: i64) -> Result<(Play, String)> {
        let rec = self.history.play_takes(session).with_context(|| {
            t(
                "This play is not in the records",
                "このプレイは記録にありません",
            )
        })?;
        let recorded = replay::RecordedRules::parse(&rec.settings);
        let mut rules = self.rules();
        recorded.apply(&mut rules);
        let path = PathBuf::from(&rec.chart_path);
        let play = Play::load_replay(&path, &rules, recorded.song_offset_ms, &rec)?;
        Ok((play, replay_view::when_label(rec.entry.started_at)))
    }

    /// リプレイを閉じて、来た画面へ戻る（シャッターを閉じたあと）
    fn leave_replay(&mut self) {
        match std::mem::replace(&mut self.screen, Screen::Select) {
            // 再生している曲はここで捨てる（音も止まる）
            Screen::Replay(run) => self.return_to(run.into_back()),
            other => self.screen = other,
        }
    }

    /// リプレイから戻る
    fn return_to(&mut self, back: Back) {
        match back {
            Back::Result(o, v) => {
                self.screen = Screen::Result(o, v);
                self.shutter.open(shutter::OPEN_DELAY);
            }
            Back::List(list) => {
                self.go_to_select();
                self.replays = Some(list);
            }
        }
    }

    /// リプレイのパッドとキー
    fn replay_input(&mut self, lane: DrumLane) {
        if self.after_close.is_some() {
            return;
        }
        let Screen::Replay(run) = &mut self.screen else {
            return;
        };
        match lane {
            DrumLane::CY => run.toggle(),
            DrumLane::HT | DrumLane::LT => {
                if let Some(se) = run.step(if lane == DrumLane::HT { -1 } else { 1 }) {
                    self.sfx.play(se);
                }
            }
            // 今見ている所を区間練習（練習メニューに区間を選んでおく）
            DrumLane::RD => {
                let (path, section) = (run.play.path.clone(), run.section());
                let options = self.practice_options(section);
                self.sfx.play(Se::Decide);
                self.close_then(AfterClose::Setup(path, Some(options), None), Duration::ZERO);
            }
            DrumLane::LC => {
                self.sfx.play(Se::Cancel);
                self.close_then(AfterClose::LeaveReplay, Duration::ZERO);
            }
            _ => {}
        }
    }

    /// 区間 `section` を、設定に残したほかの決めごとで練習する
    fn practice_options(&self, section: engine::practice::Section) -> PracticeOptions {
        let p = self.config.practice_prefs();
        PracticeOptions {
            section,
            style: p.style,
            rule: p.rule,
            speed: p.speed,
            pitch: p.pitch,
            metronome: p.metronome,
            step_laps: p.step_laps,
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
        let end_ms = at_ms.unwrap_or_else(|| p.now_ms());
        let finish = view.finish(p, end_ms);
        let practice = p.practice.is_some();
        let drill = p.drill.as_ref().map(|d| d.run.drill.id);
        // 基礎練習の最高は、今回を残す前の記録と比べる
        let best_before = drill.and_then(|id| self.history.drill_records().get(id)?.best);
        // Esc でやめたときも結果は出す（記録は「最後まで遊んでいない」として残す）
        let cleared = p.finished();
        let session = self
            .history
            .save(p, Some(play_score(&finish, cleared)), end_ms);
        let stats = self.history.chart_stats();
        // New Record と前回のベストのバーは、今回より前のプレイと比べる。自動で叩くレーンがあるときと、
        // 早送りか巻き戻しをしたときは比べない
        let best = if p.has_auto() || p.skipped() {
            None
        } else {
            self.history.best_score(&p.path, session)
        };
        let mut outcome = p.outcome();
        outcome.session_id = session;
        if let Some(d) = outcome.drill.as_mut() {
            d.best_before = best_before;
        }
        if !practice && drill.is_none() {
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
            // 今回のプレイも入れて、補正のおすすめを出す
            let charts = self.songs.song_charts(&outcome.path);
            let advice =
                self.history
                    .lag_advice(&charts, self.config.input_offset_ms, p.song_offset_ms);
            outcome.lag = Some(LagResult {
                charts,
                advice,
                applied: None,
            });
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
            drill: drill.is_some(),
            cleared: cleared && !practice && drill.is_none(),
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
            self.history.save(p, Some(play_score(&finish, false)), now);
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
        // ほかの DTXMania を探し終えたら、まだ引き継ぐ画面を見せていなければタイトルで知らせる
        if self.import.poll() {
            let first = self.import.sources().first().map(nx::Source::title);
            if !self.config.import_offered {
                self.title.set_found(first);
            }
        }
        // 曲を探し終えたら、譜面の隣の NX の score.ini をまだなら記録に引き継ぐ
        if self.songs.poll() {
            if self.history.import_nx_scores(&self.songs.library) > 0 {
                self.songs.set_stats(self.history.chart_stats());
            }
            if std::mem::take(&mut self.rescanning_for_add) {
                self.reveal_ready = self.jump_to.is_some();
            }
        }
        self.poll_adder();
        if self.reveal_ready && self.can_reveal() {
            self.reveal_ready = false;
            self.show_added_song();
        }
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
        self.capture_new_record();
        let stage_failed = self.stage_failed_enabled();
        let mut failed_out = false;
        let mut finished = false;
        // セットの合間で「やめる」を選んだ（その時刻）
        let mut quit_at = None;
        // 練習で 1 周叩かなかった（一時停止の板を出す）
        let mut rested = false;
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
                for e in play.take_practice_events() {
                    match e {
                        game::PracticeEvent::StepUp => self.sfx.play(Se::Decide),
                        game::PracticeEvent::Cleared => self.sfx.play(Se::StageClear),
                        game::PracticeEvent::Finished { at_ms } => quit_at = Some(at_ms),
                        game::PracticeEvent::Rest => rested = true,
                    }
                }
                view.feed(play);
                if stage_failed
                    && play.practice.is_none()
                    && play.drill.is_none()
                    && !play.skipped()
                    && view.gauge_empty()
                {
                    // 上流と同じく、音を止めて STAGE FAILED を出し、選曲に戻る（記録は残す）
                    view.start_failure(play.view_ms(self.config.visual_offset_ms));
                    play.silence();
                    self.sfx.play(Se::StageFailed);
                } else {
                    finished = play.finished();
                }
            }
        }
        if let Screen::Replay(run) = &mut self.screen
            && !closing
        {
            run.tick(play_view::view_ahead_ms(self.config.scroll_px_per_ms));
        }
        if failed_out {
            self.save_unfinished();
            self.go_to_select();
        } else if finished {
            self.finish_play(None);
        } else if let Some(at_ms) = quit_at {
            // 一時停止の板の「演奏をやめる」と同じく、シャッターを閉じてから結果へ
            self.close_then(AfterClose::Result { at_ms }, Duration::ZERO);
        } else if rested && self.quick.is_none() && self.after_close.is_none() {
            // ドラムだけで止められるように、叩くのをやめたら一時停止する（叩かなかった周は数えない）
            let drill = matches!(&self.screen, Screen::Play(p, _) if p.drill.is_some());
            self.open_pause();
            let text = if drill {
                t(
                    "Paused because you stopped playing",
                    "叩いていないので止めました",
                )
            } else {
                t(
                    "Paused because you stopped playing\nThis loop won't count",
                    "叩いていないので止めました\nこの周は数えません",
                )
            };
            self.toast_for(text.to_owned(), Duration::from_secs(4));
        }
        if self.shutter.is_closed()
            && let Some(next) = self.after_close.take()
        {
            self.closed(next);
        }
        // 練習メニューを開いていれば、プレビューの代わりに選んでいる所の音を流す
        self.menu_sound = match &self.menu {
            Some(m) if self.after_close.is_none() => {
                let shown = m.shown();
                Some(self.section_audio.update(&mut self.sfx, shown))
            }
            _ => {
                self.section_audio.stop(&mut self.sfx);
                None
            }
        };
        if matches!(self.screen, Screen::Select)
            && self.after_close.is_none()
            && self.menu.is_none()
            && self.replays.is_none()
        {
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
            Some(TitleItem::Drills) => {
                self.sfx.play(Se::Decide);
                self.sfx.stop_title();
                self.close_then(AfterClose::Drills, Duration::ZERO);
            }
            Some(TitleItem::Records) => {
                self.sfx.play(Se::Decide);
                self.sfx.stop_title();
                self.close_then(
                    AfterClose::Records(RecordsFrom::Title, None),
                    Duration::ZERO,
                );
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
            Some(TitleItem::Update) => self.title_update(),
            None => {}
        }
    }

    /// タイトルの「更新」の行を決めた（落とし始める、もう一度落とす、再起動して入れ替える）
    fn title_update(&mut self) {
        if self.updater.download() {
            self.sfx.play(Se::Decide);
        } else if let Some(v) = self.updater.request_restart() {
            self.sfx.play(Se::Decide);
            self.sfx.stop_title();
            self.applying = Some((v, Instant::now()));
        }
    }

    /// 別のスレッドからの更新の知らせを受け取り、落とし終えたら知らせる（タイトルと演奏中は、タイトルの行だけで知らせる）
    fn poll_update(&mut self) {
        let Some(event) = self.updater.poll() else {
            return;
        };
        let text = match event {
            update::Event::Ready(v) => tf!(
                "v{v} downloaded. It will be installed when you quit.",
                "v{v} を落としました。終了すると入れ替わります"
            ),
            update::Event::Failed(v) => tf!("Could not download v{v}", "v{v} を落とせませんでした"),
        };
        match self.screen {
            // タイトルは「更新」の行が変わるので知らせない（知らせはその行と重なる）
            Screen::Title | Screen::Play(..) | Screen::Loading(_) | Screen::Replay(_) => {}
            Screen::Config(..) => self.config_toast(text),
            _ => self.toast(text),
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
        let mut info = config_info(
            &self.config,
            &self.midi,
            &self.monitor,
            &self.songs,
            &self.monitor_list,
        );
        info.timing = self.timing.as_ref().map(SongTiming::info);
        info.import = Some(self.import.info());
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
                    tf!(
                        "Added note {note} to {} ({device})",
                        "ノート {note} を{}に足しました（{device}）",
                        inputs::lane_label(lane)
                    )
                } else {
                    tf!(
                        "Removed note {note} from {} ({device})",
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
                    tf!(
                        "Added key {key} to {}",
                        "キー {key} を{}に足しました",
                        inputs::lane_label(lane)
                    )
                } else {
                    tf!(
                        "Removed key {key} from {}",
                        "キー {key} を{}から外しました",
                        inputs::lane_label(lane)
                    )
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
                self.config_toast(tf!(
                    "Cleared everything assigned to {}",
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
        let mut info = config_info(
            &self.config,
            &self.midi,
            &self.monitor,
            &self.songs,
            &self.monitor_list,
        );
        info.song = self.songs.selected_chart().is_some();
        info.timing = self.timing.as_ref().map(SongTiming::info);
        if let Screen::Play(p, _) = &self.screen {
            info.play = Some(play_pos(p));
        }
        info
    }

    /// 選曲の簡易設定（パッドは RD、キーボードは F2）を開く
    fn open_quick(&mut self) {
        if self.quick.is_some() || self.menu.is_some() {
            return;
        }
        self.sfx.play(Se::Decide);
        let chart = self.songs.selected_chart();
        self.load_timing(chart.as_deref());
        self.quick = Some(Box::new(ConfigView::panel(config_view::Page::Quick)));
    }

    /// 補正の行に出す、この曲（`chart`）の補正とおすすめを記録から出す
    fn load_timing(&mut self, chart: Option<&std::path::Path>) {
        let (charts, offset_ms) = match chart {
            Some(p) => (self.songs.song_charts(p), self.history.song_offset(p)),
            None => (Vec::new(), 0.0),
        };
        let advice = self
            .history
            .lag_advice(&charts, self.config.input_offset_ms, offset_ms);
        self.timing = Some(SongTiming {
            charts,
            offset_ms,
            advice,
        });
    }

    /// 補正を変えたので、おすすめを出し直す
    fn refresh_timing(&mut self) {
        if let Some(t) = &mut self.timing {
            t.advice = self
                .history
                .lag_advice(&t.charts, self.config.input_offset_ms, t.offset_ms);
        }
    }

    /// この曲の補正を変えて記録に残す（演奏中なら演奏にも）
    fn set_song_offset(&mut self, offset_ms: f64) {
        let Some(t) = &mut self.timing else {
            return;
        };
        if t.charts.is_empty() {
            return;
        }
        t.offset_ms = offset_ms;
        self.history.set_song_offset(&t.charts, offset_ms);
        if let Screen::Play(play, _) = &mut self.screen
            && t.charts.contains(&play.path)
        {
            play.song_offset_ms = offset_ms;
        }
        self.refresh_timing();
    }

    /// 補正をおすすめの値にする。元の値（全体、この曲）を返す
    fn apply_advice(&mut self, advice: &engine::lag::Advice) -> (f64, f64) {
        let before = (
            self.config.input_offset_ms,
            self.timing.as_ref().map_or(0.0, |t| t.offset_ms),
        );
        self.set_offsets(advice.global_ms as f64, advice.song_ms as f64);
        before
    }

    /// 入力の補正（全体）とこの曲の補正を変えて残す
    fn set_offsets(&mut self, global_ms: f64, song_ms: f64) {
        if self.config.input_offset_ms != global_ms {
            self.config.input_offset_ms = global_ms;
            self.config.save();
            if let Screen::Play(play, _) = &mut self.screen {
                play.input_offset_ms = global_ms;
            }
        }
        if self.timing.as_ref().is_some_and(|t| t.offset_ms != song_ms) {
            self.set_song_offset(song_ms);
        } else {
            self.refresh_timing();
        }
    }

    /// 演奏を一時停止して、一時停止の板を開く（Esc・Space・Pause）
    fn open_pause(&mut self) {
        let Screen::Play(play, _) = &mut self.screen else {
            return;
        };
        if !play.paused() {
            play.toggle_pause();
        }
        let path = play.path.clone();
        self.sfx.play(Se::Decide);
        self.load_timing(Some(&path));
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
                if let Screen::Play(play, view) = &mut self.screen {
                    play.input_offset_ms = self.config.input_offset_ms;
                    play.drum_sounds = self.config.drum_sounds;
                    view.set_hit_error(self.config.settings.flag(settings::HIT_ERROR_BAR));
                    // メトロノームは、練習でなければ設定の、練習なら練習の決めごとの行（一時停止の板）
                    match (id, play.practice.is_some()) {
                        (ItemId::Table(settings::METRONOME), false) => {
                            play.set_metronome(self.config.settings.flag(settings::METRONOME));
                        }
                        (ItemId::Practice(Drill::Metronome), true) => {
                            play.set_metronome(self.config.practice_metronome);
                        }
                        _ => {}
                    }
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
                self.quick = None;
                self.restart_current();
            }
            ConfigAction::Run(ItemId::Quit) => {
                if let Screen::Play(play, _) = &self.screen {
                    // Esc でやめていたときと同じく、シャッターを閉じてから結果へ
                    let at_ms = play.now_ms();
                    self.quick = None;
                    self.close_then(AfterClose::Result { at_ms }, Duration::ZERO);
                }
            }
            ConfigAction::Run(ItemId::Reselect) if matches!(&self.screen, Screen::Play(p, _) if p.drill.is_some()) =>
            {
                // 基礎練習の画面へ（ここまでの段は記録に残す。open_drills）
                self.quick = None;
                self.close_then(AfterClose::Drills, Duration::ZERO);
            }
            ConfigAction::Run(ItemId::Reselect) => {
                if let Screen::Play(play, _) = &self.screen
                    && let Some(options) = play.practice.as_ref().map(|p| p.restart_options())
                {
                    // ここまでの周は記録に残し（open_setup）、今の決めごとを選んだ練習メニューを開く
                    let path = play.path.clone();
                    self.quick = None;
                    self.close_then(AfterClose::Setup(path, Some(options), None), Duration::ZERO);
                }
            }
            ConfigAction::Run(ItemId::PracticeSong) => {
                self.quick = None;
                self.select_action(ui::SelectAction::Practice);
            }
            ConfigAction::Run(ItemId::ReplaySong) => {
                self.quick = None;
                self.open_replay_list();
            }
            ConfigAction::Run(ItemId::SongRecords) => {
                self.quick = None;
                let chart = self.songs.selected_chart();
                self.close_then(
                    AfterClose::Records(RecordsFrom::Select, chart),
                    Duration::ZERO,
                );
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
            KeyCode::KeyV if !paused => {
                self.quick_action(ConfigAction::Run(ItemId::ReplaySong));
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
                match s {
                    Setting::MasterVolume => apply_volumes(&self.config),
                    Setting::Update => self.updater.set_mode(self.config.update_mode()),
                    Setting::Language => {
                        i18n::set_lang(self.config.lang());
                        // BOX の名前（並べ替えで作るもの）を今の言語で作り直す
                        self.songs.relabel();
                    }
                    _ => {}
                }
                self.config.save();
                if s == Setting::InputAdjust {
                    self.refresh_timing();
                }
            }
            ItemId::SongOffset => {
                let r = config_view::OFFSET_RANGE;
                self.set_song_offset((v as f64).clamp(-r, r));
            }
            ItemId::Table(key) => {
                self.config.settings.set_raw(key, v);
                if settings::VOLUMES.contains(&key) {
                    apply_volumes(&self.config);
                }
                self.config.save();
            }
            ItemId::Practice(d) => {
                config_view::set_drill(d, &mut self.config, v);
                self.config.save();
            }
            ItemId::Monitor => {
                // 窓はこのあとのフレームで移す（apply_window_settings）
                if config_view::set_monitor(&mut self.config, &self.monitor_list, v) {
                    self.config.save();
                }
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
            ItemId::ImportMidi(i) => {
                if let Some(m) = self.import.plan.as_mut().and_then(|p| p.midi.get_mut(i)) {
                    m.on = v != 0;
                }
            }
            ItemId::ImportMidiTarget(i) => {
                if let Some(p) = self.import.plan.as_mut() {
                    let last = p.targets.len().saturating_sub(1) as i64;
                    if let Some(m) = p.midi.get_mut(i) {
                        m.target = v.clamp(0, last) as usize;
                    }
                }
            }
            ItemId::ImportItem(i) => {
                if let Some(it) = self.import.plan.as_mut().and_then(|p| p.items.get_mut(i)) {
                    it.on = v != 0;
                }
            }
            _ => {}
        }
    }

    /// 設定の画面の動かす項目
    fn config_run(&mut self, id: ItemId) {
        match id {
            ItemId::AddSongDir => {
                if let Some(dir) = ui::pick_folder(t("Choose a Song Folder", "曲のフォルダを選ぶ"))
                {
                    self.add_song_dir(dir);
                }
            }
            ItemId::SongDir(i) => {
                if i < self.config.song_dirs.len() {
                    self.config.song_dirs.remove(i);
                    self.config.save();
                    self.songs.rescan(self.config.song_dirs.clone());
                    self.config_toast(
                        t(
                            "Removed the song folder from the list",
                            "曲のフォルダを一覧から外しました",
                        )
                        .to_owned(),
                    );
                }
            }
            ItemId::AddTo => {
                self.config.next_add_dest();
                self.config.save();
            }
            ItemId::Rescan => {
                if !self.config.song_dirs.is_empty() {
                    self.songs.rescan(self.config.song_dirs.clone());
                }
            }
            ItemId::ReconnectMidi => self.reopen_midi(),
            ItemId::OpenEditor => match editor::open() {
                Ok(editor::Opened::Opened) => {
                    log::info!("opened the chart editor: {}", editor::URL);
                    self.config_toast(
                        t(
                            "Opened the chart editor in your browser",
                            "ブラウザで譜面エディタを開きました",
                        )
                        .to_owned(),
                    );
                }
                Ok(editor::Opened::Recently) => {}
                Err(e) => {
                    log::warn!("could not open the chart editor: {e}");
                    self.config_toast(tf!(
                        "Couldn't open a browser. Please open {} yourself.",
                        "ブラウザを開けませんでした。{} を開いてください",
                        editor::URL
                    ));
                }
            },
            ItemId::OpenCredits => match credits::open() {
                Ok(editor::Opened::Opened) => {
                    log::info!("opened the credits: {}", credits::URL);
                    self.config_toast(
                        t(
                            "Opened the credits in your browser",
                            "ブラウザでクレジットを開きました",
                        )
                        .to_owned(),
                    );
                }
                Ok(editor::Opened::Recently) => {}
                Err(e) => {
                    log::warn!("could not open the credits: {e}");
                    self.config_toast(tf!(
                        "Couldn't open a browser. Please open {} yourself.",
                        "ブラウザを開けませんでした。{} を開いてください",
                        credits::URL
                    ));
                }
            },
            ItemId::ApplyAdvice => {
                if let Some(a) = self.timing.as_ref().map(|t| t.advice)
                    && a.changes()
                {
                    self.apply_advice(&a);
                    self.config_toast(tf!(
                        "Applied the suggested offset (global {:+} ms, this song {:+} ms)",
                        "補正をおすすめにしました（全体 {:+} ms、この曲 {:+} ms）",
                        a.global_ms,
                        a.song_ms
                    ));
                }
            }
            ItemId::ImportSource(i) => {
                if let Some(source) = self.import.sources().get(i).cloned() {
                    // 探したときから変わっているかもしれないので読み直す
                    match nx::source::read(&source.found) {
                        Ok(s) => self.import_open(&s),
                        Err(_) => self.import_from_path(&source.found.config.clone()),
                    }
                }
            }
            ItemId::ImportPickFile => {
                if let Some(path) = ui::pick_file(
                    t(
                        "Choose a DTXMania Shortcut, exe, or Config.ini",
                        "DTXMania のショートカット・exe・Config.ini を選ぶ",
                    ),
                    ("DTXMania", &["lnk", "exe", "ini", "xml", "desktop"]),
                ) {
                    self.import_from_path(&path);
                }
            }
            ItemId::ImportPickFolder => {
                if let Some(dir) =
                    ui::pick_folder(t("Choose the DTXMania Folder", "DTXMania のフォルダを選ぶ"))
                {
                    self.import_from_path(&dir);
                }
            }
            ItemId::ImportSearch => {
                self.import.error = None;
                self.import.search = Some(takeover::Search::start());
                // 自分で探し直したので、タイトルで知らせなくてよい
                if !self.config.import_offered {
                    self.config.import_offered = true;
                    self.config.save();
                }
            }
            ItemId::ImportApply => self.import_apply(),
            ItemId::ImportUndo => self.import_undo(),
            ItemId::ResetPads => {
                inputs::reset_all(&mut self.config);
                self.config.save();
                self.refresh_pads();
                self.config_toast(
                    t("Mappings reset to default", "割り当てを既定に戻しました").to_owned(),
                );
            }
            ItemId::DeviceReset(port) => {
                let ports = self.ports().to_vec();
                inputs::reset_device(&mut self.config, &ports, port);
                self.config.save();
                self.refresh_pads();
                self.config_toast(
                    t(
                        "This device's mappings were reset to its preset",
                        "この機器の割り当てをプリセットに戻しました",
                    )
                    .to_owned(),
                );
            }
            _ => {}
        }
    }

    fn on_key(&mut self, code: KeyCode) {
        if self.after_close.is_some() || self.applying.is_some() {
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
            Screen::Drills(_) => {
                let input = match code {
                    KeyCode::ArrowUp => drill_menu::Input::Up,
                    KeyCode::ArrowDown => drill_menu::Input::Down,
                    KeyCode::ArrowLeft => drill_menu::Input::Left,
                    KeyCode::ArrowRight => drill_menu::Input::Right,
                    KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                        drill_menu::Input::Decide
                    }
                    KeyCode::Escape | KeyCode::Backspace => drill_menu::Input::Cancel,
                    _ => return,
                };
                self.drills_input(input);
            }
            Screen::Records(_) => {
                let input = match code {
                    KeyCode::ArrowUp => RecordsInput::Up,
                    KeyCode::ArrowDown => RecordsInput::Down,
                    KeyCode::ArrowLeft => RecordsInput::Left,
                    KeyCode::ArrowRight => RecordsInput::Right,
                    KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => RecordsInput::Decide,
                    KeyCode::Escape | KeyCode::Backspace => RecordsInput::Cancel,
                    _ => return,
                };
                self.records_input(input);
            }
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
                KeyCode::KeyO => self.toggle_result_advice(),
                KeyCode::KeyV => self.result_replay(),
                _ => {}
            },
            Screen::Replay(_) => {
                let lane = match code {
                    KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Pause => {
                        DrumLane::CY
                    }
                    KeyCode::ArrowLeft | KeyCode::ArrowUp => DrumLane::HT,
                    KeyCode::ArrowRight | KeyCode::ArrowDown => DrumLane::LT,
                    KeyCode::KeyP => DrumLane::RD,
                    KeyCode::Escape | KeyCode::Backspace => DrumLane::LC,
                    _ => return,
                };
                self.replay_input(lane);
            }
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
                self.restart_current();
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
                tf!(
                    "{} to {:.0} s (this play won't be recorded)",
                    "{} {:.0} 秒へ（このプレイは記録に残しません）",
                    if d < 0.0 {
                        t("Rewind", "巻き戻し")
                    } else {
                        t("Skip", "早送り")
                    },
                    to.max(0.0) / 1000.0
                )
            }
            KeyCode::Comma | KeyCode::Period if play.practice.is_some() => {
                play.nudge_speed(if code == KeyCode::Comma { -1 } else { 1 });
                let speed = play.practice.as_ref().map_or(100, |p| p.speed());
                tf!(
                    "Speed {speed}% (from the next loop, after the lead-in; this step starts over from loop 1)",
                    "速度 {speed}%（次の周から、助走のあと。この段は 1 周目から数え直し）"
                )
            }
            HIT_SOUND_KEY => {
                play.drum_sounds = !play.drum_sounds;
                self.config.drum_sounds = play.drum_sounds;
                self.config.save();
                if play.drum_sounds {
                    t("Hit sounds on", "叩いた音を鳴らす")
                } else {
                    t("Hit sounds off", "叩いた音を鳴らさない")
                }
                .to_owned()
            }
            KeyCode::Minus | KeyCode::Equal => {
                let d = if code == KeyCode::Minus { -1.0 } else { 1.0 };
                play.input_offset_ms += d;
                self.config.input_offset_ms = play.input_offset_ms;
                self.config.save();
                tf!(
                    "Input offset {:+.0} ms",
                    "入力の補正 {:+.0}ms",
                    play.input_offset_ms
                )
            }
            KeyCode::BracketLeft | KeyCode::BracketRight => {
                let d = if code == KeyCode::BracketLeft {
                    -1.0
                } else {
                    1.0
                };
                self.config.visual_offset_ms += d;
                self.config.save();
                tf!(
                    "Visual offset {:+.0} ms",
                    "表示の補正 {:+.0}ms",
                    self.config.visual_offset_ms
                )
            }
            KeyCode::ArrowUp | KeyCode::ArrowDown => {
                let d = if code == KeyCode::ArrowUp {
                    0.05
                } else {
                    -0.05
                };
                self.config.scroll_px_per_ms = (self.config.scroll_px_per_ms + d).clamp(0.1, 3.0);
                self.config.save();
                tf!(
                    "Scroll speed {:.2}",
                    "譜面の速さ {:.2}",
                    self.config.scroll_px_per_ms
                )
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
            || self.replays.is_some()
        {
            self.wheel = 0.0;
            return;
        }
        if let Some(m) = &mut self.menu {
            self.wheel = 0.0;
            let r = m.mouse(mouse.menu);
            self.menu_response(r);
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
            Some(d) if self.songs.len() > 0 => {
                self.view.push_to(d);
                self.sfx.play(Se::Move);
            }
            _ => {}
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
        if self.replays.is_some() {
            let input = match code {
                KeyCode::ArrowUp => ListInput::Up,
                KeyCode::ArrowDown => ListInput::Down,
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => ListInput::Decide,
                KeyCode::Escape | KeyCode::Backspace => ListInput::Cancel,
                _ => return,
            };
            self.replay_list_input(input);
            return;
        }
        if self.menu.is_some() {
            self.on_menu_key(code);
            return;
        }
        // 簡易設定（パッドなら RD）
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
            KeyCode::Escape => self.select_cancel(),
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

    /// 選曲の「戻る」（Esc と LC。v2 の ActionCancel）。BOX の中なら 1 つ上へ出て、
    /// 一番上ならタイトルへ（v2 はすぐ切り替え、タイトルが白から出る）
    fn select_cancel(&mut self) {
        self.sfx.play(Se::Cancel);
        if self.songs.back() {
            self.view.jumped();
        } else {
            self.sfx.preview(None);
            self.go_to_title();
        }
    }

    /// 練習メニューを開いているときのキー
    fn on_menu_key(&mut self, code: KeyCode) {
        let input = match code {
            KeyCode::ArrowUp => MenuInput::Up,
            KeyCode::ArrowDown => MenuInput::Down,
            KeyCode::ArrowLeft => MenuInput::Left,
            KeyCode::ArrowRight => MenuInput::Right,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => MenuInput::Decide,
            KeyCode::Escape | KeyCode::Backspace => MenuInput::Cancel,
            KeyCode::Comma | KeyCode::Period => {
                if let Some(m) = &mut self.menu {
                    m.setup
                        .nudge_speed(if code == KeyCode::Comma { -1 } else { 1 });
                    self.menu_response(practice_menu::Response {
                        se: Some(Se::Move),
                        done: None,
                    });
                }
                return;
            }
            _ => return,
        };
        self.menu_input(input);
    }

    /// メニューの画面での電子ドラムの操作（上流の割り当てがもと）。
    /// HT で上、LT で下、CY か RD で決定、LC で戻る。選曲は HH を 2 回で難易度、RD で簡易設定（決定は CY だけ）。
    /// 結果は SD でもう一度、RD で区間練習、CY か LC で選曲へ（練習の結果なら CY・RD・LC で練習メニューへ）。
    /// 足で踏むパッド（BD・LP・LBD）は演奏の合間に踏んでしまうので、メニューの操作には使わない
    fn on_menu_pad(&mut self, lane: DrumLane, at_us: i64) {
        if self.after_close.is_some() || self.applying.is_some() {
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
            // 画面の下の帯に出す役目（`RecordsView::hints`）。足のパッドは使わない
            Screen::Records(_) => {
                let input = match lane {
                    DrumLane::HT => RecordsInput::Up,
                    DrumLane::LT => RecordsInput::Down,
                    DrumLane::SD => RecordsInput::Left,
                    DrumLane::FT => RecordsInput::Right,
                    DrumLane::CY => RecordsInput::Decide,
                    DrumLane::LC => RecordsInput::Cancel,
                    _ => return,
                };
                self.records_input(input);
            }
            Screen::Select => {
                if self.replays.is_some() {
                    let input = match lane {
                        DrumLane::HT => ListInput::Up,
                        DrumLane::LT => ListInput::Down,
                        DrumLane::CY | DrumLane::RD => ListInput::Decide,
                        DrumLane::LC => ListInput::Cancel,
                        _ => return,
                    };
                    self.replay_list_input(input);
                    return;
                }
                if self.menu.is_some() {
                    // 設定の画面と同じパッドの役目（SD・FT で区間や値をその場で変える）
                    let input = match lane {
                        DrumLane::HT => MenuInput::Up,
                        DrumLane::LT => MenuInput::Down,
                        DrumLane::SD => MenuInput::Left,
                        DrumLane::FT => MenuInput::Right,
                        DrumLane::CY | DrumLane::RD => MenuInput::Decide,
                        DrumLane::LC => MenuInput::Cancel,
                        _ => return,
                    };
                    self.menu_input(input);
                    return;
                }
                match lane {
                    DrumLane::HT => self.move_cursor(-1),
                    DrumLane::LT => self.move_cursor(1),
                    // v2 と同じく SD と FT で並べ替え
                    DrumLane::SD => self.change_sort(-1),
                    DrumLane::FT => self.change_sort(1),
                    DrumLane::CY => self.select_action(ui::SelectAction::Enter),
                    // 簡易設定（v2 は BD 2 回だが、足は演奏の合間に踏んでしまうので手で叩く RD にした）
                    DrumLane::RD => self.open_quick(),
                    // v2 と同じく LC は Esc と同じ（BOX の中なら出て、一番上ならタイトルへ）
                    DrumLane::LC => self.select_cancel(),
                    DrumLane::HH | DrumLane::HHO => {
                        if self.last_hh_us.is_some_and(|t| at_us - t <= DOUBLE_HIT_US) {
                            self.last_hh_us = None;
                            self.cycle_level();
                        } else {
                            self.last_hh_us = Some(at_us);
                        }
                    }
                    _ => {}
                }
            }
            // 画面の下の帯に出す役目（`ResultView::hints`）
            Screen::Result(..) => match lane {
                DrumLane::CY | DrumLane::LC => self.result_action(ui::ResultAction::Back),
                // もう一度（練習なら同じ決めごとで。キーボードは R）
                DrumLane::SD => self.result_action(ui::ResultAction::Retry),
                // 練習メニュー（練習の結果なら今の区間を選んでおく。キーボードは P）
                DrumLane::RD => self.result_action(ui::ResultAction::Practice(None)),
                // 「ずれ」の板のおすすめ（キーボードは O）
                DrumLane::FT => self.toggle_result_advice(),
                // 選曲と同じく HT と LT で動かす
                DrumLane::HT | DrumLane::LT => self.turn_result_page(),
                // 今のプレイのリプレイ（キーボードは V）
                DrumLane::HH | DrumLane::HHO => self.result_replay(),
                _ => {}
            },
            Screen::Replay(_) => self.replay_input(lane),
            // 設定の画面と同じパッドの役目（SD・FT でテンポ）
            Screen::Drills(_) => {
                let input = match lane {
                    DrumLane::HT => drill_menu::Input::Up,
                    DrumLane::LT => drill_menu::Input::Down,
                    DrumLane::SD => drill_menu::Input::Left,
                    DrumLane::FT => drill_menu::Input::Right,
                    DrumLane::CY | DrumLane::RD => drill_menu::Input::Decide,
                    DrumLane::LC => drill_menu::Input::Cancel,
                    _ => return,
                };
                self.drills_input(input);
            }
            Screen::Loading(_) | Screen::Play(..) => {}
        }
    }

    /// 結果の画面の「ずれ」の板のおすすめにする。もうしていれば、もとに戻す
    fn toggle_result_advice(&mut self) {
        // 板は 1 ページ目だけに出す
        let Screen::Result(o, view) = &self.screen else {
            return;
        };
        if view.page != Page::Summary {
            return;
        }
        let Some(lag) = &o.lag else {
            return;
        };
        let (advice, applied) = (lag.advice, lag.applied);
        if applied.is_none() && !advice.changes() {
            return;
        }
        let charts = lag.charts.clone();
        let offset_ms = charts.first().map_or(0.0, |p| self.history.song_offset(p));
        self.timing = Some(SongTiming {
            charts,
            offset_ms,
            advice,
        });
        let applied = match applied {
            Some((global, song)) => {
                self.set_offsets(global, song);
                self.sfx.play(Se::Cancel);
                None
            }
            None => {
                let before = self.apply_advice(&advice);
                self.sfx.play(Se::Decide);
                Some(before)
            }
        };
        if let Screen::Result(o, _) = &mut self.screen
            && let Some(lag) = &mut o.lag
        {
            lag.applied = applied;
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
                    self.open_setup(path, None, None);
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
        // 基礎練習の結果からは、もう一度か基礎練習の画面へ
        if let Some(d) = &o.drill {
            let run = d.run;
            match action {
                ui::ResultAction::Retry => {
                    self.sfx.play(Se::Decide);
                    self.close_then(AfterClose::Drill(run), Duration::ZERO);
                }
                _ => {
                    self.sfx.play(Se::Cancel);
                    self.close_then(AfterClose::Drills, Duration::ZERO);
                }
            }
            return;
        }
        let path = o.path.clone();
        let options = o.practice.as_ref().map(|p| p.options);
        let finished = o.practice.as_ref().map(|p| Finished {
            section: p.options.section,
            cleared: p.cleared,
            speed: p.options.speed.goal(),
        });
        match action {
            // 練習の結果からは、選曲でなく練習メニューへ戻る（次の苦手な所を選べるように）
            ui::ResultAction::Back | ui::ResultAction::Practice(None) if finished.is_some() => {
                self.sfx.play(Se::Decide);
                self.close_then(AfterClose::Setup(path, options, finished), Duration::ZERO);
            }
            ui::ResultAction::Back => {
                // 上流と同じく戻る音を鳴らして選曲へ（v2：シャッターを閉じてから）
                self.sfx.play(Se::Cancel);
                self.close_then(AfterClose::Select, Duration::ZERO);
            }
            ui::ResultAction::Retry => match options {
                Some(o) => self.start_practice(path, o),
                None => self.start(path),
            },
            ui::ResultAction::Practice(section) => {
                let previous = match section {
                    Some(section) => Some(self.practice_options(section)),
                    None => options,
                };
                self.close_then(AfterClose::Setup(path, previous, None), Duration::ZERO);
            }
        }
    }

    /// ファイルをウィンドウにドロップされた。譜面ならすぐ遊び、zip や曲のフォルダなら曲のフォルダへ写す
    fn on_drop(&mut self, path: PathBuf) {
        self.drop_hover.clear();
        if ui::is_chart_file(&path) {
            // シャッターを閉じているあいだは次の画面が決まっているので、曲を始めない
            if self.after_close.is_none() {
                self.start(path);
            }
            return;
        }
        // 他の DTXMania から引き継ぐ画面なら、ショートカットやフォルダから読む
        if self.on_import_page() {
            self.import_from_path(&path);
            return;
        }
        log::info!("dropped: {}", path.display());
        self.adder
            .add(path, self.config.add_dest(), self.config.song_dirs.clone());
    }

    /// 他の DTXMania から引き継ぐページを開いている（ドロップは曲の追加でなく設定を読む）
    fn on_import_page(&self) -> bool {
        matches!(&self.screen, Screen::Config(view, _) if matches!(
            view.page(),
            config_view::Page::Import | config_view::Page::ImportItems
        ))
    }

    /// 写し終えたものを受け取る。全部終わったら曲を探し直し、知らせを出す
    fn poll_adder(&mut self) {
        let done = self.adder.poll();
        if done.is_empty() {
            return;
        }
        for o in done {
            match &o {
                song_adder::Outcome::Added { dest, added, .. } => {
                    // 曲のフォルダが無かったときの写す先（ドキュメントの Songs）を曲のフォルダにする
                    if self.config.add_song_dir(dest.clone()) {
                        self.config.save();
                    }
                    if !added.already {
                        self.view.fresh.push(added.dir.clone());
                    }
                    if self.jump_to.is_none() {
                        self.jump_to = o.first_chart().map(std::path::Path::to_owned);
                    }
                }
                song_adder::Outcome::InLibrary { dir } => {
                    self.jump_to.get_or_insert_with(|| dir.clone());
                }
                song_adder::Outcome::Register { dir } => {
                    if self.config.add_song_dir(dir.clone()) {
                        self.config.save();
                    }
                    self.jump_to.get_or_insert_with(|| dir.clone());
                }
                song_adder::Outcome::Failed { .. } => {}
            }
            self.added.push(o);
        }
        if self.adder.busy() {
            return;
        }
        let batch = std::mem::take(&mut self.added);
        if let Some(text) = song_adder::summary(&batch) {
            self.toast_for(text, ADD_TOAST_FOR);
        }
        if batch
            .iter()
            .any(|o| !matches!(o, song_adder::Outcome::Failed { .. }))
        {
            self.songs.rescan(self.config.song_dirs.clone());
            self.rescanning_for_add = true;
        }
    }

    /// 追加した曲へ移ってよいか。選曲（練習メニューや簡易設定を開いていない）かタイトルにいるときだけ。
    /// 演奏や設定の途中では選んでいる曲を変えず、選曲に戻ってから移る
    fn can_reveal(&self) -> bool {
        self.after_close.is_none()
            && match self.screen {
                Screen::Title => true,
                Screen::Select => self.menu.is_none() && self.quick.is_none(),
                _ => false,
            }
    }

    /// 追加した曲を選曲で選ぶ。タイトルにいれば選曲へ移る
    fn show_added_song(&mut self) {
        let Some(target) = self.jump_to.take() else {
            return;
        };
        if !self.songs.reveal(&target) {
            return;
        }
        self.view.jumped();
        if matches!(self.screen, Screen::Title) {
            self.sfx.stop_title();
            self.close_then(AfterClose::Select, Duration::ZERO);
        }
    }

    fn redraw(&mut self) {
        if matches!(self.screen, Screen::Config(..)) {
            self.refresh_monitors(false);
        }
        let (Some(window), Some(state)) = (self.window.clone(), &mut self.egui_state) else {
            return;
        };
        self.perf.set_mode(perf::Mode::from_index(
            self.config.settings.choice(settings::MONITOR),
        ));
        self.perf.frames.begin();
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
        self.feed_perf();
        let paused_panel = self.quick.is_some() && matches!(self.screen, Screen::Play(..));
        self.poll_update();
        self.title.set_update(self.updater.status());
        // 「更新しています」を少し見せてから閉じる（閉じたあと入れ替えて起動し直す）
        if self
            .applying
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= APPLY_SHOW)
        {
            self.quit = true;
        }
        self.shutter.update(Instant::now());
        let egui_ctx = self.egui_ctx.clone();
        let output = egui_ctx.run_ui(input, |ui| {
            self.draw_screen(ui, &ports, scanning, &mut mouse, &mut result_actions);
            // 短い知らせはどの画面でも（F12 で画面を残したときなど）
            let ctx = ui.ctx().clone();
            if let Some(t) = &toast {
                // 一時停止の板を開いているときは、板と下の帯にかからないよう、レーンの見えている所に
                let at = if paused_panel {
                    (468.0, 300.0)
                } else {
                    (574.0, 676.0)
                };
                ui::play_toast(
                    &ctx,
                    &Canvas::overlay(
                        &ctx.layer_painter(egui::LayerId::background()),
                        ctx.content_rect(),
                    ),
                    t,
                    at,
                );
            }
            // 更新のようす（落としている間の隅の印、「更新しています」）
            self.draw_update(&Canvas::overlay(
                &ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::unique("update"),
                )),
                ctx.content_rect(),
            ));
            // ドロップで曲を追加するようす（重ねているあいだの案内、写している途中の札）
            self.draw_adding(&Canvas::overlay(
                &ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::unique("adding"),
                )),
                ctx.content_rect(),
            ));
            // シャッターはどの画面のどの窓よりも上に
            let top = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Debug,
                egui::Id::unique("shutter"),
            ));
            self.shutter
                .draw(&Canvas::overlay(&top, ctx.content_rect()), &mut self.gfx);
            self.perf.draw(&Canvas::overlay(&top, ctx.content_rect()));
        });
        self.perf.frames.built();
        if let Some(state) = &mut self.egui_state {
            state.handle_platform_output(&window, output.platform_output.clone());
        }
        self.apply_window_settings();
        let saved = self
            .gpu
            .as_mut()
            .and_then(|gpu| gpu.render(&self.egui_ctx, output));
        if let Some(gpu) = &self.gpu {
            self.perf.frames.end(gpu.waited);
        }
        // 残した画面に知らせが写らないように、知らせは残したあとに出す
        if let Some(path) = saved {
            self.toast(tf!(
                "Screenshot saved ({})",
                "画面を残しました（{}）",
                path.display()
            ));
        }
        self.select_mouse(mouse);
        for a in result_actions {
            self.result_action(a);
        }
    }

    /// 更新のようすを画面に重ねる。落としている間は隅に小さな印（タイトルは版の下に自分で描く。演奏中は出さない）、
    /// 終わる画面では入れ替えることを、「再起動して入れ替える」を選んだら「更新しています」を出す
    fn draw_update(&mut self, c: &Canvas) {
        let since = self.update_since;
        if let Some((v, at)) = &self.applying {
            update_view::applying(c, &mut self.gfx, update::current_version(), v, *at);
            return;
        }
        if let Screen::End(_) = self.screen {
            if let Some(v) = self.updater.pending_on_exit() {
                let text = tf!(
                    "v{v} will be installed after you quit",
                    "終わったあと v{v} に入れ替えます"
                );
                update_view::chip(
                    c,
                    &mut self.gfx,
                    &text,
                    None,
                    Corner::BottomRight(1264.0, 704.0),
                    since,
                );
            }
            return;
        }
        let corner = match self.screen {
            // 設定と選曲は右下の帯の端、結果は右下に案内があるので右上
            Screen::Config(..) | Screen::Select => Corner::BottomRight(1264.0, 717.0),
            Screen::Result(..) => Corner::TopRight(1264.0, 16.0),
            _ => return,
        };
        if let update::Status::Downloading { version, percent } = self.updater.status() {
            let text = tf!(
                "Downloading v{version}… {percent}%",
                "v{version} を落としています {percent}%"
            );
            let p = f32::from(*percent) / 100.0;
            update_view::chip(c, &mut self.gfx, &text, Some(p), corner, since);
        }
    }

    /// ドロップで曲を追加するようすを重ねる。ファイルを重ねているあいだは案内を、写しているあいだは隅に札を
    /// （演奏中と読み込み中は札を出さない。終わったら知らせる）
    fn draw_adding(&mut self, c: &Canvas) {
        if !self.drop_hover.is_empty() {
            if self.on_import_page() {
                drop_view::hover_import(c, &mut self.gfx, &self.drop_hover);
            } else {
                drop_view::hover(c, &mut self.gfx, &self.drop_hover, &self.config.add_dest());
            }
            return;
        }
        let Some(w) = self.adder.working().cloned() else {
            return;
        };
        let updating = matches!(self.updater.status(), update::Status::Downloading { .. });
        let corner = match self.screen {
            // 更新の印と同じ所。両方あれば上に重ねる
            Screen::Config(..) | Screen::Select => {
                Corner::BottomRight(1264.0, if updating { 683.0 } else { 717.0 })
            }
            Screen::Result(..) => Corner::TopRight(1264.0, if updating { 50.0 } else { 16.0 }),
            // 記録と基礎練習の画面は更新の印を出さないので、いつも帯の端
            Screen::Records(_) | Screen::Drills(_) => Corner::BottomRight(1264.0, 717.0),
            Screen::Title | Screen::End(_) => Corner::BottomRight(1264.0, 704.0),
            Screen::Loading(_) | Screen::Play(..) | Screen::Replay(_) => return,
        };
        drop_view::adding_chip(c, &mut self.gfx, &w, corner);
    }

    /// 今の画面を描く
    fn draw_screen(
        &mut self,
        ui: &mut egui::Ui,
        ports: &[String],
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
            Screen::Drills(m) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                m.draw(&c, &mut self.gfx);
                if let Some(msg) = &self.message {
                    ui::message_overlay(&ctx, msg);
                }
            }
            Screen::Config(view, _) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                let mut info = config_info(
                    &self.config,
                    &self.midi,
                    &self.monitor,
                    &self.songs,
                    &self.monitor_list,
                );
                info.timing = self.timing.as_ref().map(SongTiming::info);
                info.import = Some(self.import.info());
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
            Screen::Records(view) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                view.draw(&c, &mut self.gfx);
            }
            Screen::Select => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                self.view.update(&mut self.songs);
                self.view.left_bass_apart = self.config.settings.left_bass_apart();
                self.view.draw(&c, &mut self.gfx, &self.songs);
                if let Some(m) = &self.menu {
                    let sound = self.menu_sound.unwrap_or(practice_menu::Sound::Off);
                    m.draw(&c, &mut self.gfx, sound);
                }
                if let Some(q) = &mut self.quick {
                    let mut info = config_info(
                        &self.config,
                        &self.midi,
                        &self.monitor,
                        &self.songs,
                        &self.monitor_list,
                    );
                    info.song = self.songs.selected_chart().is_some();
                    info.timing = self.timing.as_ref().map(SongTiming::info);
                    let rows = config_view::rows(q.page(), &info);
                    q.draw_panel(&c, &mut self.gfx, &rows, &info);
                }
                if let Some(l) = &self.replays {
                    l.draw(&c, &mut self.gfx);
                }
                if let Some(m) = &self.message {
                    ui::message_overlay(&ctx, m);
                }
                *mouse = SelectMouse::read(&ctx, &c, &self.view);
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
                    .unwrap_or_else(|| play.view_ms(self.config.visual_offset_ms));
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
                if let Some(q) = &mut self.quick {
                    let mut info = config_info(
                        &self.config,
                        &self.midi,
                        &self.monitor,
                        &self.songs,
                        &self.monitor_list,
                    );
                    info.play = Some(play_pos(play));
                    info.timing = self.timing.as_ref().map(SongTiming::info);
                    let rows = config_view::rows(q.page(), &info);
                    q.draw_panel(&c, &mut self.gfx, &rows, &info);
                }
            }
            Screen::Replay(run) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                run.draw(
                    &c,
                    &mut self.gfx,
                    self.config.scroll_px_per_ms,
                    self.config.visual_offset_ms,
                );
            }
            Screen::Result(o, view) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                view.draw_background(&c, &mut self.gfx);
                match view.page {
                    Page::Summary => view.draw_summary(&c, &mut self.gfx, o),
                    Page::Analysis => match &o.practice {
                        Some(p) => view.draw_practice_laps(&c, &mut self.gfx, p),
                        None => *result_actions = ui::draw_result(ui, &mut self.gfx, o),
                    },
                }
                view.draw_legend(&c, &mut self.gfx, o);
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

/// 一時停止の板に出す、演奏しているところ
fn play_pos(play: &Play) -> PlayPos {
    PlayPos {
        practice: play.practice.is_some(),
        drill: play.drill.is_some(),
        at_ms: play.now_ms(),
        end_ms: play.end_ms,
        metronome: play.metronome(),
    }
}

/// 設定の画面の項目を作るのに要るもの
fn config_info<'a>(
    config: &'a Config,
    midi: &Option<MidiIn>,
    monitor: &MidiMonitor,
    songs: &SongList,
    monitors: &'a [window_mode::MonitorInfo],
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
        timing: None,
        monitors,
        import: None,
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let want = WindowWant::of(&self.config);
        let mode = want.mode;
        // 選んだモニター（無ければ、外れていれば OS が決める）
        let target = window_mode::target_in(
            event_loop.available_monitors(),
            event_loop.primary_monitor(),
            want.monitor.as_ref(),
        );
        // ウィンドウの位置と大きさ（前回の位置、設定の大きさ。選んだモニターに無ければその真ん中）。
        // 全画面やボーダーレスで開いたときは、ウィンドウに戻るときに使う
        let all = window_mode::monitors_in(
            event_loop.available_monitors(),
            event_loop.primary_monitor(),
        );
        let opening = window_mode::opening(
            self.config.window.as_ref(),
            want.size,
            &all,
            want.monitor
                .as_ref()
                .and_then(|p| window_mode::find(p, &all)),
        );
        // ボーダーレスは普通の窓で開いてから広げる（Windows では広げるまで隠しておく）
        let hide = mode == DisplayMode::Borderless && cfg!(windows);
        let mut attrs = Window::default_attributes()
            .with_title("DTXmaniaNXT")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .with_visible(!hide)
            .with_fullscreen(
                (mode == DisplayMode::Fullscreen).then(|| Fullscreen::Borderless(target.clone())),
            );
        match (mode, opening, &target) {
            (DisplayMode::Window, Some(o), _) => {
                attrs = attrs.with_position(o.pos()).with_inner_size(o.size());
            }
            // ボーダーレスと、前回の無いウィンドウは、選んだモニターの真ん中に開く
            (DisplayMode::Window | DisplayMode::Borderless, _, Some(t)) => {
                attrs = attrs.with_position(window_mode::centered(t));
            }
            _ => {}
        }
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.startup_error = Some(
                    anyhow::Error::new(e).context(t("Cannot open the window", "窓を開けません")),
                );
                event_loop.exit();
                return;
            }
        };
        app_icon::apply(&window);
        let opened = match mode {
            DisplayMode::Fullscreen => DisplayMode::Fullscreen,
            _ => DisplayMode::Window,
        };
        // ボーダーレスへ広げる前の窓の位置ではなく、前回のウィンドウの位置へ戻れるように
        let mut opened_at = None;
        window_mode::apply(&window, Some(opened), mode, &mut opened_at, target.as_ref());
        self.window_placement = opening.or(opened_at);
        if mode == DisplayMode::Window && opening.is_some_and(|o| o.maximized) {
            window.set_maximized(true);
        }
        if hide {
            window.set_visible(true);
        }
        match Gpu::new(window.clone(), want.vsync) {
            Ok(gpu) => self.gpu = Some(gpu),
            Err(e) => {
                self.startup_error =
                    Some(e.context(t("Cannot start the GPU", "GPU を使い始められません")));
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
        self.applied_window = Some(want);
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
            // ボーダーレスのまま DPI の違うモニターへ移ると、winit が窓の大きさを DPI に合わせて変えてしまうので、
            // 次のフレームでモニターに合わせ直す
            WindowEvent::ScaleFactorChanged { .. }
                if cfg!(windows)
                    && self.config.settings.display_mode() == DisplayMode::Borderless =>
            {
                self.refit_window = true;
            }
            WindowEvent::DroppedFile(path) => self.on_drop(path),
            WindowEvent::HoveredFile(path) => {
                if !self.drop_hover.contains(&path) {
                    self.drop_hover.push(path);
                }
            }
            WindowEvent::HoveredFileCancelled => self.drop_hover.clear(),
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && let PhysicalKey::Code(code) = event.physical_key
                {
                    if code == MONITOR_KEY {
                        if !event.repeat {
                            self.cycle_monitor();
                        }
                        return;
                    }
                    // どの画面でも効くキー（上流の Alt+Enter と Capture キー）
                    let alt_enter = self.modifiers.alt_key()
                        && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter);
                    if !event.repeat && (alt_enter || code == KeyCode::F11) {
                        self.toggle_fullscreen();
                        return;
                    }
                    if code == KeyCode::F12 {
                        if !event.repeat {
                            self.request_screenshot();
                        }
                        return;
                    }
                    // 押し続けたときの繰り返しは、設定の画面の矢印（左右は値を続けて変える）と、
                    // 選曲の画面の ↑↓ だけ（上流のキーの反復と同じ。選曲は v2 と同じく、上に何も開いていないとき）
                    let repeat_ok = match self.screen {
                        Screen::Config(..) => matches!(
                            code,
                            KeyCode::ArrowUp
                                | KeyCode::ArrowDown
                                | KeyCode::ArrowLeft
                                | KeyCode::ArrowRight
                        ),
                        Screen::Select => {
                            self.menu.is_none()
                                && matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown)
                        }
                        _ => false,
                    };
                    if !event.repeat || repeat_ok {
                        self.on_key(code);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.pump();
                self.redraw();
                self.after_frame();
            }
            WindowEvent::Focused(focused) => self.focused = focused,
            _ => {}
        }
        if self.quit {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // 叩いた音をすぐ鳴らすため、描画を待たずに MIDI を処理する（上限で眠っていても、MIDI が届くと起きてここに来る）
        self.pump();
        // 描画の頼みはここでも出す。描画の中で次を頼むだけだと、描画の知らせが 1 度でも来なかったとき
        // 画面が止まったまま遊びだけ進む（頼みが重なっても描画は 1 回）。上限があれば、次に描く時刻まで眠る
        let now = Instant::now();
        let (draw, flow) = match self.frame_interval() {
            None => (true, ControlFlow::Poll),
            Some(iv) if now >= self.next_frame => (true, ControlFlow::WaitUntil(now + iv)),
            Some(_) => (false, ControlFlow::WaitUntil(self.next_frame)),
        };
        if draw && let Some(w) = &self.window {
            w.request_redraw();
        }
        event_loop.set_control_flow(flow);
        if self.quit {
            event_loop.exit();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.remember_window();
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
    /// 練習メニューへのマウスの操作
    menu: MenuMouse,
}

impl SelectMouse {
    fn read(ctx: &egui::Context, c: &Canvas, view: &SelectView) -> Self {
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
            let clicked_row = clicked.and_then(|p| view.row_at(p));
            let clicked_sort = clicked.and_then(SelectView::sort_at);
            let menu = MenuMouse {
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
                menu,
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
    // MIDI が届いたら主のループを起こす（フレームレートの上限で眠っているときも、叩いた音をすぐ鳴らす）
    let proxy = std::sync::Mutex::new(event_loop.create_proxy());
    midi::set_waker(move || {
        if let Ok(p) = proxy.lock() {
            let _ = p.send_event(());
        }
    });
    let mut app = App::new(arg);
    // 前に V51 の値を書き換えたまま終わっていたら（落ちたなど）、ここで書き戻す
    module_link::restore_on_start();
    event_loop.run_app(&mut app)?;
    app.updater.apply_on_exit();
    match app.startup_error.take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_interval_takes_the_lower_of_the_limit_and_the_background() {
        assert_eq!(frame_interval(None, false), None);
        let ms =
            |d: Option<Duration>| d.map(|d| (d.as_secs_f64() * 1000.0 * 100.0).round() / 100.0);
        assert_eq!(ms(frame_interval(Some(144), false)), Some(6.94));
        // ほかの窓を見ているあいだは 30 回（上限がもっと低ければそちら）
        assert_eq!(ms(frame_interval(None, true)), Some(33.33));
        assert_eq!(ms(frame_interval(Some(240), true)), Some(33.33));
        assert_eq!(ms(frame_interval(Some(20), true)), Some(50.0));
    }

    #[test]
    fn low_latency_needs_mailbox() {
        use wgpu::PresentMode::*;
        assert_eq!(present_mode(Vsync::On, &[Fifo, Mailbox]), AutoVsync);
        assert_eq!(present_mode(Vsync::LowLatency, &[Fifo, Mailbox]), Mailbox);
        assert_eq!(present_mode(Vsync::LowLatency, &[Fifo]), AutoVsync);
        assert_eq!(present_mode(Vsync::Off, &[Fifo]), AutoNoVsync);
    }
}
