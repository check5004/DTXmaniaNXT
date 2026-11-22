//! Windows の Media Foundation で動画を読む。
//!
//! Media Foundation の DLL（mfplat.dll と mfreadwrite.dll）は、動画を読むときに読み込む。
//! ふつうに結ぶと、Media Foundation の無い Windows（N エディションなど）では exe そのものが起動しないため。
//! 呼ぶのは DLL から引いた 5 つの関数と、COM のインターフェイスのメソッドだけ。

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer, IMFAttributes, IMFMediaType, IMFSample, IMFSourceReader, MF_MT_DEFAULT_STRIDE,
    MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE,
    MF_MT_SUBTYPE, MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF_VERSION, MFMediaType_Video,
    MFSTARTUP_NOSOCKET, MFVideoFormat_RGB32,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::System::Variant::VT_I8;
use windows::core::{GUID, HRESULT, HSTRING, Interface, PCSTR, PCWSTR, w};

use crate::{Area, Frame, Source, Timing, bgrx_to_frame, fit_size};

type RawFn = unsafe extern "system" fn() -> isize;
type StartupFn = unsafe extern "system" fn(u32, u32) -> HRESULT;
type ShutdownFn = unsafe extern "system" fn() -> HRESULT;
type CreateAttributesFn = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type CreateMediaTypeFn = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CreateReaderFn = unsafe extern "system" fn(PCWSTR, *mut c_void, *mut *mut c_void) -> HRESULT;

/// DLL から引いた Media Foundation の関数
struct Api {
    startup: StartupFn,
    shutdown: ShutdownFn,
    create_attributes: CreateAttributesFn,
    create_media_type: CreateMediaTypeFn,
    create_reader: CreateReaderFn,
}

fn api() -> Result<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| match load_api() {
        Ok(api) => Some(api),
        Err(e) => {
            log::warn!("Media Foundation を使えません: {e:#}");
            None
        }
    })
    .as_ref()
    .ok_or_else(|| {
        anyhow!("Media Foundation を使えません（Windows の N エディションなら Media Feature Pack が要ります）")
    })
}

fn load_api() -> Result<Api> {
    let plat = system_dll(w!("mfplat.dll")).context("mfplat.dll")?;
    let rw = system_dll(w!("mfreadwrite.dll")).context("mfreadwrite.dll")?;
    // SAFETY: どれも Windows SDK の宣言（mfapi.h・mfreadwrite.h）どおりの型にする。DLL は放さないので、関数はずっと使える
    unsafe {
        Ok(Api {
            startup: std::mem::transmute::<RawFn, StartupFn>(proc(plat, b"MFStartup\0")?),
            shutdown: std::mem::transmute::<RawFn, ShutdownFn>(proc(plat, b"MFShutdown\0")?),
            create_attributes: std::mem::transmute::<RawFn, CreateAttributesFn>(proc(
                plat,
                b"MFCreateAttributes\0",
            )?),
            create_media_type: std::mem::transmute::<RawFn, CreateMediaTypeFn>(proc(
                plat,
                b"MFCreateMediaType\0",
            )?),
            create_reader: std::mem::transmute::<RawFn, CreateReaderFn>(proc(
                rw,
                b"MFCreateSourceReaderFromURL\0",
            )?),
        })
    }
}

/// System32 の DLL を読み込む（ほかの場所の同じ名前の DLL は読まない）
fn system_dll(name: PCWSTR) -> Result<HMODULE> {
    // SAFETY: 名前は終わりに 0 のある文字列（w! で作ったもの）
    Ok(unsafe { LoadLibraryExW(name, None, LOAD_LIBRARY_SEARCH_SYSTEM32) }?)
}

/// DLL の関数を名前（終わりに 0 のあるバイト列）で引く
fn proc(dll: HMODULE, name: &'static [u8]) -> Result<RawFn> {
    debug_assert_eq!(name.last(), Some(&0));
    // SAFETY: dll は読み込んだままの DLL、名前は終わりに 0 のある文字列
    unsafe { GetProcAddress(dll, PCSTR(name.as_ptr())) }.ok_or_else(|| {
        anyhow!(
            "{} がありません",
            String::from_utf8_lossy(&name[..name.len() - 1])
        )
    })
}

/// 出力の引数でインターフェイスを返す関数を呼ぶ
fn out<I: Interface>(call: impl FnOnce(*mut *mut c_void) -> HRESULT) -> Result<I> {
    let mut p = std::ptr::null_mut();
    call(&mut p).ok()?;
    if p.is_null() {
        bail!("空のインターフェイスが返った");
    }
    // SAFETY: 成功したら、参照を 1 つ持ったインターフェイスが入っている（COM の決まり）
    Ok(unsafe { I::from_raw(p) })
}

/// このスレッドで COM と Media Foundation を使うあいだ持つ
struct Session {
    api: &'static Api,
    com: bool,
}

