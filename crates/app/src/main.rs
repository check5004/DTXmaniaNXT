//! DTXmaniaNXT 本体。フェーズ 1 は「1 曲を最後まで遊べる」最小版。
//!
//! - 引数かドロップで DTX を読み、BGM を鳴らし、落ちてくる音符を叩いて判定する
//! - 判定は MIDI の時刻と音の再生位置で行い、画面のフレームには左右されない
//! - 電子ドラムが無いときはキーボードで叩ける

mod audio;
mod fonts;
mod game;
mod midi;
mod time;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use dtx::DrumLane;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::game::Play;
use crate::midi::MidiIn;
use crate::time::host_us;
use crate::ui::View;

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

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    midi: Option<MidiIn>,
    play: Option<Play>,
    view: View,
    pending: Option<PathBuf>,
    message: Option<String>,
}

impl App {
    fn new(pending: Option<PathBuf>) -> Self {
        let midi = match MidiIn::open_all() {
            Ok(m) => Some(m),
            Err(e) => {
                log::warn!("MIDI is off: {e:#}");
                None
            }
        };
        Self {
            window: None,
            gpu: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            midi,
            play: None,
            view: View::default(),
            pending,
            message: None,
        }
    }

    fn start(&mut self, path: PathBuf) {
        self.play = None;
        match Play::load(&path) {
            Ok(p) => {
                self.message = None;
                self.play = Some(p);
            }
            Err(e) => {
                log::error!("{e:#}");
                self.message = Some(format!("読み込めませんでした: {e:#}"));
            }
        }
    }

    /// 届いた MIDI をすべて処理し、時間を進める
    fn pump(&mut self) {
        if let (Some(midi), Some(play)) = (&self.midi, &mut self.play) {
            for raw in midi.drain() {
                play.on_midi(raw);
            }
        } else if let Some(midi) = &self.midi {
            midi.drain().for_each(drop);
        }
        if let Some(play) = &mut self.play {
            play.update();
        }
    }

    fn on_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode) {
        let at = host_us();
        if let Some(lane) = key_lane(code)
            && let Some(play) = &mut self.play
        {
            play.on_key(lane, at);
            return;
        }
        match code {
            KeyCode::Escape => {
                if self.play.take().is_none() {
                    event_loop.exit();
                }
            }
            KeyCode::KeyR => {
                if let Some(path) = self.play.as_ref().map(|p| p.path.clone()) {
                    self.start(path);
                }
            }
            KeyCode::KeyM => {
                if let Some(p) = &mut self.play {
                    p.drum_sounds = !p.drum_sounds;
                }
            }
            KeyCode::Minus => self.adjust_input(-1.0),
            KeyCode::Equal => self.adjust_input(1.0),
            KeyCode::BracketLeft => self.view.visual_offset_ms -= 1.0,
            KeyCode::BracketRight => self.view.visual_offset_ms += 1.0,
            KeyCode::ArrowUp => {
                self.view.scroll_px_per_ms = (self.view.scroll_px_per_ms + 0.05).min(3.0)
            }
            KeyCode::ArrowDown => {
                self.view.scroll_px_per_ms = (self.view.scroll_px_per_ms - 0.05).max(0.1)
            }
            _ => {}
        }
    }

    fn adjust_input(&mut self, d: f64) {
        if let Some(p) = &mut self.play {
            p.input_offset_ms += d;
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
        let output = self.egui_ctx.run_ui(input, |ui| match &self.play {
            Some(play) => ui::draw_play(ui, play, &self.view, &ports),
            None => ui::draw_idle(ui, &ports, self.message.as_deref()),
        });
        state.handle_platform_output(window, output.platform_output.clone());
        if let Some(gpu) = &mut self.gpu {
            gpu.render(&self.egui_ctx, output);
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
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::DroppedFile(path) => self.start(path),
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
    let pending = std::env::args().nth(1).map(PathBuf::from);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(pending);
    event_loop.run_app(&mut app)?;
    Ok(())
}
