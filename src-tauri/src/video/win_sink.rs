// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Windows native video sink (P2.1, Dev-Docs active/MOBILE_RTSP.md): H264/HEVC access
//! units → Media Foundation decoder → D3D11 swapchain on a child window UNDER the
//! transparent WebView (the hole-punch surface the spike proved).
//!
//! Pipeline, all on one dedicated sink thread (which also owns the child HWND and pumps
//! its messages):
//!   Annex-B AU → IMFSample → decoder MFT (found via MFTEnumEx by input subtype; the
//!   Microsoft H264 decoder is always present, HEVC needs the "HEVC Video Extensions") →
//!   NV12 → ID3D11VideoProcessor blit (colour convert + scale + letterbox) → flip-model
//!   swapchain, presented immediately (sync interval 0 — the frame goes out, latency
//!   first, matching the client's no-buffering design).
//!
//! Latency knobs: MF_LOW_LATENCY on the decoder (output per input, no internal queueing),
//! DXGI device manager handed to the MFT so decode stays on the GPU (DXVA); a CPU-output
//! fallback (IMF2DBuffer copy into an NV12 staging texture) covers a non-D3D-aware MFT.
//!
//! The GDI-alpha trap from the spike applies here unchanged: everything visible must go
//! through the swapchain — never GDI — because the transparent Tauri window makes the DWM
//! honour surface alpha.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::Interface;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Resource,
    ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice, ID3D11VideoProcessor,
    ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView,
    ID3D11VideoProcessorOutputView, D3D11_BIND_DECODER, D3D11_CPU_ACCESS_WRITE,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DYNAMIC,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory2, IDXGISwapChain1, DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer, IMFActivate, IMFDXGIBuffer, IMFDXGIDeviceManager, IMFMediaType, IMFSample,
    IMFTransform, MFCreateDXGIDeviceManager, MFCreateMediaType, MFCreateMemoryBuffer,
    MFCreateSample, MFStartup, MFTEnumEx, MFMediaType_Video, MFT_CATEGORY_VIDEO_DECODER,
    MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
    MFT_FRIENDLY_NAME_Attribute, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
    MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER,
    MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MFSTARTUP_FULL,
    MF_API_VERSION, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE,
    MF_E_BUFFERTOOSMALL, MF_E_TRANSFORM_TYPE_NOT_SET,
    MF_LOW_LATENCY, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_TRANSFORM_ASYNC,
    MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_SDK_VERSION,
    MFVideoFormat_H264, MFVideoFormat_HEVC, MFVideoFormat_HEVC_ES, MFVideoFormat_NV12,
};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW,
    RegisterClassW, SetWindowPos, ShowWindow, TranslateMessage, HWND_BOTTOM, MSG, PM_REMOVE,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA, WINDOW_EX_STYLE, WNDCLASSW, WS_CHILD,
    WS_VISIBLE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkCodec {
    H264,
    H265,
}

enum Cmd {
    /// One Annex-B access unit + its (unwrapped) RTP timestamp in 90 kHz ticks.
    Frame(Vec<u8>, u64),
    Rect(i32, i32, i32, i32),
    Visible(bool),
    Stop,
}

#[derive(Default)]
struct Shared {
    frames_presented: AtomicU64,
    /// Decoded picture size, once known.
    size: Mutex<Option<(u32, u32)>>,
    error: Mutex<Option<String>>,
    stopped: AtomicBool,
}

