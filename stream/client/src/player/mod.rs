//! The window. Threads:
//! - router: session events -> video decoder / audio player
//! - video: FFmpeg hardware decode (D3D11VA, VideoToolbox, VAAPI; software
//!   fallback), scaled to the window by swscale, handed to the UI
//! - audio: Opus decode -> a short ring buffer -> cpal output
//! - gamepads: gilrs -> input sender
//! - main: winit window, softbuffer blit, keyboard and mouse
//!
//! Shortcuts (like Moonlight): Ctrl+Alt+Shift+Z capture/release the mouse,
//! Ctrl+Alt+Shift+F fullscreen, Ctrl+Alt+Shift+Q quit.
//!
//! First version copies the decoded frame to the CPU before display
//! (estimated 1-3 ms at 1080p); zero-copy display per OS is later work.

pub mod keymap;

use crate::{connect, ClientEvent, ConnectParams, InputSender, KeyframeRequester, Session, Stats};
use anyhow::{anyhow, Context as _};
use dcl_stream_proto::msg::{Codec, InputEvent, MouseButton};
use ffmpeg_next as ff;
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{info, warn};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

enum UserEvent {
    Frame,
    Ended(String),
}

/// The latest picture, already scaled to fit the window (letterboxed).
#[derive(Default)]
struct Picture {
    win_w: u32,
    win_h: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pixels: Vec<u32>,
}

struct Shared {
    picture: Mutex<Option<Picture>>,
    /// Window size the decoder scales to.
    target: Mutex<(u32, u32)>,
    stream: (u32, u32),
    decoded: AtomicU64,
    stop: AtomicBool,
}

pub fn run(rt: tokio::runtime::Runtime, params: ConnectParams) -> anyhow::Result<()> {
    ff::init().context("FFmpeg")?;
    let mut session: Session = rt.block_on(connect(params))?;
    let start = session.start.clone();
    info!(codec = ?start.codec, w = start.width, h = start.height, fps = start.fps, encoder = %start.caps.encoder, "connected");

    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let shared = Arc::new(Shared {
        picture: Mutex::new(None),
        target: Mutex::new((start.width, start.height)),
        stream: (start.width, start.height),
        decoded: AtomicU64::new(0),
        stop: AtomicBool::new(false),
    });

    // The session stays alive (it owns the QUIC endpoint) until the window closes.
    let events = std::mem::replace(&mut session.events, tokio::sync::mpsc::channel(1).1);
    let (input, stats) = (session.input.clone(), session.stats.clone());
    let keyframe = session.keyframe_requester();
    let (vtx, vrx) = mpsc::sync_channel::<dcl_stream_proto::reassembly::CompleteFrame>(2);
    let (atx, arx) = mpsc::sync_channel::<Vec<u8>>(64);

    // Router.
    {
        let proxy = proxy.clone();
        let mut events = events;
        std::thread::Builder::new().name("router".into()).spawn(move || {
            while let Some(ev) = events.blocking_recv() {
                match ev {
                    ClientEvent::Video(f) => {
                        if vtx.send(f).is_err() {
                            break;
                        }
                    }
                    ClientEvent::Audio(f) => {
                        let _ = atx.try_send(f.data);
                    }
                    ClientEvent::Rumble { .. } => {}
                    ClientEvent::Ended(r) => {
                        let _ = proxy.send_event(UserEvent::Ended(r));
                        break;
                    }
                }
            }
        })?;
    }

    // Video decode.
    {
        let shared = shared.clone();
        let proxy = proxy.clone();
        let codec = start.codec;
        let kf = keyframe.clone();
        std::thread::Builder::new().name("decode".into()).spawn(move || {
            if let Err(e) = decode_loop(codec, vrx, &shared, &proxy, &kf) {
                let _ = proxy.send_event(UserEvent::Ended(format!("decoder: {e:#}")));
            }
        })?;
    }

    // Audio.
    let _audio = if start.audio {
        match audio::start(arx) {
            Ok(a) => Some(a),
            Err(e) => {
                warn!("no audio: {e:#}");
                None
            }
        }
    } else {
        None
    };

    // Gamepads.
    {
        let input = input.clone();
        let shared = shared.clone();
        std::thread::Builder::new().name("gamepads".into()).spawn(move || pads::run(input, &shared.stop))?;
    }

    let mut app = App {
        window: None,
        surface: None,
        shared: shared.clone(),
        input,
        stats,
        captured: false,
        mods: ModifiersState::empty(),
        title_at: Instant::now(),
        last_decoded: 0,
        ended: None,
        _proxy: proxy,
    };
    event_loop.run_app(&mut app)?;
    shared.stop.store(true, Ordering::Relaxed);
    rt.block_on(session.close());
    if let Some(r) = app.ended {
        info!("session ended: {r}");
    }
    Ok(())
}

