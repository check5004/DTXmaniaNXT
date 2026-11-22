//! 画面を PNG で残す（上流の Capture キーと AutoSaveResult）。
//!
//! 窓に描いた画像はそのまま読み出せないことがあるので、同じ絵を読み出せる画像にもう一度描き、
//! GPU から受け取って別のスレッドで PNG にする。

use std::path::{Path, PathBuf};

/// 1 枚分の描く先と、GPU から受け取る入れ物
pub struct Shot {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    /// 1 行のバイト数（GPU の決まりで 256 の倍数にそろえる）
    row_bytes: u32,
    format: wgpu::TextureFormat,
}

impl Shot {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let row_bytes = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture"),
            size: u64::from(row_bytes) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            texture,
            view,
            buffer,
            width,
            height,
            row_bytes,
            format,
        }
    }

    /// 描く先
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// 描いた絵を受け取る入れ物に写す（描いたあとに同じ命令の列で）
    pub fn copy(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.row_bytes),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// GPU から受け取り（命令を送ったあと）、PNG にする仕事は別のスレッドに渡す
    pub fn save(self, device: &wgpu::Device, path: PathBuf) {
        let Some(order) = channel_order(self.format) else {
            log::warn!("capture: {:?} is not supported", self.format);
            return;
        };
        let slice = self.buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        if let Err(e) = device.poll(wgpu::PollType::wait_indefinitely()) {
            log::warn!("capture: {e}");
            return;
        }
        let Ok(data) = slice.get_mapped_range() else {
            log::warn!("capture: the image could not be read");
            return;
        };
        let pixels = pack_rows(&data, self.width, self.height, self.row_bytes, order);
        drop(data);
        self.buffer.unmap();
        let (w, h) = (self.width, self.height);
        std::thread::spawn(move || match write_png(&path, &pixels, w, h) {
            Ok(()) => log::info!("saved {}", path.display()),
            Err(e) => log::warn!("{}: {e:#}", path.display()),
        });
    }
}

/// 画素の並び
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    Rgba,
    Bgra,
}

fn channel_order(format: wgpu::TextureFormat) -> Option<Order> {
    use wgpu::TextureFormat as F;
    match format {
        F::Rgba8Unorm | F::Rgba8UnormSrgb => Some(Order::Rgba),
        F::Bgra8Unorm | F::Bgra8UnormSrgb => Some(Order::Bgra),
        _ => None,
    }
}

/// 行ごとの詰め物を除いて RGBA の並びにする。透けないように α は 255
fn pack_rows(data: &[u8], width: u32, height: u32, row_bytes: u32, order: Order) -> Vec<u8> {
    let (w, row) = (width as usize * 4, row_bytes as usize);
    let mut out = Vec::with_capacity(w * height as usize);
    for y in 0..height as usize {
        let (pixels, _) = data[y * row..y * row + w].as_chunks::<4>();
        for px in pixels {
            let (r, b) = match order {
                Order::Rgba => (px[0], px[2]),
                Order::Bgra => (px[2], px[0]),
            };
            out.extend_from_slice(&[r, px[1], b, 255]);
        }
    }
    out
}

fn write_png(path: &Path, pixels: &[u8], width: u32, height: u32) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    image::save_buffer(path, pixels, width, height, image::ExtendedColorType::Rgba8)?;
    Ok(())
}

/// ファイルの名前に使う今の日時（上流と同じ yyyyMMddHHmmss、この PC の時刻）
fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d%H%M%S").to_string()
}

/// F12 で残す先：「ピクチャ」の DTXmaniaNXT（無ければ設定のフォルダの Capture_img）
pub fn screenshot_path() -> Option<PathBuf> {
    let dir = dirs::picture_dir()
        .map(|d| d.join("DTXmaniaNXT"))
        .or_else(|| crate::config::Config::dir().map(|d| d.join("Capture_img")))?;
    Some(dir.join(format!("{}.png", stamp())))
}

/// 記録を更新した結果を残す先（上流と同じく譜面の隣に「譜面の名前.日時_DRUMS_ランク.png」）
pub fn result_path(chart: &Path, rank: &str) -> PathBuf {
    result_path_at(chart, &stamp(), rank)
}

fn result_path_at(chart: &Path, stamp: &str, rank: &str) -> PathBuf {
    let mut name = chart.as_os_str().to_owned();
    name.push(format!(".{stamp}_DRUMS_{rank}.png"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_drop_padding_and_swap_blue_and_red() {
        // 幅 1、高さ 2、1 行 8 バイト（後ろ 4 バイトは詰め物）
        let data = [1, 2, 3, 0, 9, 9, 9, 9, 4, 5, 6, 7, 9, 9, 9, 9];
        assert_eq!(
            pack_rows(&data, 1, 2, 8, Order::Bgra),
            vec![3, 2, 1, 255, 6, 5, 4, 255]
        );
        assert_eq!(
            pack_rows(&data, 1, 2, 8, Order::Rgba),
            vec![1, 2, 3, 255, 4, 5, 6, 255]
        );
    }

    #[test]
    fn result_images_sit_next_to_the_chart() {
        let p = result_path_at(Path::new("songs/a/b.dtx"), "20261122153000", "SS");
        assert_eq!(p, Path::new("songs/a/b.dtx.20261122153000_DRUMS_SS.png"));
    }
}