/// Handle to the sink thread. Dropping stops it.
pub struct WinVideoSink {
    tx: Sender<Cmd>,
    join: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl WinVideoSink {
    /// Spawn the sink: child window at `rect` (PHYSICAL px, parent client coords) inside
    /// `parent_hwnd`, decoder for `codec`. Returns once device + decoder initialised —
    /// "HEVC Video Extensions missing" style failures surface here, before any stream runs.
    pub fn start(
        parent_hwnd: isize,
        rect: (i32, i32, i32, i32),
        codec: SinkCodec,
    ) -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let shared = Arc::new(Shared::default());
        let shared2 = shared.clone();
        let join = std::thread::Builder::new()
            .name("win-video-sink".into())
            .spawn(move || run_sink(parent_hwnd, rect, codec, rx, ready_tx, shared2))
            .map_err(|e| format!("sink thread spawn: {e}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(())) => Ok(Self { tx, join: Some(join), shared }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => Err("video sink did not initialise within 8 s".into()),
        }
    }

    pub fn push(&self, au: Vec<u8>, rtp_ts_90k: u64) {
        let _ = self.tx.send(Cmd::Frame(au, rtp_ts_90k));
    }

    pub fn set_rect(&self, x: i32, y: i32, w: i32, h: i32) {
        let _ = self.tx.send(Cmd::Rect(x, y, w, h));
    }

    pub fn set_visible(&self, visible: bool) {
        let _ = self.tx.send(Cmd::Visible(visible));
    }

    pub fn frames_presented(&self) -> u64 {
        self.shared.frames_presented.load(Ordering::Relaxed)
    }

    pub fn picture_size(&self) -> Option<(u32, u32)> {
        *self.shared.size.lock().unwrap()
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    pub fn stop(&mut self) {
        if !self.shared.stopped.swap(true, Ordering::SeqCst) {
            let _ = self.tx.send(Cmd::Stop);
            if let Some(j) = self.join.take() {
                let _ = j.join();
            }
        }
    }
}

impl Drop for WinVideoSink {
    fn drop(&mut self) {
        self.stop();
    }
}

// ─── Sink thread ─────────────────────────────────────────────────────────────

static MF_ONCE: Once = Once::new();
static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn sink_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn run_sink(
    parent_hwnd: isize,
    rect: (i32, i32, i32, i32),
    codec: SinkCodec,
    rx: Receiver<Cmd>,
    ready_tx: Sender<Result<(), String>>,
    shared: Arc<Shared>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    MF_ONCE.call_once(|| unsafe {
        let version = ((MF_SDK_VERSION as u32) << 16) | MF_API_VERSION as u32;
        if let Err(e) = MFStartup(version, MFSTARTUP_FULL) {
            log::warn!("[video] MFStartup failed: {e}");
        }
    });

    let mut state = match unsafe { SinkState::init(parent_hwnd, rect, codec) } {
        Ok(s) => {
            let _ = ready_tx.send(Ok(()));
            s
        }
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    loop {
        // Pump the child window's messages (it lives on this thread).
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        match rx.recv_timeout(Duration::from_millis(5)) {
            Ok(Cmd::Frame(au, ts)) => {
                if shared.error.lock().unwrap().is_some() {
                    continue; // fatal — drain the queue without touching the decoder
                }
                if let Err(e) = unsafe { state.feed(&au, ts, &shared) } {
                    log::warn!("[video] sink decode error: {e}");
                    *shared.error.lock().unwrap() = Some(e);
                }
            }
            Ok(Cmd::Rect(x, y, w, h)) => unsafe { state.set_rect(x, y, w, h) },
            Ok(Cmd::Visible(v)) => unsafe {
                let _ = ShowWindow(state.hwnd, if v { SW_SHOWNA } else { SW_HIDE });
            },
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }

    unsafe {
        state.teardown();
    }
}

// ─── Device / decoder / render state ─────────────────────────────────────────

struct SinkState {
    hwnd: HWND,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    swapchain: IDXGISwapChain1,
    _dxgi_manager: IMFDXGIDeviceManager,
    decoder: IMFTransform,
    decoder_provides_samples: bool,
    /// Non-D3D-aware MFT fallback: CPU NV12 samples uploaded into this dynamic texture.
    cpu_upload: Option<ID3D11Texture2D>,
    /// Video processor, cached per (in_w, in_h, out_w, out_h).
    vp: Option<(ID3D11VideoProcessorEnumerator, ID3D11VideoProcessor, (u32, u32, u32, u32))>,
    picture: (u32, u32),
    client: (i32, i32),
    sample_index: u64,
}

impl SinkState {
    unsafe fn init(
        parent_hwnd: isize,
        rect: (i32, i32, i32, i32),
        codec: SinkCodec,
    ) -> Result<Self, String> {
        unsafe {
            let hwnd = create_child(parent_hwnd, rect)?;

            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
            let device = device.ok_or("no D3D11 device")?;
            let context = context.ok_or("no D3D11 context")?;

            // The MFT decodes on its own threads against this device.
            let _ = device
                .cast::<ID3D11Multithread>()
                .map_err(|e| format!("ID3D11Multithread: {e}"))?
                .SetMultithreadProtected(true);

            let video_device = device
                .cast::<ID3D11VideoDevice>()
                .map_err(|e| format!("ID3D11VideoDevice: {e}"))?;
            let video_context = context
                .cast::<ID3D11VideoContext>()
                .map_err(|e| format!("ID3D11VideoContext: {e}"))?;

            let factory: IDXGIFactory2 =
                CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: 0,
                Height: 0,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let swapchain = factory
                .CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)
                .map_err(|e| format!("CreateSwapChainForHwnd: {e}"))?;

            let mut reset_token = 0u32;
            let mut manager: Option<IMFDXGIDeviceManager> = None;
            MFCreateDXGIDeviceManager(&mut reset_token, &mut manager)
                .map_err(|e| format!("MFCreateDXGIDeviceManager: {e}"))?;
            let manager = manager.ok_or("no DXGI device manager")?;
            manager
                .ResetDevice(&device, reset_token)
                .map_err(|e| format!("ResetDevice: {e}"))?;

            let (decoder, decoder_provides_samples) = create_decoder(codec, &manager)?;

            Ok(Self {
                hwnd,
                device,
                context,
                video_device,
                video_context,
                swapchain,
                _dxgi_manager: manager,
                decoder,
                decoder_provides_samples,
                cpu_upload: None,
                vp: None,
                picture: (0, 0),
                client: (rect.2.max(1), rect.3.max(1)),
                sample_index: 0,
            })
        }
    }

    unsafe fn set_rect(&mut self, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                x,
                y,
                w.max(1),
                h.max(1),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            if (w.max(1), h.max(1)) != (self.client.0, self.client.1) {
                self.client = (w.max(1), h.max(1));
                self.vp = None; // output size changed → rebuild the processor
                let _ = self.swapchain.ResizeBuffers(
                    0,
                    self.client.0 as u32,
                    self.client.1 as u32,
                    DXGI_FORMAT_UNKNOWN,
                    DXGI_SWAP_CHAIN_FLAG(0),
                );
            }
        }
    }

    /// Feed one access unit and render every frame it yields.
    unsafe fn feed(&mut self, au: &[u8], ts_90k: u64, shared: &Shared) -> Result<(), String> {
        unsafe {
            let sample = MFCreateSample().map_err(|e| format!("MFCreateSample: {e}"))?;
            let buffer =
                MFCreateMemoryBuffer(au.len() as u32).map_err(|e| format!("buffer: {e}"))?;
            let mut ptr = std::ptr::null_mut();
            buffer
                .Lock(&mut ptr, None, None)
                .map_err(|e| format!("Lock: {e}"))?;
            std::ptr::copy_nonoverlapping(au.as_ptr(), ptr, au.len());
            let _ = buffer.Unlock();
            let _ = buffer.SetCurrentLength(au.len() as u32);
            sample.AddBuffer(&buffer).map_err(|e| format!("AddBuffer: {e}"))?;
            // 90 kHz → 100 ns units. Monotonic (the depacketizer already unwraps order).
            let _ = sample.SetSampleTime((ts_90k as i64) * 10_000_000 / 90_000);
            let _ = sample.SetSampleDuration(10_000_000 / 30);
            self.sample_index += 1;
            if self.sample_index == 1 {
                let _ = self
                    .decoder
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0);
                let _ = self
                    .decoder
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);
            }

            self.decoder
                .ProcessInput(0, &sample, 0)
                .map_err(|e| format!("ProcessInput: {e}"))?;
            self.drain_outputs(shared)
        }
    }