fn codec_id(c: Codec) -> ff::codec::Id {
    match c {
        Codec::H264 => ff::codec::Id::H264,
        Codec::Hevc => ff::codec::Id::HEVC,
        Codec::Av1 => ff::codec::Id::AV1,
    }
}

fn hw_device_type() -> Option<ff::ffi::AVHWDeviceType> {
    use ff::ffi::AVHWDeviceType::*;
    if cfg!(target_os = "windows") {
        Some(AV_HWDEVICE_TYPE_D3D11VA)
    } else if cfg!(target_os = "macos") {
        Some(AV_HWDEVICE_TYPE_VIDEOTOOLBOX)
    } else if cfg!(target_os = "linux") {
        Some(AV_HWDEVICE_TYPE_VAAPI)
    } else {
        None
    }
}

fn open_decoder(codec: Codec, hw: bool) -> anyhow::Result<ff::decoder::Video> {
    let c = ff::decoder::find(codec_id(codec)).ok_or_else(|| anyhow!("no {codec:?} decoder in FFmpeg"))?;
    let mut ctx = ff::codec::context::Context::new_with_codec(c);
    unsafe {
        let raw = ctx.as_mut_ptr();
        (*raw).flags |= ff::ffi::AV_CODEC_FLAG_LOW_DELAY as i32;
        // Slice threads only: frame threads add a frame of delay per thread.
        (*raw).thread_type = ff::ffi::FF_THREAD_SLICE as i32;
        if hw {
            let ty = hw_device_type().ok_or_else(|| anyhow!("no hardware decode on this OS"))?;
            let mut dev = std::ptr::null_mut();
            let r = ff::ffi::av_hwdevice_ctx_create(&mut dev, ty, std::ptr::null(), std::ptr::null_mut(), 0);
            if r < 0 {
                return Err(anyhow!("hardware decoder: {}", ff::Error::from(r)));
            }
            (*raw).hw_device_ctx = dev;
        }
    }
    Ok(ctx.decoder().video()?)
}

