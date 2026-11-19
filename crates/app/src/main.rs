//! DTXmaniaNXT 本体。
//!
//! - 曲のフォルダを探して選曲し、BGM を鳴らし、落ちてくる音符を叩いて判定する
//! - 曲の区間（小節の範囲）を選んで繰り返し練習できる。遊んだ記録から苦手な小節を出す
//! - 判定は MIDI の時刻と音の再生位置で行い、画面のフレームには左右されない
//! - 電子ドラムが無いときはキーボードで叩ける
//! - 設定（曲のフォルダ・補正値など）は config.toml に残す

mod audio;
mod config;
mod fonts;
mod game;
mod history;
mod midi;
mod practice;
mod songs;
mod stretch;
mod time;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use dtx::DrumLane;
use engine::input::{InputEvent, MidiMessage, MidiProfile, Normalizer};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::config::Config;
use crate::game::{Outcome, Play};
use crate::history::History;
use crate::midi::MidiIn;
use crate::practice::{PracticeOptions, PracticeSetup};
use crate::songs::SongList;
use crate::time::host_us;

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    egui_renderer: egui_wgpu::Renderer,
}

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
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
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
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            other => {
                log::warn!("skip frame: {other:?}");
                self.surface.configure(&self.device, &self.config);
                return;
            }
        };
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