    unsafe fn drain_outputs(&mut self, shared: &Shared) -> Result<(), String> {
        unsafe {
            // A type renegotiation that doesn't stick must not spin forever.
            let mut renegotiations = 0u32;
            loop {
                let mut out = [MFT_OUTPUT_DATA_BUFFER::default()];
                if !self.decoder_provides_samples {
                    // CPU path: the caller allocates the output sample.
                    let info = self
                        .decoder
                        .GetOutputStreamInfo(0)
                        .map_err(|e| format!("GetOutputStreamInfo: {e}"))?;
                    let sample = MFCreateSample().map_err(|e| e.to_string())?;
                    // cbSize reads 0 before the first decode — size the buffer ourselves
                    // (NV12 = w*h*3/2) from the current picture, else the placeholder type.
                    let (w, h) = if self.picture.0 > 0 { self.picture } else { (1280, 720) };
                    let nv12 = w * h * 3 / 2;
                    let buf = MFCreateMemoryBuffer(info.cbSize.max(nv12).max(1))
                        .map_err(|e| e.to_string())?;
                    sample.AddBuffer(&buf).map_err(|e| e.to_string())?;
                    out[0].pSample = std::mem::ManuallyDrop::new(Some(sample));
                }
                let mut status = 0u32;
                let result = self.decoder.ProcessOutput(0, &mut out, &mut status);
                let sample = std::mem::ManuallyDrop::take(&mut out[0].pSample);
                let _ = std::mem::ManuallyDrop::take(&mut out[0].pEvents);
                if std::env::var("KITE_SINK_DEBUG").is_ok() {
                    eprintln!(
                        "[sink] ProcessOutput -> {:?} (provides={}, picture={:?})",
                        result.as_ref().map(|_| "OK").map_err(|e| e.code()),
                        self.decoder_provides_samples,
                        self.picture,
                    );
                }
                match result {
                    Ok(()) => {
                        if let Some(sample) = sample {
                            self.render(&sample, shared)?;
                        }
                    }
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                    Err(e)
                        if e.code() == MF_E_TRANSFORM_STREAM_CHANGE
                            || e.code() == MF_E_TRANSFORM_TYPE_NOT_SET
                            // The HEVC MFT reports a placeholder-vs-stream size mismatch as
                            // "buffer too small" rather than a stream change — same cure.
                            || e.code() == MF_E_BUFFERTOOSMALL =>
                    {
                        renegotiations += 1;
                        if renegotiations > 8 {
                            return Err("output type renegotiation loop — decoder refuses every NV12 type".into());
                        }
                        self.negotiate_output()?;
                    }
                    Err(e) => return Err(format!("ProcessOutput: {e}")),
                }
            }
        }
    }