fn decode_loop(
    codec: Codec,
    rx: mpsc::Receiver<dcl_stream_proto::reassembly::CompleteFrame>,
    shared: &Shared,
    proxy: &EventLoopProxy<UserEvent>,
    keyframe: &KeyframeRequester,
) -> anyhow::Result<()> {
    let mut dec = match open_decoder(codec, true) {
        Ok(d) => {
            info!("hardware video decode");
            d
        }
        Err(e) => {
            warn!("{e:#}; software decode");
            open_decoder(codec, false)?
        }
    };
    let mut scaler: Option<(ff::software::scaling::Context, (u32, u32, ff::format::Pixel, u32, u32))> = None;
    let mut errors = 0u32;
    while let Ok(f) = rx.recv() {
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }
        let pkt = ff::Packet::copy(&f.data);
        if dec.send_packet(&pkt).is_err() {
            errors += 1;
            if errors % 10 == 1 {
                keyframe.request();
            }
            continue;
        }
        let mut frame = ff::frame::Video::empty();
        while dec.receive_frame(&mut frame).is_ok() {
            let sw = if is_hw(frame.format()) {
                let mut sw = ff::frame::Video::empty();
                let r = unsafe { ff::ffi::av_hwframe_transfer_data(sw.as_mut_ptr(), frame.as_ptr(), 0) };
                if r < 0 {
                    warn!("hardware frame download failed");
                    continue;
                }
                sw
            } else {
                frame.clone()
            };
            let (win_w, win_h) = *shared.target.lock().unwrap();
            let (x, y, w, h) = fit(shared.stream, (win_w, win_h));
            let key = (sw.width(), sw.height(), sw.format(), w, h);
            if scaler.as_ref().is_none_or(|(_, k)| *k != key) {
                let s = ff::software::scaling::Context::get(sw.format(), sw.width(), sw.height(), ff::format::Pixel::BGRZ, w, h, ff::software::scaling::Flags::BILINEAR)?;
                scaler = Some((s, key));
            }
            let mut out = ff::frame::Video::new(ff::format::Pixel::BGRZ, w, h);
            scaler.as_mut().expect("scaler").0.run(&sw, &mut out)?;
            let stride = out.stride(0);
            let data = out.data(0);
            let mut pixels = Vec::with_capacity((w * h) as usize);
            for row in 0..h as usize {
                let line = &data[row * stride..row * stride + w as usize * 4];
                pixels.extend(line.chunks_exact(4).map(|p| u32::from_le_bytes([p[0], p[1], p[2], 0])));
            }
            *shared.picture.lock().unwrap() = Some(Picture { win_w, win_h, x, y, w, h, pixels });
            shared.decoded.fetch_add(1, Ordering::Relaxed);
            let _ = proxy.send_event(UserEvent::Frame);
        }
    }
    Ok(())
}

fn is_hw(p: ff::format::Pixel) -> bool {
    matches!(p, ff::format::Pixel::D3D11 | ff::format::Pixel::VIDEOTOOLBOX | ff::format::Pixel::VAAPI | ff::format::Pixel::CUDA | ff::format::Pixel::D3D11VA_VLD)
}

/// Largest rectangle with the stream's aspect inside the window, centered.
fn fit(stream: (u32, u32), win: (u32, u32)) -> (u32, u32, u32, u32) {
    let (sw, sh) = (stream.0.max(1) as u64, stream.1.max(1) as u64);
    let (ww, wh) = (win.0.max(2) as u64, win.1.max(2) as u64);
    let (w, h) = if ww * sh <= wh * sw { (ww, ww * sh / sw) } else { (wh * sw / sh, wh) };
    let (w, h) = ((w as u32).max(2) & !1, (h as u32).max(2) & !1);
    ((win.0.saturating_sub(w)) / 2, (win.1.saturating_sub(h)) / 2, w, h)
}

struct App {
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    shared: Arc<Shared>,
    input: InputSender,
    stats: Arc<Mutex<Stats>>,
    captured: bool,
    mods: ModifiersState,
    title_at: Instant,
    last_decoded: u64,
    ended: Option<String>,
    _proxy: EventLoopProxy<UserEvent>,
}

