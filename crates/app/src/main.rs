//! DTXmaniaNXT 本体。
//!
//! - 曲のフォルダを探して選曲し、BGM を鳴らし、落ちてくる音符を叩いて判定する
//! - 曲の区間（小節の範囲）を選んで繰り返し練習できる。遊んだ記録から苦手な小節を出す
//! - 判定は MIDI の時刻と音の再生位置で行い、画面のフレームには左右されない
//! - 電子ドラムが無いときはキーボードで叩ける
//! - 設定（曲のフォルダ・補正値など）は config.toml に残す
//! - 画面は DTXManiaNX と同じ画像と配置で描く。画像と音はユーザーの PC にある NX のフォルダから読む
//! - 起動するとタイトル。そこから選曲と設定へ進む
//! - Setup.exe で入れたときは、起動したときに GitHub のリリースから新しい版を落とし、終えるときに入れ替える

mod audio;
mod band;
mod config;
mod config_view;
mod fonts;
mod game;
mod gfx;
mod history;
mod loading;
mod midi;
mod play_view;
mod practice;
mod practice_hud;
mod practice_popup;
mod result_training;
mod result_view;
mod select;
mod sfx;
mod songs;
mod stretch;
mod text;
mod time;
mod title_view;
mod ui;
mod update;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dtx::DrumLane;
use engine::input::{InputEvent, MidiMessage, MidiProfile, Normalizer};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::config::Config;
use crate::config_view::{ConfigAction, ConfigInfo, ConfigInput, ConfigView, Setting};
use crate::game::{Outcome, Play};
use crate::gfx::{Canvas, Gfx, Skin};
use crate::history::History;
use crate::loading::{Job, Loading};
use crate::midi::MidiIn;
use crate::play_view::PlayView;
use crate::practice::{PracticeOptions, PracticeSetup};
use crate::practice_popup::{Done, PopupInput, PopupMouse, PracticePopup};
use crate::result_view::{Page, ResultInfo, ResultView};
use crate::select::{PREVIEW_WAIT, SelectView};
use crate::sfx::{Se, Sfx};
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
            .context("create surface")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .context("no GPU adapter")?;
        log::info!("GPU: {:?}", adapter.get_info().name);
        // NX のスキンには縦 4080 の画像などがあるので、GPU が扱える一番大きい画像の大きさを使う
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .context("request device")?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("surface is not supported by the adapter")?;
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
    profile: MidiProfile,
}