    /// (Re-)select NV12 on the output and pick up the coded picture size.
    unsafe fn negotiate_output(&mut self) -> Result<(), String> {
        unsafe {
            // Stream-change etiquette: clear the stale output type FIRST — several MFTs
            // (the HEVC one included) enumerate nothing while one is still set.
            let _ = self.decoder.SetOutputType(0, None, 0);
            let mut i = 0u32;
            loop {
                let mt: IMFMediaType = match self.decoder.GetOutputAvailableType(0, i) {
                    Ok(mt) => mt,
                    Err(e) => {
                        // The HEVC MFT enumerates NOTHING until an output type is set —
                        // break the circle by constructing the NV12 type ourselves.
                        log::debug!("[video] output enumeration empty ({e}) — constructing NV12 type manually");
                        return self.set_manual_output_type();
                    }
                };
                if mt.GetGUID(&MF_MT_SUBTYPE).map(|g| g == MFVideoFormat_NV12).unwrap_or(false) {
                    if mt.GetUINT64(&MF_MT_FRAME_SIZE).is_err() {
                        let _ = mt.SetUINT64(&MF_MT_FRAME_SIZE, (1280u64 << 32) | 720);
                    }
                    self.decoder
                        .SetOutputType(0, &mt, 0)
                        .map_err(|e| format!("SetOutputType: {e}"))?;
                    if std::env::var("KITE_SINK_DEBUG").is_ok() {
                        eprintln!("[sink] negotiated offered NV12 type (index {i})");
                    }
                    if let Ok(size) = mt.GetUINT64(&MF_MT_FRAME_SIZE) {
                        self.picture = ((size >> 32) as u32, (size & 0xFFFF_FFFF) as u32);
                        self.vp = None;
                        self.cpu_upload = None;
                    }
                    // The provides-samples verdict can change with the D3D manager applied.
                    if let Ok(info) = self.decoder.GetOutputStreamInfo(0) {
                        self.decoder_provides_samples =
                            info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
                    }
                    return Ok(());
                }
                i += 1;
            }
        }
    }