impl Session {
    fn start() -> Result<Self> {
        let api = api()?;
        // SAFETY: このスレッドで COM を使い始める。うまくいったら drop で終える
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        // SAFETY: 引いた MFStartup を宣言どおりに呼ぶ
        if let Err(e) = unsafe { (api.startup)(MF_VERSION, MFSTARTUP_NOSOCKET) }.ok() {
            if com {
                // SAFETY: 上で始めた COM を終える
                unsafe { CoUninitialize() };
            }
            return Err(e).context("MFStartup");
        }
        Ok(Self { api, com })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: start で始めた Media Foundation と COM を、同じスレッドで終える
        unsafe {
            let _ = (self.api.shutdown)();
            if self.com {
                CoUninitialize();
            }
        }
    }
}

/// 読む映像（最初の映像のストリーム）
const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

/// 読み出す絵の形
#[derive(Debug, Clone, Copy)]
struct Format {
    /// 並びの幅と行の数
    width: u32,
    height: u32,
    /// 1 行のバイト数（負なら下の行から）。2D のバッファでないときに使う
    stride: i64,
    /// 見せる範囲（H.264 の 1080 行は 1088 行で並んでいることがある）
    area: Area,
    /// 1 枚の長さ（絵に長さが付いていないときに使う）
    frame_us: i64,
}

impl Format {
    fn read(t: &IMFMediaType) -> Result<Self> {
        // SAFETY: 受け取った型から値を読むだけ
        unsafe {
            let (width, height) = split(t.GetUINT64(&MF_MT_FRAME_SIZE)?);
            let stride = t
                .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                .map_or(i64::from(width) * 4, |s| i64::from(s as i32));
            let mut blob = [0u8; 16];
            let area = match t.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut blob, None) {
                Ok(()) => Area::from_video_area(&blob, width, height),
                Err(_) => Area::full(width, height),
            };
            let frame_us = t
                .GetUINT64(&MF_MT_FRAME_RATE)
                .ok()
                .map(split)
                .filter(|&(n, d)| n > 0 && d > 0)
                .map_or(33_333, |(n, d)| i64::from(d) * 1_000_000 / i64::from(n));
            Ok(Self {
                width,
                height,
                stride,
                area,
                frame_us,
            })
        }
    }
}