impl App {
    fn set_capture(&mut self, on: bool) {
        let Some(w) = &self.window else { return };
        if on {
            let ok = w.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined)).is_ok();
            w.set_cursor_visible(!ok);
            self.captured = ok;
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
            self.captured = false;
        }
    }

    fn shortcut(&self) -> bool {
        self.mods.control_key() && self.mods.alt_key() && self.mods.shift_key()
    }

    fn redraw(&mut self) {
        let (Some(surface), Some(window)) = (self.surface.as_mut(), self.window.as_ref()) else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        let pic = self.shared.picture.lock().unwrap();
        match pic.as_ref() {
            Some(p) if p.win_w == size.width && p.win_h == size.height => {
                buf.fill(0);
                for row in 0..p.h as usize {
                    let dst = (p.y as usize + row) * size.width as usize + p.x as usize;
                    let src = row * p.w as usize;
                    if dst + p.w as usize <= buf.len() {
                        buf[dst..dst + p.w as usize].copy_from_slice(&p.pixels[src..src + p.w as usize]);
                    }
                }
            }
            _ => buf.fill(0),
        }
        drop(pic);
        let _ = buf.present();

        if self.title_at.elapsed() >= Duration::from_secs(1) {
            let n = self.shared.decoded.load(Ordering::Relaxed);
            let fps = (n - self.last_decoded) as f64 / self.title_at.elapsed().as_secs_f64();
            self.last_decoded = n;
            self.title_at = Instant::now();
            let s = self.stats.lock().unwrap().clone();
            window.set_title(&format!(
                "DCL Stream - {fps:.0} fps, {:.1} ms, {:.1} Mbit/s, FEC {}%, lost {}{}",
                s.last_latency_us.unwrap_or(0) as f64 / 1000.0,
                s.video_kbps as f64 / 1000.0,
                s.fec_pct,
                s.frames_lost,
                if self.captured { "  [Ctrl+Alt+Shift+Z releases the mouse]" } else { "  [click to capture the mouse]" }
            ));
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (w, h) = self.shared.stream;
        let attrs = Window::default_attributes()
            .with_title("DCL Stream")
            .with_inner_size(winit::dpi::LogicalSize::new(w.min(1600) as f64, (h.min(900)) as f64));
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.ended = Some(format!("window: {e}"));
                el.exit();
                return;
            }
        };
        let ctx = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        self.surface = Some(softbuffer::Surface::new(&ctx, window.clone()).expect("softbuffer surface"));
        let size = window.inner_size();
        *self.shared.target.lock().unwrap() = (size.width, size.height);
        self.window = Some(window);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                *self.shared.target.lock().unwrap() = (s.width, s.height);
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Focused(false) => self.set_capture(false),
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let down = event.state == ElementState::Pressed;
                if down && self.shortcut() {
                    match code {
                        KeyCode::KeyZ => return self.set_capture(!self.captured),
                        KeyCode::KeyQ => return el.exit(),
                        KeyCode::KeyF => {
                            if let Some(w) = &self.window {
                                w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) });
                            }
                            return;
                        }
                        _ => {}
                    }
                }
                if event.repeat {
                    return; // the guest's X server repeats keys itself
                }
                if let Some(c) = keymap::evdev(code) {
                    self.input.key(c, down);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if !self.captured && state == ElementState::Pressed {
                    self.set_capture(true);
                }
                let b = match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    winit::event::MouseButton::Back => MouseButton::Back,
                    winit::event::MouseButton::Forward => MouseButton::Forward,
                    _ => return,
                };
                self.input.button(b, state == ElementState::Pressed);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => ((x * 120.0) as i32, (-y * 120.0) as i32),
                    MouseScrollDelta::PixelDelta(p) => ((p.x * 2.0) as i32, (-p.y * 2.0) as i32),
                };
                self.input.send(InputEvent::Scroll { dx: dx.clamp(-32000, 32000) as i16, dy: dy.clamp(-32000, 32000) as i16 });
            }
            WindowEvent::CursorMoved { position, .. } if !self.captured => {
                // Desktop use without capture: absolute position on the stream.
                let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
                let (x, y, w, h) = fit(self.shared.stream, (size.width, size.height));
                let nx = ((position.x - x as f64) / w.max(1) as f64).clamp(0.0, 1.0);
                let ny = ((position.y - y as f64) / h.max(1) as f64).clamp(0.0, 1.0);
                self.input.mouse_abs((nx * 65535.0) as u16, (ny * 65535.0) as u16);
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.captured {
                self.input.mouse_move(delta.0.round() as i32, delta.1.round() as i32);
            }
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, e: UserEvent) {
        match e {
            UserEvent::Frame => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::Ended(r) => {
                self.ended = Some(r);
                el.exit();
            }
        }
    }
}