/// いまの画面
enum Screen {
    Select,
    /// 練習する区間を選ぶ
    Setup(Box<PracticeSetup>),
    /// 遊んでいる（練習中も）
    Play(Box<Play>),
    Result(Box<Outcome>),
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
    /// 次の描画で、選んでいる行が見えるように一覧を動かす
    scroll_to_cursor: bool,
    /// 選曲や結果の画面で、電子ドラムで操作するための入力
    menu_input: Normalizer,
    /// 選曲の画面で最後に HH を叩いた時刻（2 回で難易度を変える）
    last_hh_us: Option<i64>,
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
        Self {
            window: None,
            gpu: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            midi,
            screen: Screen::Select,
            songs,
            config,
            history: History::open(),
            pending,
            message: None,
            scroll_to_cursor: true,
            menu_input: Normalizer::new(MidiProfile::roland_vdrums()),
            last_hh_us: None,
        }
    }

    fn start(&mut self, path: PathBuf) {
        // やり直しなどで途中の曲を捨てるときも、そこまでの記録は残す
        self.save_unfinished();
        // 前の曲の音を止めてから読む
        self.screen = Screen::Select;
        match Play::load(&path, self.config.input_offset_ms, self.config.drum_sounds) {
            Ok(p) => {
                self.message = None;
                self.screen = Screen::Play(Box::new(p));
            }
            Err(e) => {
                log::error!("{e:#}");
                self.message = Some(format!("読み込めませんでした: {e:#}"));
            }
        }
    }

    /// 区間を選ぶ画面を開く。`previous` があればその区間とつなぎ方を選んでおく
    fn open_setup(&mut self, path: PathBuf, previous: Option<PracticeOptions>) {
        self.save_unfinished();
        self.screen = Screen::Select;
        match PracticeSetup::load(&path, &self.history, previous, self.config.practice_prefs()) {
            Ok(s) => {
                self.message = None;
                self.screen = Screen::Setup(Box::new(s));
            }
            Err(e) => {
                log::error!("{e:#}");
                self.message = Some(format!("読み込めませんでした: {e:#}"));
            }
        }
    }

    /// 区間の練習を始める
    fn start_practice(&mut self, path: PathBuf, options: PracticeOptions) {
        self.save_unfinished();
        self.screen = Screen::Select;
        match Play::load_practice(
            &path,
            self.config.input_offset_ms,
            self.config.drum_sounds,
            options,
        ) {
            Ok(mut p) => {
                let band = self
                    .history
                    .weak_map(&path, p.notes_per_measure(), None)
                    .weakness;
                if let Some(pr) = p.practice.as_mut() {
                    pr.band = band;
                }
                self.message = None;
                self.screen = Screen::Play(Box::new(p));
            }
            Err(e) => {
                log::error!("{e:#}");
                self.message = Some(format!("読み込めませんでした: {e:#}"));
            }
        }
    }

    /// 曲（練習）を終える。結果の画面へ
    fn finish_play(&mut self) {
        if let Screen::Play(p) = &self.screen {
            self.history.save(p);
            let mut outcome = p.outcome();
            if outcome.practice.is_none() {
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
            self.screen = Screen::Result(Box::new(outcome));
        }
    }

    /// 遊んでいる途中の曲があれば、そこまでの記録を残す
    fn save_unfinished(&mut self) {
        if let Screen::Play(p) = &self.screen {
            self.history.save(p);
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
        match (&self.midi, &mut self.screen) {
            (Some(midi), Screen::Play(play)) => {
                for raw in midi.drain() {
                    play.on_midi(raw);
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
        if let Screen::Play(play) = &mut self.screen {
            // 判定の線（画面の上から 85%）より上に見えている時間
            if let Some(w) = &self.window {
                let height = w.inner_size().height as f64 / w.scale_factor();
                play.view_ahead_ms =
                    height * 0.85 / f64::from(self.config.scroll_px_per_ms.max(0.05));
            }
            play.update();
            if play.finished() {
                self.finish_play();
            }
        }
    }

    fn on_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode) {
        let at = host_us();
        match &mut self.screen {
            Screen::Play(play) => {
                if let Some(lane) = key_lane(code) {
                    play.on_key(lane, at);
                    return;
                }
                self.on_play_key(code);
            }
            Screen::Select => self.on_select_key(event_loop, code),
            Screen::Setup(_) => self.on_setup_key(code),
            Screen::Result(_) => match code {
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape => {
                    self.result_action(ui::ResultAction::Back);
                }
                KeyCode::KeyR => self.result_action(ui::ResultAction::Retry),
                KeyCode::KeyP => self.result_action(ui::ResultAction::Practice(None)),
                _ => {}
            },
        }
    }

    fn on_setup_key(&mut self, code: KeyCode) {
        use ui::SetupAction as A;
        let Screen::Setup(s) = &self.screen else {
            return;
        };
        let action = match code {
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => A::Start,
            KeyCode::Escape | KeyCode::Backspace => A::Back,
            KeyCode::ArrowLeft => A::Shift(-1),
            KeyCode::ArrowRight => A::Shift(1),
            KeyCode::ArrowUp => A::Resize(1),
            KeyCode::ArrowDown => A::Resize(-1),
            KeyCode::KeyT => A::Style(match s.style {
                engine::practice::LoopStyle::Connect => engine::practice::LoopStyle::Restart,
                engine::practice::LoopStyle::Restart => engine::practice::LoopStyle::Connect,
            }),
            KeyCode::KeyL => A::ToggleLadder,
            KeyCode::Comma => A::Speed(-1),
            KeyCode::Period => A::Speed(1),
            _ => return,
        };
        self.setup_action(action);
    }

    fn setup_action(&mut self, action: ui::SetupAction) {
        use ui::SetupAction as A;
        let Screen::Setup(s) = &mut self.screen else {
            return;
        };
        match action {
            A::Select(sec) => s.section = Some(sec),
            A::Shift(d) => s.shift(d),
            A::Resize(d) => s.resize(d),
            A::Style(style) => s.style = style,
            A::Rule(rule) => s.rule = rule,
            A::ToggleLadder => s.toggle_ladder(),
            A::Speed(d) => s.nudge_speed(d),
            A::Target(t) => s.set_target(t),
            A::Pitch(p) => s.pitch = p,
            A::Start => {
                if let Some(o) = s.options() {
                    let path = s.path.clone();
                    self.start_practice(path, o);
                }
                return;
            }
            A::Back => {
                self.screen = Screen::Select;
                self.scroll_to_cursor = true;
                return;
            }
        }
        // 区間のほかの決めごとは次の練習でも使う
        let prefs = s.prefs();
        if prefs != self.config.practice_prefs() {
            self.config.set_practice_prefs(prefs);
            self.config.save();
        }
    }

    fn on_play_key(&mut self, code: KeyCode) {
        let Screen::Play(play) = &mut self.screen else {
            return;
        };
        match code {
            KeyCode::Escape => self.finish_play(),
            KeyCode::KeyR => {
                let path = play.path.clone();
                match play.practice.as_ref().map(|p| p.restart_options()) {
                    Some(o) => self.start_practice(path, o),
                    None => self.start(path),
                }
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight if play.practice.is_some() => {
                // 区間を 1 小節ずらして始め直す
                let d = if code == KeyCode::ArrowLeft { -1 } else { 1 };
                let path = play.path.clone();
                if let Some(mut o) = play.practice.as_ref().map(|p| p.restart_options()) {
                    o.section = o.section.shifted(d, play.notes_per_measure().len());
                    self.start_practice(path, o);
                }
            }
            KeyCode::Comma | KeyCode::Period if play.practice.is_some() => {
                play.nudge_speed(if code == KeyCode::Comma { -1 } else { 1 });
            }
            KeyCode::KeyM => {
                play.drum_sounds = !play.drum_sounds;
                self.config.drum_sounds = play.drum_sounds;
                self.config.save();
            }
            KeyCode::Minus | KeyCode::Equal => {
                let d = if code == KeyCode::Minus { -1.0 } else { 1.0 };
                play.input_offset_ms += d;
                self.config.input_offset_ms = play.input_offset_ms;
                self.config.save();
            }
            KeyCode::BracketLeft | KeyCode::BracketRight => {
                let d = if code == KeyCode::BracketLeft {
                    -1.0
                } else {
                    1.0
                };
                self.config.visual_offset_ms += d;
                self.config.save();
            }
            KeyCode::ArrowUp | KeyCode::ArrowDown => {
                let d = if code == KeyCode::ArrowUp {
                    0.05
                } else {
                    -0.05
                };
                self.config.scroll_px_per_ms = (self.config.scroll_px_per_ms + d).clamp(0.1, 3.0);
                self.config.save();
            }
            _ => {}
        }
    }

    fn on_select_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode) {
        let moved = match code {
            KeyCode::ArrowUp => {
                self.songs.move_by(-1);
                true
            }
            KeyCode::ArrowDown => {
                self.songs.move_by(1);
                true
            }
            KeyCode::PageUp => {
                self.songs.move_by(-10);
                true
            }
            KeyCode::PageDown => {
                self.songs.move_by(10);
                true
            }
            _ => false,
        };
        if moved {
            self.scroll_to_cursor = true;
            return;
        }
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                self.select_action(ui::SelectAction::Enter)
            }
            KeyCode::Backspace => self.select_action(ui::SelectAction::Back),
            KeyCode::Escape => {
                if !self.songs.back() {
                    event_loop.exit();
                }
                self.scroll_to_cursor = true;
            }
            KeyCode::ArrowLeft => self.change_level(-1),
            KeyCode::ArrowRight => self.change_level(1),
            KeyCode::F5 => self.select_action(ui::SelectAction::Rescan),
            KeyCode::KeyP => self.select_action(ui::SelectAction::Practice),
            _ => {}
        }
    }

    /// 選曲と結果の画面での電子ドラムの操作（上流と同じ割り当て）。
    /// HT で上、LT で下、CY か RD で決定、LC で戻る、HH を 2 回で難易度
    fn on_menu_pad(&mut self, lane: DrumLane, at_us: i64) {
        match &self.screen {
            Screen::Select => match lane {
                DrumLane::HT => {
                    self.songs.move_by(-1);
                    self.scroll_to_cursor = true;
                }
                DrumLane::LT => {
                    self.songs.move_by(1);
                    self.scroll_to_cursor = true;
                }
                DrumLane::CY | DrumLane::RD => self.select_action(ui::SelectAction::Enter),
                // 一番上で LC を叩いてもアプリは終わらない
                DrumLane::LC => self.select_action(ui::SelectAction::Back),
                DrumLane::HH | DrumLane::HHO => {
                    if self.last_hh_us.is_some_and(|t| at_us - t <= DOUBLE_HIT_US) {
                        self.last_hh_us = None;
                        self.songs.cycle_level();
                        self.remember_level();
                    } else {
                        self.last_hh_us = Some(at_us);
                    }
                }
                _ => {}
            },
            Screen::Setup(_) => {
                let action = match lane {
                    DrumLane::HT => ui::SetupAction::Shift(-1),
                    DrumLane::LT => ui::SetupAction::Shift(1),
                    DrumLane::CY | DrumLane::RD => ui::SetupAction::Start,
                    DrumLane::LC => ui::SetupAction::Back,
                    _ => return,
                };
                self.setup_action(action);
            }
            Screen::Result(_) => {
                if matches!(lane, DrumLane::CY | DrumLane::RD | DrumLane::LC) {
                    self.screen = Screen::Select;
                }
            }
            Screen::Play(_) => {}
        }
    }

    fn change_level(&mut self, delta: isize) {
        self.songs.change_level(delta);
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
            A::Select(i) => self.songs.select(i),
            A::Enter => {
                if let Some(path) = self.songs.enter() {
                    self.start(path);
                }
                self.scroll_to_cursor = true;
            }
            A::Back => {
                self.songs.back();
                self.scroll_to_cursor = true;
            }
            A::Level(l) => {
                self.songs.level = l;
                self.remember_level();
            }
            A::AddFolder => {
                if let Some(dir) = ui::pick_folder() {
                    self.add_song_dir(dir);
                }
            }
            A::RemoveFolder(i) => {
                if i < self.config.song_dirs.len() {
                    self.config.song_dirs.remove(i);
                    self.config.save();
                    self.songs.rescan(self.config.song_dirs.clone());
                }
            }
            A::Rescan => {
                if !self.config.song_dirs.is_empty() {
                    self.songs.rescan(self.config.song_dirs.clone());
                }
            }
            A::Practice => {
                if let Some(path) = self.songs.selected_chart() {
                    self.open_setup(path, None);
                }
            }
        }
    }

    fn result_action(&mut self, action: ui::ResultAction) {
        let Screen::Result(o) = &self.screen else {
            return;
        };
        let path = o.path.clone();
        let options = o.practice.as_ref().map(|p| p.options);
        match action {
            ui::ResultAction::Back => {
                self.screen = Screen::Select;
                self.scroll_to_cursor = true;
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
        let (Some(window), Some(state)) = (&self.window, &mut self.egui_state) else {
            return;
        };
        let input = state.take_egui_input(window);
        let ports = self
            .midi
            .as_ref()
            .map(|m| m.ports.clone())
            .unwrap_or_default();
        let mut select_actions = Vec::new();
        let mut setup_actions = Vec::new();
        let mut result_actions = Vec::new();
        let scroll = std::mem::take(&mut self.scroll_to_cursor);
        let output = self.egui_ctx.run_ui(input, |ui| match &self.screen {
            Screen::Select => {
                select_actions = ui::draw_select(
                    ui,
                    &self.songs,
                    &self.config,
                    &ports,
                    self.message.as_deref(),
                    scroll,
                );
            }
            Screen::Setup(s) => setup_actions = ui::draw_setup(ui, s),
            Screen::Play(play) => ui::draw_play(ui, play, &self.config, &ports),
            Screen::Result(o) => result_actions = ui::draw_result(ui, o, &self.config),
        });
        state.handle_platform_output(window, output.platform_output.clone());
        if let Some(gpu) = &mut self.gpu {
            gpu.render(&self.egui_ctx, output);
        }
        for a in select_actions {
            self.select_action(a);
        }
        for a in setup_actions {
            self.setup_action(a);
        }
        for a in result_actions {
            self.result_action(a);
        }
    }
}

/// キーボードの割り当て（DTX Web Editor と同じ並び。Q は HH オープン）
fn key_lane(code: KeyCode) -> Option<DrumLane> {
    Some(match code {
        KeyCode::KeyA => DrumLane::LC,
        KeyCode::KeyS => DrumLane::HH,
        KeyCode::KeyQ => DrumLane::HHO,
        KeyCode::KeyZ => DrumLane::LP,
        KeyCode::KeyD => DrumLane::SD,
        KeyCode::KeyG => DrumLane::HT,
        KeyCode::KeyF => DrumLane::BD,
        KeyCode::KeyH => DrumLane::LT,
        KeyCode::KeyJ => DrumLane::FT,
        KeyCode::KeyK => DrumLane::CY,
        KeyCode::KeyL => DrumLane::RD,
        _ => return None,
    })
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("DTXmaniaNXT")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
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
        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
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
                    && !event.repeat
                    && let PhysicalKey::Code(code) = event.physical_key
                {
                    self.on_key(event_loop, code);
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
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // 叩いた音をすぐ鳴らすため、描画を待たずに MIDI を処理する
        self.pump();
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let _ = host_us();
    let arg = std::env::args_os().nth(1).map(PathBuf::from);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(arg);
    event_loop.run_app(&mut app)?;
    Ok(())
}