    /// Build the NV12 output type by hand (size from the last known picture, else the
    /// input placeholder) — the path for MFTs whose enumeration is empty pre-negotiation.
    unsafe fn set_manual_output_type(&mut self) -> Result<(), String> {
        unsafe {
            // After the first parsed input the MFT knows the real dimensions — it updates
            // its CURRENT INPUT type even while offering no output types (the HEVC MFT's
            // enumeration stays empty). That is the authoritative size source here.
            let from_input = self
                .decoder
                .GetInputCurrentType(0)
                .ok()
                .and_then(|mt| mt.GetUINT64(&MF_MT_FRAME_SIZE).ok())
                .map(|s| ((s >> 32) as u32, (s & 0xFFFF_FFFF) as u32))
                .filter(|&(w, h)| w > 0 && h > 0);
            // Bench probe: KITE_SINK_FORCE_SIZE=WxH pins the manual type to the real
            // stream dimensions (hypothesis test for the HEVC MFT's size handling).
            let forced = std::env::var("KITE_SINK_FORCE_SIZE").ok().and_then(|s| {
                let (w, h) = s.split_once('x')?;
                Some((w.parse().ok()?, h.parse().ok()?))
            });
            let (w, h) = forced
                .or(from_input)
                .or(if self.picture.0 > 0 { Some(self.picture) } else { None })
                .unwrap_or((1280, 720));
            if (w, h) != self.picture {
                self.picture = (w, h);
                self.vp = None;
                self.cpu_upload = None;
            }
            let mt = MFCreateMediaType().map_err(|e| e.to_string())?;
            mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| e.to_string())?;
            mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).map_err(|e| e.to_string())?;
            let _ = mt.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64);
            let _ = mt.SetUINT64(&MF_MT_FRAME_RATE, (30u64 << 32) | 1);
            let _ = mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1);
            let _ = mt.SetUINT32(&MF_MT_INTERLACE_MODE, 2);
            self.decoder
                .SetOutputType(0, &mt, 0)
                .map_err(|e| format!("manual SetOutputType(NV12 {w}x{h}): {e}"))?;
            if std::env::var("KITE_SINK_DEBUG").is_ok() {
                eprintln!("[sink] manual NV12 type set ({w}x{h})");
            }
            if let Ok(info) = self.decoder.GetOutputStreamInfo(0) {
                self.decoder_provides_samples =
                    info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
            }
            Ok(())
        }
    }

    unsafe fn render(&mut self, sample: &IMFSample, shared: &Shared) -> Result<(), String> {
        unsafe {
            let buffer = sample
                .GetBufferByIndex(0)
                .map_err(|e| format!("GetBufferByIndex: {e}"))?;

            // GPU path: the buffer wraps a D3D texture on OUR device.
            let (texture, subresource) = match buffer.cast::<IMFDXGIBuffer>() {
                Ok(dxgi) => {
                    let mut tex: Option<ID3D11Texture2D> = None;
                    dxgi.GetResource(&ID3D11Texture2D::IID, &mut tex as *mut _ as *mut _)
                        .map_err(|e| format!("IMFDXGIBuffer::GetResource: {e}"))?;
                    let tex = tex.ok_or("no texture from DXGI buffer")?;
                    let sub = dxgi.GetSubresourceIndex().unwrap_or(0);
                    (tex, sub)
                }
                Err(_) => {
                    // CPU path: copy the NV12 planes into the dynamic upload texture.
                    let tex = self.cpu_texture()?;
                    let two_d: IMF2DBuffer =
                        buffer.cast().map_err(|e| format!("IMF2DBuffer: {e}"))?;
                    let mut src = std::ptr::null_mut();
                    let mut pitch = 0i32;
                    two_d
                        .Lock2D(&mut src, &mut pitch)
                        .map_err(|e| format!("Lock2D: {e}"))?;
                    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                    self.context
                        .Map(&tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                        .map_err(|e| format!("Map: {e}"))?;
                    let (w, h) = self.picture;
                    let rows = h + h / 2; // Y plane + interleaved UV half-height
                    for row in 0..rows {
                        std::ptr::copy_nonoverlapping(
                            src.add((row as usize) * pitch as usize),
                            (mapped.pData as *mut u8).add((row as usize) * mapped.RowPitch as usize),
                            (w as usize).min(pitch as usize),
                        );
                    }
                    self.context.Unmap(&tex, 0);
                    let _ = two_d.Unlock2D();
                    (tex, 0)
                }
            };

            self.blit(&texture, subresource)?;
            let _ = self.swapchain.Present(0, DXGI_PRESENT(0));
            shared.frames_presented.fetch_add(1, Ordering::Relaxed);
            if shared.size.lock().unwrap().is_none() && self.picture.0 > 0 {
                *shared.size.lock().unwrap() = Some(self.picture);
            }
            Ok(())
        }
    }

    unsafe fn cpu_texture(&mut self) -> Result<ID3D11Texture2D, String> {
        unsafe {
            if let Some(t) = &self.cpu_upload {
                return Ok(t.clone());
            }
            let (w, h) = self.picture;
            if w == 0 || h == 0 {
                return Err("picture size unknown before first stream change".into());
            }
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_NV12,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_DECODER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut tex: Option<ID3D11Texture2D> = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .map_err(|e| format!("CreateTexture2D(NV12): {e}"))?;
            let tex = tex.ok_or("no NV12 upload texture")?;
            self.cpu_upload = Some(tex.clone());
            Ok(tex)
        }
    }

    /// NV12 → backbuffer via the video processor (convert + scale + letterbox).
    unsafe fn blit(&mut self, texture: &ID3D11Texture2D, subresource: u32) -> Result<(), String> {
        unsafe {
            let (in_w, in_h) = if self.picture.0 > 0 {
                self.picture
            } else {
                // Stream change hasn't fired yet — read the texture itself.
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut desc);
                (desc.Width, desc.Height)
            };
            let (out_w, out_h) = (self.client.0 as u32, self.client.1 as u32);
            let key = (in_w, in_h, out_w, out_h);

            if self.vp.as_ref().map(|(_, _, k)| *k != key).unwrap_or(true) {
                let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                    InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                    InputWidth: in_w,
                    InputHeight: in_h,
                    OutputWidth: out_w,
                    OutputHeight: out_h,
                    Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                    ..Default::default()
                };
                let enumerator = self
                    .video_device
                    .CreateVideoProcessorEnumerator(&content)
                    .map_err(|e| format!("CreateVideoProcessorEnumerator: {e}"))?;
                let processor = self
                    .video_device
                    .CreateVideoProcessor(&enumerator, 0)
                    .map_err(|e| format!("CreateVideoProcessor: {e}"))?;
                // Letterbox: aspect-preserving destination rect on a black background.
                let dest = letterbox(in_w, in_h, out_w, out_h);
                self.video_context
                    .VideoProcessorSetStreamFrameFormat(&processor, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
                self.video_context
                    .VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dest));
                self.vp = Some((enumerator, processor, key));
            }
            let (enumerator, processor, _) = self.vp.as_ref().unwrap();

            let backbuffer: ID3D11Texture2D = self
                .swapchain
                .GetBuffer(0)
                .map_err(|e| format!("GetBuffer: {e}"))?;

            let in_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: windows::Win32::Graphics::Direct3D11::D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: windows::Win32::Graphics::Direct3D11::D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: subresource,
                    },
                },
            };
            let mut in_view: Option<ID3D11VideoProcessorInputView> = None;
            self.video_device
                .CreateVideoProcessorInputView(
                    &texture.cast::<ID3D11Resource>().unwrap(),
                    enumerator,
                    &in_desc,
                    Some(&mut in_view),
                )
                .map_err(|e| format!("CreateVideoProcessorInputView: {e}"))?;
            let in_view = in_view.ok_or("no input view")?;

            let out_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: windows::Win32::Graphics::Direct3D11::D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: windows::Win32::Graphics::Direct3D11::D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut out_view: Option<ID3D11VideoProcessorOutputView> = None;
            self.video_device
                .CreateVideoProcessorOutputView(
                    &backbuffer.cast::<ID3D11Resource>().unwrap(),
                    enumerator,
                    &out_desc,
                    Some(&mut out_view),
                )
                .map_err(|e| format!("CreateVideoProcessorOutputView: {e}"))?;
            let out_view = out_view.ok_or("no output view")?;

            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: std::mem::ManuallyDrop::new(Some(in_view)),
                ..Default::default()
            };
            let result = self
                .video_context
                .VideoProcessorBlt(processor, &out_view, 0, &[stream.clone()]);
            let mut stream = stream;
            let _ = std::mem::ManuallyDrop::take(&mut stream.pInputSurface);
            result.map_err(|e| format!("VideoProcessorBlt: {e}"))
        }
    }

    unsafe fn teardown(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn letterbox(in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> RECT {
    if in_w == 0 || in_h == 0 || out_w == 0 || out_h == 0 {
        return RECT { left: 0, top: 0, right: out_w.max(1) as i32, bottom: out_h.max(1) as i32 };
    }
    let scale = (out_w as f64 / in_w as f64).min(out_h as f64 / in_h as f64);
    // Never a zero-area dest rect: the sink starts on a 1×1 placeholder window until the
    // frontend pushes the real surface rect, and VideoProcessorBlt rejects an empty
    // destination outright (E_INVALIDARG — found by the rtsp_native H264 bench).
    let mut w = ((in_w as f64 * scale).round() as i32).max(1);
    let mut h = ((in_h as f64 * scale).round() as i32).max(1);
    // The surfaces size themselves to the stream's aspect, so the fit lands within a
    // rounding pixel of the full box — snap that, or the leftover backbuffer column shows
    // as a hairline black edge on the picture (user-visible on the floating window).
    if out_w as i32 - w <= 2 {
        w = out_w as i32;
    }
    if out_h as i32 - h <= 2 {
        h = out_h as i32;
    }
    let x = (out_w as i32 - w) / 2;
    let y = (out_h as i32 - h) / 2;
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}

/// Find + configure the platform decoder MFT for `codec`: low latency, DXGI manager for
/// GPU decode, H264/HEVC bytestream in, NV12 out.
unsafe fn create_decoder(
    codec: SinkCodec,
    manager: &IMFDXGIDeviceManager,
) -> Result<(IMFTransform, bool), String> {
    unsafe {
        let subtype = match codec {
            SinkCodec::H264 => MFVideoFormat_H264,
            SinkCodec::H265 => MFVideoFormat_HEVC,
        };
        let input_info = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: subtype };
        let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input_info),
            None,
            &mut activates,
            &mut count,
        )
        .map_err(|e| format!("MFTEnumEx: {e}"))?;
        if count == 0 || activates.is_null() {
            return Err(match codec {
                SinkCodec::H265 => "No HEVC decoder found — install the 'HEVC Video Extensions' from the Microsoft Store".into(),
                SinkCodec::H264 => "No H264 decoder MFT found".into(),
            });
        }
        let list = std::slice::from_raw_parts(activates, count as usize);
        // Prefer the Microsoft decoder when several register for the subtype — vendor
        // wrappers have shown broken sync-MFT negotiation on the bench.
        let name_of = |a: &IMFActivate| -> String {
            let mut buf = [0u16; 256];
            let mut len = 0u32;
            match a.GetString(&MFT_FRIENDLY_NAME_Attribute, &mut buf, Some(&mut len)) {
                Ok(()) => String::from_utf16_lossy(&buf[..len as usize]),
                Err(_) => String::new(),
            }
        };
        let debug = std::env::var("KITE_SINK_DEBUG").is_ok();
        let mut pick = 0usize;
        for (n, a) in list.iter().enumerate() {
            if let Some(a) = a {
                let name = name_of(a);
                if debug {
                    eprintln!("[sink] decoder candidate {n}: {name}");
                }
                if name.contains("Microsoft") && pick == 0 && n > 0 {
                    pick = n;
                }
            }
        }
        if debug {
            eprintln!("[sink] picking candidate {pick}");
        }
        let decoder: Result<IMFTransform, String> = list[pick]
            .as_ref()
            .ok_or("empty MFT activate".to_string())
            .and_then(|a| a.ActivateObject::<IMFTransform>().map_err(|e| format!("ActivateObject: {e}")));
        for a in list {
            if let Some(a) = a {
                let _ = a.ShutdownObject();
            }
        }
        CoTaskMemFree(Some(activates as *const _));
        let decoder = decoder?;

        if let Ok(attrs) = decoder.GetAttributes() {
            let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
            if debug {
                eprintln!(
                    "[sink] MF_TRANSFORM_ASYNC={:?}",
                    attrs.GetUINT32(&MF_TRANSFORM_ASYNC).ok()
                );
            }
        }
        // GPU decode: hand over the device manager. A non-D3D-aware MFT refuses — that's
        // fine, the CPU output path covers it.
        let d3d_aware = decoder
            .ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)
            .is_ok();
        if !d3d_aware {
            log::warn!("[video] decoder MFT is not D3D11-aware — CPU output path in use");
        }

        let input = MFCreateMediaType().map_err(|e| e.to_string())?;
        input
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| e.to_string())?;
        // Annex-B byte streams: 'HEVC' means length-prefixed MP4-style samples — feeding
        // start-code AUs under it leaves the decoder unable to parse anything (measured:
        // empty output enumeration + an endless stream-change/buffer-too-small dance).
        // The elementary-stream subtype is HEVC_ES. H264's subtype takes Annex-B as-is.
        let input_subtype = match codec {
            SinkCodec::H264 => MFVideoFormat_H264,
            SinkCodec::H265 => MFVideoFormat_HEVC_ES,
        };
        input
            .SetGUID(&MF_MT_SUBTYPE, &input_subtype)
            .map_err(|e| e.to_string())?;
        // The HEVC MFT wants dimensions on the INPUT type as well (placeholder — the
        // bitstream overrides them; the H264 decoder doesn't care either way).
        let _ = input.SetUINT64(&MF_MT_FRAME_SIZE, (1280u64 << 32) | 720);
        let _ = input.SetUINT64(&MF_MT_FRAME_RATE, (30u64 << 32) | 1);
        let _ = input.SetUINT32(&MF_MT_INTERLACE_MODE, 2);
        decoder
            .SetInputType(0, &input, 0)
            .map_err(|e| format!("SetInputType: {e}"))?;

        // Output NV12 — best-effort at init: the H264 decoder accepts it right away,
        // the HEVC MFT may refuse anything before the first input; the drain loop then
        // negotiates on MF_E_TRANSFORM_STREAM_CHANGE / _TYPE_NOT_SET instead.
        let mut i = 0u32;
        let provides = loop {
            let mt: IMFMediaType = decoder
                .GetOutputAvailableType(0, i)
                .map_err(|e| format!("no NV12 output type offered: {e}"))?;
            if mt.GetGUID(&MF_MT_SUBTYPE).map(|g| g == MFVideoFormat_NV12).unwrap_or(false) {
                // The HEVC MFT offers its initial NV12 type WITHOUT a frame size and then
                // refuses SetOutputType over the missing attribute (0xC00D36E6) — the H264
                // decoder fills in defaults. Give it a placeholder; the real size arrives
                // with the first MF_E_TRANSFORM_STREAM_CHANGE and is renegotiated there.
                if mt.GetUINT64(&MF_MT_FRAME_SIZE).is_err() {
                    let _ = mt.SetUINT64(&MF_MT_FRAME_SIZE, (1280u64 << 32) | 720);
                }
                if mt.GetUINT64(&MF_MT_FRAME_RATE).is_err() {
                    let _ = mt.SetUINT64(&MF_MT_FRAME_RATE, (30u64 << 32) | 1);
                }
                if mt.GetUINT64(&MF_MT_PIXEL_ASPECT_RATIO).is_err() {
                    let _ = mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1);
                }
                if mt.GetUINT32(&MF_MT_INTERLACE_MODE).is_err() {
                    let _ = mt.SetUINT32(&MF_MT_INTERLACE_MODE, 2); // MFVideoInterlace_Progressive
                }
                if let Err(e) = decoder.SetOutputType(0, &mt, 0) {
                    log::debug!("[video] initial SetOutputType deferred ({e}) — negotiating after first input");
                    break true; // assume provides-samples; corrected on renegotiation
                }
                let info = decoder
                    .GetOutputStreamInfo(0)
                    .map_err(|e| format!("GetOutputStreamInfo: {e}"))?;
                break info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
            }
            i += 1;
        };

        Ok((decoder, provides))
    }
}