mod audio {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    /// Ring buffer limits: keep about 20 ms queued, never more than 60 ms.
    const TARGET: usize = 48_000 * 2 * 20 / 1000;
    const MAX: usize = 48_000 * 2 * 60 / 1000;

    pub struct Audio {
        _stream: cpal::Stream,
    }

    pub fn start(rx: mpsc::Receiver<Vec<u8>>) -> anyhow::Result<Audio> {
        let ring = Arc::new(Mutex::new(VecDeque::<f32>::with_capacity(MAX * 2)));
        let host = cpal::default_host();
        let dev = host.default_output_device().ok_or_else(|| anyhow!("no audio output device"))?;
        let cfg = cpal::StreamConfig { channels: 2, sample_rate: cpal::SampleRate(48_000), buffer_size: cpal::BufferSize::Default };
        let r = ring.clone();
        let started = Arc::new(AtomicBool::new(false));
        let st = started.clone();
        let stream = dev.build_output_stream(
            &cfg,
            move |out: &mut [f32], _| {
                let mut q = r.lock().unwrap();
                if !st.load(Ordering::Relaxed) && q.len() >= TARGET {
                    st.store(true, Ordering::Relaxed);
                }
                for s in out.iter_mut() {
                    *s = if st.load(Ordering::Relaxed) { q.pop_front().unwrap_or(0.0) } else { 0.0 };
                }
            },
            |e| warn!("audio output: {e}"),
            None,
        )?;
        stream.play()?;

        std::thread::Builder::new().name("audio".into()).spawn(move || {
            let mut dec = match opus_decoder() {
                Ok(d) => d,
                Err(e) => {
                    warn!("opus decoder: {e:#}");
                    return;
                }
            };
            while let Ok(pkt) = rx.recv() {
                if dec.send_packet(&ff::Packet::copy(&pkt)).is_err() {
                    continue;
                }
                let mut f = ff::frame::Audio::empty();
                while dec.receive_frame(&mut f).is_ok() {
                    let n = f.samples();
                    let mut q = ring.lock().unwrap();
                    let planar = f.is_planar();
                    for i in 0..n {
                        for ch in 0..2usize {
                            let v = if planar {
                                sample(f.data(ch.min(f.planes() - 1)), i)
                            } else {
                                sample(f.data(0), i * 2 + ch)
                            };
                            q.push_back(v);
                        }
                    }
                    // Latency cap: drop the oldest audio rather than let it grow.
                    while q.len() > MAX {
                        q.pop_front();
                    }
                }
            }
        })?;
        Ok(Audio { _stream: stream })
    }

    fn sample(plane: &[u8], i: usize) -> f32 {
        plane.get(i * 4..i * 4 + 4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0.0)
    }

    fn opus_decoder() -> anyhow::Result<ff::decoder::Audio> {
        let c = ff::decoder::find(ff::codec::Id::OPUS).ok_or_else(|| anyhow!("FFmpeg has no Opus decoder"))?;
        let mut ctx = ff::codec::context::Context::new_with_codec(c);
        unsafe {
            let raw = ctx.as_mut_ptr();
            (*raw).sample_rate = 48_000;
            ff::ffi::av_channel_layout_default(&mut (*raw).ch_layout, 2);
            (*raw).request_sample_fmt = ff::ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT;
        }
        Ok(ctx.decoder().audio()?)
    }
}

mod pads {
    use super::*;
    use gilrs::{Axis, Button, EventType, Gilrs};
    use std::collections::HashMap;