impl LoadSettings {
    /// 曲を読む（時間がかかる）。`practice` があれば区間の練習
    fn load(self, path: &std::path::Path, practice: Option<PracticeOptions>) -> Result<Play> {
        let started = Instant::now();
        let mut p = match practice {
            Some(o) => Play::load_practice(path, self.input_offset_ms, self.drum_sounds, o)?,
            None => Play::load(path, self.input_offset_ms, self.drum_sounds)?,
        };
        p.set_midi_profile(self.profile);
        log::info!(
            "loaded in {:.0}ms",
            started.elapsed().as_secs_f64() * 1000.0
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
    /// 選曲や結果の画面で、電子ドラムで操作するための入力
    menu_input: Normalizer,
    /// 選曲の画面で最後に HH を叩いた時刻（2 回で難易度を変える）
    last_hh_us: Option<i64>,
    /// DTXManiaNX のフォルダ（画面の画像と音、曲のフォルダ、パッドの割り当て）
    nx: Option<nx::Install>,
    gfx: Gfx,
    /// 選曲画面の動き
    view: SelectView,
    /// 画面の効果音
    sfx: Sfx,
    /// タイトル画面の動き
    title: TitleView,
    /// 終わる（次のイベントでループを抜ける）
    quit: bool,
    /// 選曲画面で決めたもの（暗くし終えたら始める）
    decided: Option<Decided>,
    /// 選曲画面に重ねる、練習の決めごとのポップアップ
    popup: Option<Box<PracticePopup>>,
    /// 選曲画面のマウスホイールの、まだ 1 行に満たない分
    wheel: f32,
    /// 演奏中に設定と入力の情報を出す（I で切り替え）
    show_play_info: bool,
    /// 演奏中に設定を変えたときの短い知らせ（出した時刻）
    toast: Option<(String, Instant)>,
    /// 音符を自動で叩く（開発用。`DTXMANIANXT_AUTOPLAY=1`）
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

/// 選曲画面で決めたもの
enum Decided {
    Play(PathBuf),
    /// タイトルへ戻る
    Title,
}

/// 電子ドラムのノート番号の割り当て。Roland の既定に、NX で使っていた割り当て、設定の画面で変えた割り当ての順に重ねる
fn midi_profile(nx: Option<&nx::Install>, config: &Config) -> MidiProfile {
    let mut p = MidiProfile::roland_vdrums();
    if let Some(nx) = nx {
        p.apply_lanes(&nx.config.midi_notes());
    }
    p.apply_lanes(&config.pad_overrides());
    p
}

/// キーボードの割り当て（DTX Web Editor と同じ並び。Q は HH オープン）
const KEYBOARD: [(KeyCode, DrumLane); 11] = [
    (KeyCode::KeyA, DrumLane::LC),
    (KeyCode::KeyS, DrumLane::HH),
    (KeyCode::KeyQ, DrumLane::HHO),
    (KeyCode::KeyZ, DrumLane::LP),
    (KeyCode::KeyD, DrumLane::SD),
    (KeyCode::KeyG, DrumLane::HT),
    (KeyCode::KeyF, DrumLane::BD),
    (KeyCode::KeyH, DrumLane::LT),
    (KeyCode::KeyJ, DrumLane::FT),
    (KeyCode::KeyK, DrumLane::CY),
    (KeyCode::KeyL, DrumLane::RD),
];

fn key_lane(code: KeyCode) -> Option<DrumLane> {
    KEYBOARD.iter().find(|(k, _)| *k == code).map(|(_, l)| *l)
}

/// レーンを叩くキーボードのキー（設定の画面の説明に出す）
fn keyboard_key(lane: DrumLane) -> Option<String> {
    KEYBOARD
        .iter()
        .find(|(_, l)| *l == lane)
        .map(|(k, _)| format!("{k:?}").trim_start_matches("Key").to_owned())
}

/// 使う NX（画面の画像と音、Config.ini）。NX から写したものがあればそれを使う。
/// まだ写していなければ、設定の NX のフォルダか、NXT の exe が置いてある NX のフォルダから写す
fn find_nx(config: &mut Config) -> Option<nx::Install> {
    if let Some(i) = Config::nx_copy_dir().and_then(|d| nx::Install::open(&d)) {
        return Some(i);
    }
    let original = match config.nx_dir.as_deref().and_then(nx::Install::open) {
        Some(i) => i,
        None => {
            let exe_dir = std::env::current_exe().ok()?.parent()?.to_owned();
            let i = nx::Install::open(&exe_dir).filter(|i| nx::is_skin(&i.skin))?;
            config.nx_dir = Some(exe_dir);
            i
        }
    };
    let (i, _) = adopt_nx(config, &original);
    config.save();
    Some(i)
}

/// NX を使い始める：スキンと Config.ini を設定のフォルダへ写し、曲のフォルダを足す。
/// 写したもの（写せなければ NX そのもの）と、曲のフォルダを足したかを返す。保存は呼ぶ側で
fn adopt_nx(config: &mut Config, original: &nx::Install) -> (nx::Install, bool) {
    let mut added = false;
    for d in original.song_dirs().into_iter().filter(|d| d.is_dir()) {
        added |= config.add_song_dir(d);
    }
    let copied = Config::nx_copy_dir().map(|d| nx::copy_install(original, &d));
    let i = match copied {
        Some(Ok(i)) => {
            log::info!(
                "copied the NX skin {} to {}",
                original.skin.display(),
                i.root.display()
            );
            i
        }
        Some(Err(e)) => {
            log::warn!("cannot copy the NX skin: {e}");
            original.clone()
        }
        None => original.clone(),
    };
    (i, added)
}

/// 2 回叩きと見なす間（上流と同じ 0.5 秒）
const DOUBLE_HIT_US: i64 = 500_000;

impl App {
    fn new(arg: Option<PathBuf>) -> Self {
        let midi = match MidiIn::open_all() {
            Ok(m) => Some(m),
            Err(e) => {
                log::warn!("MIDI is off: {e:#}");
                None
            }
        };
        let mut config = Config::load();
        let nx = find_nx(&mut config);
        if let Some(n) = &nx {
            log::info!(
                "DTXManiaNX: {} (skin {})",
                n.root.display(),
                n.skin.display()
            );
        }
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
        let gfx = Gfx::new(&egui_ctx, nx.as_ref().map(|n| n.skin.clone()));
        let mut sfx = Sfx::new();
        sfx.load(&gfx.skin);
        let menu_input = Normalizer::new(midi_profile(nx.as_ref(), &config));
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
            history: History::open(),
            pending,
            message: None,
            menu_input,
            last_hh_us: None,
            nx,
            gfx,
            view: SelectView::new(),
            sfx,
            title: TitleView::new(),
            quit: false,
            decided: None,
            popup: None,
            wheel: 0.0,
            show_play_info: false,
            toast: None,
            autoplay: std::env::var_os("DTXMANIANXT_AUTOPLAY").is_some_and(|v| v == "1"),
            updater,
        }
    }

    /// 選曲画面に戻る（フェードと登場のアニメから）
    fn go_to_select(&mut self) {
        self.sfx.stop_title();
        self.popup = None;
        self.screen = Screen::Select;
        self.view.enter();
    }

    /// タイトルへ（白からのフェードから）
    fn go_to_title(&mut self) {
        self.message = None;
        self.screen = Screen::Title;
        self.title.enter();
    }

    /// 設定の画面を開く。`from_select` なら出たら選曲へ戻る
    fn open_config(&mut self, from_select: bool) {
        self.sfx.preview(None);
        self.screen = Screen::Config(Box::new(ConfigView::new()), from_select);
    }

    /// 終わる前の画面を出す（終わりの音が鳴り終えたら閉じる）
    fn begin_end(&mut self) {
        let wait = self.sfx.play_end();
        self.screen = Screen::End(Ending::new(wait));
    }

    /// パッドの割り当てが変わったので、メニューの操作に使う割り当ても作り直す
    fn refresh_pads(&mut self) {
        self.menu_input = Normalizer::new(midi_profile(self.nx.as_ref(), &self.config));
    }

    /// DTXManiaNX のフォルダを使う
    fn set_nx(&mut self, dir: PathBuf) {
        let Some(i) = nx::Install::open(&dir) else {
            self.message = Some(format!(
                "DTXManiaNX のフォルダではありません（System フォルダがありません）: {}",
                dir.display()
            ));
            return;
        };
        log::info!(
            "DTXManiaNX: {} (skin {})",
            i.root.display(),
            i.skin.display()
        );
        self.config.nx_dir = Some(dir);
        // 選ぶたびに写し直す（NX でスキンや割り当てを変えたときも、選び直せば揃う）
        let (i, added) = adopt_nx(&mut self.config, &i);
        self.config.save();
        self.gfx.skin = Skin::new(&self.egui_ctx, Some(i.skin.clone()));
        self.sfx.load(&self.gfx.skin);
        self.menu_input = Normalizer::new(midi_profile(Some(&i), &self.config));
        self.nx = Some(i);
        self.message = None;
        if added {
            self.songs.rescan(self.config.song_dirs.clone());
        }
        self.view.enter();
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
        self.screen = Screen::Loading(Box::new(Loading::new(
            &path,
            &label,
            self.gfx.skin.dir(),
            wait,
            job,
        )));
    }

    /// 曲を読むのに使う設定（読むスレッドへ渡す）
    fn load_settings(&self) -> LoadSettings {
        LoadSettings {
            input_offset_ms: self.config.input_offset_ms,
            drum_sounds: self.config.drum_sounds,
            profile: midi_profile(self.nx.as_ref(), &self.config),
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
        let lag = play_view::LagDigits::from_nx(self.nx.as_ref().map(|n| &n.config));
        let view = PlayView::new(&play, self.gfx.skin.dir(), &label, lag);
        self.message = None;
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

    /// ゲージが尽きたら途中で終わるか（設定に無ければ NX の Config.ini、それも無ければ上流と同じく終わる）
    fn stage_failed_enabled(&self) -> bool {
        self.config
            .stage_failed
            .or_else(|| self.nx.as_ref().and_then(|n| n.config.stage_failed))
            .unwrap_or(true)
    }

    /// 演奏中に設定を変えたことを知らせる
    fn toast(&mut self, text: String) {
        self.toast = Some((text, Instant::now()));
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

    /// 曲（練習）を終える。結果の画面へ
    fn finish_play(&mut self) {
        let Screen::Play(p, view) = &self.screen else {
            return;
        };
        let finish = view.finish(p, p.now_ms());
        let practice = p.practice.is_some();
        // Esc でやめたときも結果は出す（記録は「最後まで遊んでいない」として残す）
        let cleared = p.finished();
        let session = self.history.save(p, Some(play_score(&finish, cleared)));
        // New Record と前回のベストのバーは、今回より前のプレイと比べる
        let best = self.history.best_score(&p.path, session);
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
            .nx
            .as_ref()
            .filter(|n| n.config.show_lag_hit_count)
            .map(|n| n.config.lag_color_swapped);
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
    }

    /// 遊んでいる途中の曲があれば、そこまでの記録を残す（STAGE FAILED や、途中でやめたとき）
    fn save_unfinished(&mut self) {
        if let Screen::Play(p, view) = &self.screen {
            let now = view.frozen_ms().unwrap_or_else(|| p.now_ms());
            let finish = view.finish(p, now);
            self.history.save(p, Some(play_score(&finish, false)));
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
        let mut assigned = None;
        match (&self.midi, &mut self.screen) {
            (Some(midi), Screen::Play(play, _)) => {
                for raw in midi.drain() {
                    play.on_midi(raw);
                }
            }
            (Some(midi), Screen::Config(view, _)) if view.waiting_pad() => {
                // パッドの割り当て：次に叩いたノートをそのまま使う
                let note = midi
                    .drain()
                    .filter_map(|raw| match MidiMessage::parse(&raw.bytes)? {
                        MidiMessage::NoteOn { note, .. } => Some(note),
                        _ => None,
                    })
                    .next();
                if let Some(note) = note {
                    assigned = Some(view.pad_hit(note));
                }
            }
            (Some(midi), _) => {
                let input = &mut self.menu_input;
                let hits: Vec<(DrumLane, i64)> = midi
                    .drain()
                    .filter_map(|raw| {
                        let msg = MidiMessage::parse(&raw.bytes)?;
                        match input.feed(msg, raw.arrival_us)? {
                            InputEvent::Hit {
                                lane: Some(lane),
                                host_us,
                                ..
                            } => Some((lane, host_us)),
                            _ => None,
                        }
                    })
                    .collect();
                for (lane, at) in hits {
                    self.on_menu_pad(lane, at);
                }
            }
            _ => {}
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
        if let Screen::Play(play, view) = &mut self.screen {
            if view.frozen_ms().is_some() {
                // STAGE FAILED の途中は譜面を進めない
                failed_out = view.failure_done();
            } else {
                // 判定の線より上に見えている時間
                play.view_ahead_ms = play_view::view_ahead_ms(self.config.scroll_px_per_ms);
                if self.autoplay {
                    play.autoplay();
                }
                play.update();
                view.feed(play);
                if stage_failed && play.practice.is_none() && view.gauge_empty() {
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
            self.finish_play();
        }
        if matches!(self.screen, Screen::Select) {
            if self.view.faded_out()
                && let Some(d) = self.decided.take()
            {
                match d {
                    Decided::Play(p) => self.start(p),
                    Decided::Title => self.go_to_title(),
                }
            } else if !self.view.fading_out() {
                // 選んでから少し待ってプレビューの音を鳴らす（上流 CActSelectPresound）
                let preview = if self.view.settled_for(&self.songs, PREVIEW_WAIT) {
                    self.view.preview_path(&self.songs)
                } else {
                    None
                };
                self.sfx.preview(preview);
            }
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
                match self.title.faded_out() {
                    Some(TitleItem::GameStart) => self.go_to_select(),
                    Some(TitleItem::Config) => self.open_config(false),
                    _ => {}
                }
            }
            Screen::Config(view, _) => {
                if let Some(a) = view.poll() {
                    self.config_action(a);
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
            }
            Some(TitleItem::Config) => self.sfx.play(Se::Decide),
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

    /// 設定の画面の操作
    fn config_input(&mut self, input: ConfigInput) {
        let Screen::Config(view, _) = &mut self.screen else {
            return;
        };
        let info = config_info(&self.config, self.nx.as_ref(), &self.midi, &self.songs);
        let items = config_view::items(view.list(), &info);
        let r = view.input(input, &items);
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
            ConfigAction::ChooseNx => {
                if let Some(dir) = ui::pick_folder("DTXManiaNX のフォルダを選ぶ") {
                    self.set_nx(dir);
                }
            }
            ConfigAction::AddSongDir => {
                if let Some(dir) = ui::pick_folder("曲のフォルダを選ぶ") {
                    self.add_song_dir(dir);
                }
            }
            ConfigAction::RemoveSongDir(i) => {
                if i < self.config.song_dirs.len() {
                    self.config.song_dirs.remove(i);
                    self.config.save();
                    self.songs.rescan(self.config.song_dirs.clone());
                }
            }
            ConfigAction::Rescan => {
                if !self.config.song_dirs.is_empty() {
                    self.songs.rescan(self.config.song_dirs.clone());
                }
            }
            ConfigAction::ReconnectMidi => {
                // 先に閉じてから開き直す（同じポートを二重に開かない）
                self.midi = None;
                self.midi = MidiIn::open_all()
                    .map_err(|e| log::warn!("MIDI is off: {e:#}"))
                    .ok();
            }
            ConfigAction::Nudge(s, d) => {
                let c = &mut self.config;
                let r = config_view::OFFSET_RANGE;
                let d = f64::from(d);
                match s {
                    Setting::InputAdjust => {
                        c.input_offset_ms = (c.input_offset_ms.round() + d).clamp(-r, r);
                    }
                    Setting::VisualAdjust => {
                        c.visual_offset_ms = (c.visual_offset_ms.round() + d).clamp(-r, r);
                    }
                    Setting::ScrollSpeed => {
                        // 演奏中の ↑↓ と同じ 0.05 刻み
                        let steps = (f64::from(c.scroll_px_per_ms) * 20.0).round() + d;
                        c.scroll_px_per_ms = (steps / 20.0).clamp(0.1, 3.0) as f32;
                    }
                    Setting::StageFailed | Setting::HitSound | Setting::AutoUpdate => return,
                }
                c.save();
            }
            ConfigAction::Toggle(s) => {
                let c = &mut self.config;
                match s {
                    // ON → OFF → NX に従う → ON
                    Setting::StageFailed => {
                        c.stage_failed = match c.stage_failed {
                            Some(true) => Some(false),
                            Some(false) => None,
                            None => Some(true),
                        };
                    }
                    Setting::HitSound => c.drum_sounds = !c.drum_sounds,
                    Setting::AutoUpdate => c.auto_update = !c.auto_update,
                    _ => return,
                }
                c.save();
            }
            ConfigAction::AssignPad(lane, note) => {
                self.config.assign_pad(note, lane);
                self.config.save();
                self.refresh_pads();
            }
            ConfigAction::ResetPads => {
                self.config.pad_notes.clear();
                self.config.save();
                self.refresh_pads();
            }
            ConfigAction::Leave => {
                self.message = None;
                if matches!(self.screen, Screen::Config(_, true)) {
                    self.go_to_select();
                } else {
                    self.go_to_title();
                }
            }
        }
    }

    fn on_key(&mut self, code: KeyCode) {
        let at = host_us();
        match &mut self.screen {
            Screen::Title => self.on_title_key(code),
            Screen::Config(..) => {
                let input = match code {
                    KeyCode::ArrowUp => ConfigInput::Up,
                    KeyCode::ArrowDown => ConfigInput::Down,
                    KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => ConfigInput::Decide,
                    KeyCode::Escape | KeyCode::Backspace => ConfigInput::Cancel,
                    _ => return,
                };
                self.config_input(input);
            }
            Screen::End(_) => {}
            Screen::Loading(_) => {
                // 上流と同じく Esc で読み込みをやめて選曲に戻る
                if code == KeyCode::Escape {
                    self.sfx.stop_loading();
                    self.go_to_select();
                }
            }
            Screen::Play(play, _) => {
                if let Some(lane) = key_lane(code) {
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
        let Screen::Play(play, _) = &mut self.screen else {
            return;
        };
        let toast = match code {
            KeyCode::Escape => {
                self.finish_play();
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
            KeyCode::Comma | KeyCode::Period if play.practice.is_some() => {
                play.nudge_speed(if code == KeyCode::Comma { -1 } else { 1 });
                let speed = play.practice.as_ref().map_or(100, |p| p.ladder().speed());
                format!("次の周から速度 {speed}%")
            }
            KeyCode::KeyM => {
                play.drum_sounds = !play.drum_sounds;
                self.config.drum_sounds = play.drum_sounds;
                self.config.save();
                format!(
                    "譜面の音を{}",
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
        if !matches!(self.screen, Screen::Select) || self.view.fading_out() {
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
    }

    fn on_select_key(&mut self, code: KeyCode) {
        if self.view.fading_out() {
            return;
        }
        if self.popup.is_some() {
            self.on_popup_key(code);
            return;
        }
        if code == KeyCode::F1 {
            self.sfx.play(Se::Decide);
            self.open_config(true);
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
                    // 上流と同じく一番上で Esc ならタイトルへ
                    self.sfx.preview(None);
                    self.decided = Some(Decided::Title);
                    self.view.fade_out();
                }
            }
            KeyCode::ArrowLeft => self.change_level(-1),
            KeyCode::ArrowRight => self.change_level(1),
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
        match &self.screen {
            Screen::Title => match lane {
                DrumLane::HT => self.title_move(-1),
                DrumLane::LT => self.title_move(1),
                DrumLane::CY | DrumLane::RD => self.title_decide(),
                _ => {}
            },
            Screen::Config(..) => {
                let input = match lane {
                    DrumLane::HT => ConfigInput::Up,
                    DrumLane::LT => ConfigInput::Down,
                    DrumLane::CY | DrumLane::RD => ConfigInput::Decide,
                    DrumLane::LC => ConfigInput::Cancel,
                    _ => return,
                };
                self.config_input(input);
            }
            Screen::End(_) => {}
            Screen::Select => {
                if self.view.fading_out() {
                    return;
                }
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
                    DrumLane::CY | DrumLane::RD => self.select_action(ui::SelectAction::Enter),
                    // 一番上で LC を叩いてもアプリは終わらない
                    DrumLane::LC => self.select_action(ui::SelectAction::Back),
                    DrumLane::HH | DrumLane::HHO => {
                        if self.last_hh_us.is_some_and(|t| at_us - t <= DOUBLE_HIT_US) {
                            self.last_hh_us = None;
                            let before = self.songs.level;
                            self.songs.cycle_level();
                            if self.songs.level != before {
                                self.sfx.play(Se::Change);
                            }
                            self.remember_level();
                        } else {
                            self.last_hh_us = Some(at_us);
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

    fn change_level(&mut self, delta: isize) {
        let before = self.songs.level;
        self.songs.change_level(delta);
        if self.songs.level != before {
            self.sfx.play(Se::Change);
        }
        self.remember_level();
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
                        self.sfx.play(Se::Decide);
                        self.sfx.preview(None);
                        self.decided = Some(Decided::Play(path));
                        self.view.fade_out();
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
        let Screen::Result(o, _) = &self.screen else {
            return;
        };
        let path = o.path.clone();
        let options = o.practice.as_ref().map(|p| p.options);
        match action {
            ui::ResultAction::Back => {
                // 上流と同じく戻る音を鳴らして選曲へ
                self.sfx.play(Se::Cancel);
                self.go_to_select();
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
                self.open_setup(path, previous);
            }
        }
    }

    fn on_drop(&mut self, path: PathBuf) {
        if path.is_dir() {
            if nx::is_install(&path) {
                self.set_nx(path);
            } else {
                self.add_song_dir(path);
            }
        } else if ui::is_chart_file(&path) {
            self.start(path);
        } else {
            self.message = Some(format!(
                "DTX ファイルか曲のフォルダ、DTXManiaNX のフォルダをドロップしてください: {}",
                path.display()
            ));
        }
    }

    fn redraw(&mut self) {
        let (Some(window), Some(state)) = (&self.window, &mut self.egui_state) else {
            return;
        };
        let input = state.take_egui_input(window);
        let ports = self
            .midi
            .as_ref()
            .map(|m| m.ports.clone())
            .unwrap_or_default();
        let mut mouse = SelectMouse::default();
        let mut result_actions = Vec::new();
        let toast = self
            .toast
            .as_ref()
            .filter(|(_, at)| at.elapsed() < TOAST_FOR)
            .map(|(t, _)| t.clone());
        let scanning = self.songs.scanning().is_some();
        let update = self.updater.ready().map(str::to_owned);
        let output = self.egui_ctx.run_ui(input, |ui| match &mut self.screen {
            Screen::Title => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                self.title.set_update(update.as_deref());
                self.title.draw(&c, &mut self.gfx, scanning);
            }
            Screen::Config(view, _) => {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::background());
                let c = Canvas::new(&painter, ctx.content_rect());
                self.gfx.begin(&c, ctx.pixels_per_point());
                let info = config_info(&self.config, self.nx.as_ref(), &self.midi, &self.songs);
                let items = config_view::items(view.list(), &info);
                view.draw(&c, &mut self.gfx, &items, scanning);
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
                if let Some(m) = &self.message {
                    ui::message_overlay(&ctx, m);
                }
                mouse = SelectMouse::read(&ctx, &c);
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
                        midi_ports: &ports,
                        show_info: self.show_play_info,
                    },
                );
                if let Some(t) = &toast {
                    ui::play_toast(&ctx, &c, t);
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
                        result_actions = ui::draw_result(ui, o, &self.config);
                    }
                }
                // フェードは egui の窓より上に
                let top = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::unique("result fade"),
                ));
                view.draw_fade(&Canvas::overlay(&top, ctx.content_rect()));
            }
        });
        state.handle_platform_output(window, output.platform_output.clone());
        if let Some(gpu) = &mut self.gpu {
            gpu.render(&self.egui_ctx, output);
        }
        self.select_mouse(mouse);
        for a in result_actions {
            self.result_action(a);
        }
    }
}

/// 設定の画面の項目を作るのに要るもの
fn config_info<'a>(
    config: &'a Config,
    nx: Option<&'a nx::Install>,
    midi: &'a Option<MidiIn>,
    songs: &SongList,
) -> ConfigInfo<'a> {
    ConfigInfo {
        config,
        nx_root: config.nx_dir.as_deref(),
        nx_found: config.nx_dir.as_deref().is_some_and(nx::is_install),
        nx_copied: nx.is_some_and(|n| Config::nx_copy_dir().is_some_and(|d| n.root == d)),
        nx_stage_failed: nx.and_then(|n| n.config.stage_failed),
        lanes: midi_profile(nx, config).lane_notes(),
        midi_ports: midi.as_ref().map_or(&[], |m| m.ports.as_slice()),
        scanning: songs.scanning(),
        song_count: songs.library.songs,
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
                log::error!("cannot open a window: {e}");
                event_loop.exit();
                return;
            }
        };
        match Gpu::new(window.clone()) {
            Ok(gpu) => self.gpu = Some(gpu),
            Err(e) => {
                log::error!("cannot start the GPU: {e:#}");
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
                    // 押し続けたときの繰り返しは、設定の画面の ↑↓ だけ（上流のキーの反復と同じ）
                    let repeat_ok = matches!(self.screen, Screen::Config(..))
                        && matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown);
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
    /// 押した曲バーが、選んでいる曲から何本先か
    clicked_row: Option<isize>,
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
            let clicked_row = i
                .pointer
                .primary_clicked()
                .then(|| i.pointer.interact_pos())
                .flatten()
                .and_then(|p| SelectView::row_at(c.to_nx(p)));
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
                popup,
            }
        })
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // インストールなどで Velopack から呼ばれたときは、ここで仕事をして終わる
    update::run_hooks();
    let _ = host_us();
    let arg = std::env::args_os().nth(1).map(PathBuf::from);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(arg);
    event_loop.run_app(&mut app)?;
    app.updater.apply_on_exit(app.config.auto_update);
    Ok(())
}