unsafe fn create_child(parent_raw: isize, rect: (i32, i32, i32, i32)) -> Result<HWND, String> {
    unsafe {
        let class = w!("KiteVideoSink");
        if !CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
            let instance = GetModuleHandleW(None).map_err(|e| format!("module handle: {e}"))?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(sink_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                CLASS_REGISTERED.store(false, Ordering::SeqCst);
                return Err("RegisterClassW failed".into());
            }
        }
        let child = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("video"),
            WS_CHILD | WS_VISIBLE,
            rect.0,
            rect.1,
            rect.2.max(1),
            rect.3.max(1),
            Some(HWND(parent_raw as *mut _)),
            None,
            None,
            None,
        )
        .map_err(|e| format!("CreateWindowExW: {e}"))?;
        // Below the WebView2 sibling — the hole-punch position the spike proved.
        let _ = SetWindowPos(
            child,
            Some(HWND_BOTTOM),
            0,
            0,
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                | SWP_NOACTIVATE,
        );
        Ok(child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full Windows decode chain against a real RTSP source (start
    /// tools/rtsp_test_server.py --codec h264 --file <annex-B> first):
    /// `KITE_RTSP_URL=rtsp://... [KITE_RTSP_CODEC=h265] cargo test decodes_and_presents -- --ignored --nocapture`
    /// Opens a real window on the desktop; the decoded picture is verifiable by screenshot.
    #[test]
    #[ignore]
    fn decodes_and_presents_from_a_real_server() {
        let Ok(url) = std::env::var("KITE_RTSP_URL") else {
            eprintln!("KITE_RTSP_URL not set — skipping");
            return;
        };
        let codec = match std::env::var("KITE_RTSP_CODEC").as_deref() {
            Ok("h265") => SinkCodec::H265,
            _ => SinkCodec::H264,
        };

        // Host window with its own pump, standing in for the Tauri main window.
        let (hwnd_tx, hwnd_rx) = std::sync::mpsc::channel::<isize>();
        std::thread::spawn(move || unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, WS_OVERLAPPEDWINDOW};
            let class = w!("KiteSinkBenchHost");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(sink_proc),
                hInstance: GetModuleHandleW(None).unwrap().into(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("Kite sink bench"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                740,
                520,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let _ = hwnd_tx.send(hwnd.0 as isize);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
        let host = hwnd_rx.recv_timeout(Duration::from_secs(5)).expect("host window");
        let mut sink = WinVideoSink::start(host, (40, 40, 640, 400), codec).expect("sink start");

        let cfg = crate::video::rtsp::RtspConfig { url, ..Default::default() };
        let stop = Arc::new(AtomicBool::new(false));
        {
            let stop = stop.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(10));
                stop.store(true, Ordering::Relaxed);
            });
        }
        let mut pushed = 0u64;
        let mut ts_unwrapped: u64 = 0;
        let mut last_ts: Option<u32> = None;
        let stats = {
            let sink = &sink;
            crate::video::rtsp::run_rtsp(&cfg, &stop, &mut |f| {
                if let Some(prev) = last_ts {
                    ts_unwrapped = ts_unwrapped.wrapping_add(f.rtp_timestamp.wrapping_sub(prev) as u64);
                }
                last_ts = Some(f.rtp_timestamp);
                pushed += 1;
                sink.push(f.data, ts_unwrapped);
            })
            .expect("stream")
        };
        std::thread::sleep(Duration::from_millis(500)); // let the tail decode
        let presented = sink.frames_presented();
        let size = sink.picture_size();
        let err = sink.error();
        eprintln!(
            "pushed={pushed} presented={presented} size={size:?} err={err:?} transport={:?} rtp lost={}",
            stats.transport, stats.rtp.lost
        );
        sink.stop();
        drop(sink);
        assert!(err.is_none(), "sink error: {err:?}");
        assert!(presented > 100, "expected >100 presented frames, got {presented}");
        assert_eq!(size.map(|s| s.0), Some(640), "decoded width");
    }
}