    fn button(b: Button) -> Option<u16> {
        Some(match b {
            Button::South => 0x130,
            Button::East => 0x131,
            Button::North => 0x133,
            Button::West => 0x134,
            Button::LeftTrigger => 0x136,
            Button::RightTrigger => 0x137,
            Button::Select => 0x13a,
            Button::Start => 0x13b,
            Button::Mode => 0x13c,
            Button::LeftThumb => 0x13d,
            Button::RightThumb => 0x13e,
            _ => return None,
        })
    }

    #[derive(Default, Clone, Copy, PartialEq)]
    struct State {
        axes: [i16; 6],
        hat: (i8, i8),
    }

    pub fn run(input: InputSender, stop: &AtomicBool) {
        let mut g = match Gilrs::new() {
            Ok(g) => g,
            Err(e) => {
                warn!("gamepads off: {e}");
                return;
            }
        };
        let mut ids: HashMap<gilrs::GamepadId, u8> = HashMap::new();
        let mut state: HashMap<u8, State> = HashMap::new();
        let mut sent: HashMap<u8, State> = HashMap::new();
        for (id, _) in g.gamepads() {
            let n = ids.len() as u8;
            if n < 4 {
                ids.insert(id, n);
                input.send(InputEvent::GamepadConnect { pad: n });
            }
        }
        while !stop.load(Ordering::Relaxed) {
            while let Some(ev) = g.next_event() {
                let pad = match ids.get(&ev.id) {
                    Some(p) => *p,
                    None => {
                        let n = (0..4u8).find(|n| !ids.values().any(|v| v == n));
                        let Some(n) = n else { continue };
                        ids.insert(ev.id, n);
                        input.send(InputEvent::GamepadConnect { pad: n });
                        n
                    }
                };
                let s = state.entry(pad).or_default();
                match ev.event {
                    EventType::ButtonPressed(b, _) | EventType::ButtonReleased(b, _) => {
                        let down = matches!(ev.event, EventType::ButtonPressed(..));
                        match b {
                            Button::DPadUp => s.hat.1 = if down { -1 } else { 0 },
                            Button::DPadDown => s.hat.1 = if down { 1 } else { 0 },
                            Button::DPadLeft => s.hat.0 = if down { -1 } else { 0 },
                            Button::DPadRight => s.hat.0 = if down { 1 } else { 0 },
                            _ => {
                                if let Some(code) = button(b) {
                                    input.send(InputEvent::GamepadButton { pad, button: code, down });
                                }
                            }
                        }
                    }
                    EventType::ButtonChanged(Button::LeftTrigger2, v, _) => s.axes[4] = (v * 255.0) as i16,
                    EventType::ButtonChanged(Button::RightTrigger2, v, _) => s.axes[5] = (v * 255.0) as i16,
                    EventType::AxisChanged(a, v, _) => {
                        let v = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
                        match a {
                            Axis::LeftStickX => s.axes[0] = v,
                            Axis::LeftStickY => s.axes[1] = v.saturating_neg(),
                            Axis::RightStickX => s.axes[2] = v,
                            Axis::RightStickY => s.axes[3] = v.saturating_neg(),
                            _ => {}
                        }
                    }
                    EventType::Disconnected => {
                        ids.remove(&ev.id);
                        state.remove(&pad);
                        sent.remove(&pad);
                        input.send(InputEvent::GamepadDisconnect { pad });
                    }
                    _ => {}
                }
            }
            for (pad, s) in &state {
                if sent.get(pad) != Some(s) {
                    input.gamepad_axes(*pad, s.axes, s.hat.0, s.hat.1);
                    sent.insert(*pad, *s);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn fit_letterboxes_and_keeps_aspect() {
        assert_eq!(fit((1920, 1080), (1920, 1080)), (0, 0, 1920, 1080));
        assert_eq!(fit((1920, 1080), (1000, 1000)), (0, 219, 1000, 562));
        assert_eq!(fit((1920, 1080), (2000, 900)), (200, 0, 1600, 900));
    }
}