/// 上 32 ビットと下 32 ビット（MF_MT_FRAME_SIZE なら幅と高さ）
fn split(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

fn join(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

/// Media Foundation の Source Reader で読む
pub(crate) struct MfSource {
    reader: IMFSourceReader,
    sample: Option<IMFSample>,
    format: Format,
    /// 読み手を放してから終える（フィールドは上から順に放される）
    _session: Session,
}

impl MfSource {
    pub(crate) fn open(path: &Path, max: (u32, u32)) -> Result<Self> {
        let session = Session::start()?;
        let api = session.api;
        // SAFETY: 引いた関数を宣言どおりに呼び、受け取ったインターフェイスのメソッドを呼ぶ
        unsafe {
            let attrs: IMFAttributes =
                out(|p| (api.create_attributes)(p, 1)).context("MFCreateAttributes")?;
            // 色の変換（YUV → RGB）と縮小を Source Reader にさせる
            attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
            let url = HSTRING::from(path);
            let reader: IMFSourceReader =
                out(|p| (api.create_reader)(PCWSTR(url.as_ptr()), attrs.as_raw(), p))
                    .context("動画を開けません")?;
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            reader
                .SetStreamSelection(STREAM, true)
                .context("映像がありません")?;
            let native = reader.GetNativeMediaType(STREAM, 0)?;
            let (w, h) = split(native.GetUINT64(&MF_MT_FRAME_SIZE)?);
            let small = fit_size(w, h, max);
            let set = |size: Option<(u32, u32)>| -> Result<()> {
                let t: IMFMediaType = out(|p| (api.create_media_type)(p))?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                if let Some((w, h)) = size {
                    t.SetUINT64(&MF_MT_FRAME_SIZE, join(w, h))?;
                }
                reader.SetCurrentMediaType(STREAM, None, &t)?;
                Ok(())
            };
            // 縮められなければ、元の大きさのまま RGB にする
            if small == (w, h) || set(Some(small)).is_err() {
                set(None).context("RGB の絵にできません")?;
            }
            let format = Format::read(&reader.GetCurrentMediaType(STREAM)?)?;
            log::info!(
                "{}: {w}x{h} → {}x{}",
                path.display(),
                format.area.w,
                format.area.h
            );
            Ok(Self {
                reader,
                sample: None,
                format,
                _session: session,
            })
        }
    }
}

impl Source for MfSource {
    fn next(&mut self) -> Result<Option<Timing>> {
        loop {
            let (mut flags, mut ts, mut sample) = (0u32, 0i64, None::<IMFSample>);
            // SAFETY: 出力の引数はどれもこの関数の中の変数
            unsafe {
                self.reader.ReadSample(
                    STREAM,
                    0,
                    None,
                    Some(&raw mut flags),
                    Some(&raw mut ts),
                    Some(&raw mut sample),
                )
            }?;
            if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                bail!("映像を読めなくなりました");
            }
            if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                // SAFETY: 今の型を受け取るだけ
                self.format = Format::read(&unsafe { self.reader.GetCurrentMediaType(STREAM) }?)?;
            }
            if let Some(s) = sample {
                // SAFETY: 受け取った絵の長さを読むだけ
                let dur = unsafe { s.GetSampleDuration() }.unwrap_or(0) / 10;
                self.sample = Some(s);
                return Ok(Some(Timing {
                    start_us: ts / 10,
                    dur_us: if dur > 0 { dur } else { self.format.frame_us },
                }));
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(None);
            }
        }
    }

    fn pixels(&mut self) -> Result<Frame> {
        let sample = self.sample.as_ref().context("まだ絵を読んでいません")?;
        let f = self.format;
        let row = i64::from(f.width) * 4;
        // SAFETY: ロックしているあいだだけ、バッファの中身をスライスとして読む。
        // 長さは、行の数と 1 行のバイト数から、並びが占める範囲だけにする
        let frame = unsafe {
            let buf = sample.ConvertToContiguousBuffer()?;
            if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
                let (mut scan0, mut pitch) = (std::ptr::null_mut::<u8>(), 0i32);
                b2.Lock2D(&mut scan0, &mut pitch)?;
                let pitch = i64::from(pitch);
                let span = (i64::from(f.height) - 1) * pitch.abs() + row;
                let frame = if scan0.is_null() || pitch.abs() < row || span <= 0 {
                    None
                } else {
                    // 下の行から並ぶなら、いちばん低い番地は最後の行
                    let low = if pitch >= 0 {
                        scan0
                    } else {
                        scan0.wrapping_sub(((i64::from(f.height) - 1) * -pitch) as usize)
                    };
                    let data = std::slice::from_raw_parts(low, span as usize);
                    bgrx_to_frame(data, f.height, pitch, f.area)
                };
                b2.Unlock2D()?;
                frame
            } else {
                let (mut p, mut len) = (std::ptr::null_mut::<u8>(), 0u32);
                buf.Lock(&mut p, None, Some(&raw mut len))?;
                let frame = if p.is_null() || f.stride.abs() < row {
                    None
                } else {
                    let data = std::slice::from_raw_parts(p, len as usize);
                    bgrx_to_frame(data, f.height, f.stride, f.area)
                };
                buf.Unlock()?;
                frame
            }
        };
        frame.context("絵の並びが思っていた形と違います")
    }

    fn seek(&mut self, us: i64) -> Result<()> {
        let mut at = PROPVARIANT::default();
        // SAFETY: 時刻（100 ナノ秒単位の i64）の PROPVARIANT を作って渡す
        unsafe {
            let v = &mut *at.Anonymous.Anonymous;
            v.vt = VT_I8;
            v.Anonymous.hVal = us.max(0) * 10;
            self.reader.SetCurrentPosition(&GUID::zeroed(), &at)?;
        }
        self.sample = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// testdata/video の小さな動画（前半が赤、後半が青）を読む。
    /// Media Foundation の無い Windows（Server の既定など）では読めないので、そう書いて飛ばす
    fn read_movie(name: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/video")
            .join(name);
        if let Err(e) = api() {
            eprintln!("skipped: {e:#}");
            return;
        }
        let mut src = MfSource::open(&path, (1280, 720)).unwrap();
        let first = src.next().unwrap().unwrap();
        assert_eq!(first.start_us, 0);
        let f = src.pixels().unwrap();
        assert_eq!((f.width, f.height), (160, 90));
        let reddish = |f: &Frame| f.rgba[0] > 150 && f.rgba[2] < 100;
        let bluish = |f: &Frame| f.rgba[2] > 150 && f.rgba[0] < 100;
        assert!(reddish(&f), "{:?}", &f.rgba[..4]);
        // 後半（1 秒より後）へ
        src.seek(1_500_000).unwrap();
        let mut last = None;
        while let Some(t) = src.next().unwrap() {
            if t.start_us >= 1_500_000 {
                last = Some(t);
                break;
            }
        }
        assert!(last.is_some());
        let f = src.pixels().unwrap();
        assert!(bluish(&f), "{:?}", &f.rgba[..4]);
        // 終わりまで読める
        while src.next().unwrap().is_some() {}
    }

    #[test]
    fn reads_h264_mp4() {
        read_movie("red-blue-h264.mp4");
    }

    #[test]
    fn reads_xvid_avi() {
        read_movie("red-blue-xvid.avi");
    }
}
