//! Real capture and encode through FFmpeg's libraries (no ffmpeg process).
//!
//! Video: x11grab captures the guest's X screen, swscale converts and scales
//! it, then the first encoder that opens wins, in the order given
//! (`nvenc`, `vaapi`, `software`). Settings come from the Sunshine analysis
//! (docs/sunshine-moonlight-analysis.md 1.5): low-latency tune, CBR, no
//! B-frames, VBV of one frame, endless GOP with keyframes on request.
//!
//! Audio: PulseAudio (PipeWire's pulse server works too), Opus at 48 kHz
//! stereo in 5 ms packets, low-delay mode, constant bitrate.
//!
//! Not here yet (see the design, phase 6): zero-copy KMS capture, and the
//! direct NVENC SDK path that gives reference-frame invalidation. FFmpeg's
//! encoders cannot invalidate references, so `rfi` is false and every
//! unrecoverable loss costs a keyframe (rate-limited by the driver).

use super::*;
use anyhow::{anyhow, bail, Context as _};
use dcl_stream_proto::now_us;
use ffmpeg_next as ff;
use ff::format::Pixel;
use ff::software::scaling;
use std::sync::Once;
use tracing::{info, warn};

#[derive(Clone, Debug)]
pub enum Capture {
    X11 { display: String },
}

pub struct FfmpegBackend {
    capture: Capture,
    audio: Option<String>,
    /// (codec, encoder family) pairs that opened during the probe, best first.
    available: Vec<(Codec, &'static str)>,
    uinput: bool,
}

fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        ff::init().expect("FFmpeg init");
        ff::device::register_all();
        ff::log::set_level(ff::log::Level::Error);
    });
}

fn encoder_name(codec: Codec, family: &str) -> Option<&'static str> {
    Some(match (codec, family) {
        (Codec::H264, "nvenc") => "h264_nvenc",
        (Codec::Hevc, "nvenc") => "hevc_nvenc",
        (Codec::Av1, "nvenc") => "av1_nvenc",
        (Codec::H264, "vaapi") => "h264_vaapi",
        (Codec::Hevc, "vaapi") => "hevc_vaapi",
        (Codec::Av1, "vaapi") => "av1_vaapi",
        (Codec::H264, "software") => "libx264",
        (Codec::Hevc, "software") => "libx265",
        (Codec::Av1, "software") => "libsvtav1",
        _ => return None,
    })
}

fn family_static(f: &str) -> Option<&'static str> {
    match f {
        "nvenc" => Some("nvenc"),
        "vaapi" => Some("vaapi"),
        "software" => Some("software"),
        _ => None,
    }
}

impl FfmpegBackend {
    pub fn new(capture: Capture, audio: Option<String>, families: Vec<String>, uinput: bool) -> anyhow::Result<Self> {
        init();
        let mut available = Vec::new();
        for fam in families.iter().filter_map(|f| family_static(f)) {
            for codec in [Codec::H264, Codec::Hevc, Codec::Av1] {
                let Some(name) = encoder_name(codec, fam) else { continue };
                match Encoder::open(name, fam, 640, 360, 30, 2_000) {
                    Ok(_) => available.push((codec, fam)),
                    Err(e) => info!("{name} not usable: {e:#}"),
                }
            }
        }
        if available.is_empty() {
            bail!("no video encoder opened (tried {families:?}); is the GPU driver installed?");
        }
        info!(?available, "encoders");
        Ok(FfmpegBackend { capture, audio, available, uinput })
    }
}

impl Backend for FfmpegBackend {
    fn caps(&self) -> Capabilities {
        let mut codecs: Vec<Codec> = Vec::new();
        for (c, _) in &self.available {
            if !codecs.contains(c) {
                codecs.push(*c);
            }
        }
        Capabilities {
            codecs,
            max_width: 3840,
            max_height: 2160,
            max_fps: 240,
            encoder: self.available.first().map(|(_, f)| f.to_string()).unwrap_or_default(),
            audio: self.audio.is_some(),
            rfi: false,
        }
    }

    fn open_video(&self, s: &StreamSettings, codec: Codec, kbps: u32) -> anyhow::Result<Box<dyn VideoPipeline>> {
        let fam = self.available.iter().find(|(c, _)| *c == codec).map(|(_, f)| *f).ok_or_else(|| anyhow!("no encoder for {codec:?}"))?;
        let Capture::X11 { display } = &self.capture;
        Ok(Box::new(FfmpegVideo::new(display, codec, fam, s.width, s.height, s.fps, kbps)?))
    }

    fn open_audio(&self) -> anyhow::Result<Option<Box<dyn AudioPipeline>>> {
        match &self.audio {
            None => Ok(None),
            Some(src) => match FfmpegAudio::new(src) {
                Ok(a) => Ok(Some(Box::new(a))),
                Err(e) => {
                    warn!("audio off: {e:#}");
                    Ok(None)
                }
            },
        }
    }

    fn open_input(&self) -> anyhow::Result<Box<dyn InputSink>> {
        #[cfg(all(feature = "uinput", target_os = "linux"))]
        if self.uinput {
            return Ok(Box::new(super::uinput::Uinput::new()?));
        }
        if self.uinput {
            warn!("this build has no uinput support; input is ignored");
        }
        Ok(Box::new(NoInput))
    }
}

fn find_input_format(name: &str) -> anyhow::Result<ff::Format> {
    ff::device::input::video()
        .chain(ff::device::input::audio())
        .find(|f| f.name() == name)
        .ok_or_else(|| anyhow!("FFmpeg has no '{name}' input device"))
}

/// swscale context that may move to the pipeline thread. It is created and
/// used only from one thread at a time (the pipeline's).
struct Scaler(scaling::Context);
unsafe impl Send for Scaler {}

/// One opened encoder, plus what it needs per frame.
struct Encoder {
    enc: ff::encoder::video::Encoder,
    pixel: Pixel,
    hw: Option<vaapi::Frames>,
    width: u32,
    height: u32,
}

impl Encoder {
    fn open(name: &str, family: &str, width: u32, height: u32, fps: u16, kbps: u32) -> anyhow::Result<Encoder> {
        let codec = ff::encoder::find_by_name(name).ok_or_else(|| anyhow!("{name} not in this FFmpeg"))?;
        let mut ctx = ff::codec::context::Context::new_with_codec(codec).encoder().video()?;
        let bits = kbps as usize * 1000;
        ctx.set_width(width);
        ctx.set_height(height);
        ctx.set_time_base((1, fps as i32));
        ctx.set_frame_rate(Some((fps as i32, 1)));
        ctx.set_bit_rate(bits);
        ctx.set_max_bit_rate(bits);
        ctx.set_max_b_frames(0);
        // Endless GOP: keyframes only at start and on request.
        ctx.set_gop(u32::MAX >> 1);
        let pixel = match family {
            "nvenc" => Pixel::NV12,
            "vaapi" => Pixel::VAAPI,
            _ => Pixel::YUV420P,
        };
        ctx.set_format(pixel);
        unsafe {
            let raw = ctx.as_mut_ptr();
            // VBV of one frame: no frame much bigger than the average, so no
            // frame takes much longer than a frame interval to send.
            (*raw).rc_buffer_size = (bits / fps as usize) as i32;
            (*raw).rc_initial_buffer_occupancy = (*raw).rc_buffer_size;
            (*raw).flags |= ff::ffi::AV_CODEC_FLAG_LOW_DELAY as i32;
        }
        let hw = if family == "vaapi" {
            let frames = vaapi::Frames::new(width, height)?;
            unsafe {
                (*ctx.as_mut_ptr()).hw_frames_ctx = frames.new_ref();
            }
            Some(frames)
        } else {
            None
        };
        let mut o = ff::Dictionary::new();
        o.set("forced-idr", "1");
        match family {
            "nvenc" => {
                o.set("preset", "p1");
                o.set("tune", "ull");
                o.set("rc", "cbr");
                o.set("zerolatency", "1");
                o.set("delay", "0");
            }
            "vaapi" => {
                o.set("rc_mode", "CBR");
                o.set("async_depth", "1");
            }
            _ if name == "libx264" => {
                o.set("preset", "ultrafast");
                o.set("tune", "zerolatency");
            }
            _ if name == "libx265" => {
                o.set("preset", "ultrafast");
                o.set("tune", "zerolatency");
            }
            _ => {
                o.set("preset", "12");
            }
        }
        let enc = ctx.open_with(o).with_context(|| format!("open {name}"))?;
        Ok(Encoder { enc, pixel, hw, width, height })
    }

    /// Sends one frame (in the encoder's software pixel format) and returns
    /// what came out.
    fn encode(&mut self, frame: &mut ff::frame::Video, key: bool) -> anyhow::Result<(Vec<u8>, bool)> {
        frame.set_kind(if key { ff::picture::Type::I } else { ff::picture::Type::None });
        match &self.hw {
            Some(hw) => {
                let mut up = hw.upload(frame)?;
                up.set_pts(frame.pts());
                up.set_kind(frame.kind());
                self.enc.send_frame(&up)?;
            }
            None => self.enc.send_frame(frame)?,
        }
        let mut out = Vec::new();
        let mut is_key = false;
        let mut pkt = ff::Packet::empty();
        while self.enc.receive_packet(&mut pkt).is_ok() {
            is_key |= pkt.is_key();
            out.extend_from_slice(pkt.data().unwrap_or_default());
        }
        Ok((out, is_key))
    }

    /// Software format the frames must be in before `encode`.
    fn sw_pixel(&self) -> Pixel {
        if self.pixel == Pixel::VAAPI {
            Pixel::NV12
        } else {
            self.pixel
        }
    }
}

struct FfmpegVideo {
    input: ff::format::context::Input,
    stream: usize,
    decoder: ff::decoder::Video,
    scaler: Option<Scaler>,
    encoder: Encoder,
    info: VideoInfo,
    family: &'static str,
    kbps: u32,
    pending_kbps: Option<u32>,
    key_next: bool,
    pts: i64,
}

impl FfmpegVideo {
    fn new(display: &str, codec: Codec, family: &'static str, width: u32, height: u32, fps: u16, kbps: u32) -> anyhow::Result<Self> {
        let fmt = find_input_format("x11grab")?;
        let mut o = ff::Dictionary::new();
        o.set("framerate", &fps.to_string());
        o.set("draw_mouse", "1");
        let input = ff::format::open_with(&display.to_string(), &fmt, o).with_context(|| format!("x11grab {display}"))?.input();
        let stream = input.streams().best(ff::media::Type::Video).ok_or_else(|| anyhow!("x11grab gave no video"))?;
        let idx = stream.index();
        let decoder = ff::codec::context::Context::from_parameters(stream.parameters())?.decoder().video()?;
        let name = encoder_name(codec, family).ok_or_else(|| anyhow!("no encoder"))?;
        let encoder = Encoder::open(name, family, width, height, fps, kbps)?;
        info!(screen = %format!("{}x{}", decoder.width(), decoder.height()), stream = %format!("{width}x{height}@{fps}"), name, "video pipeline");
        Ok(FfmpegVideo {
            input,
            stream: idx,
            decoder,
            scaler: None,
            encoder,
            info: VideoInfo { codec, width, height, fps, encoder: name.to_string(), rfi: false },
            family,
            kbps,
            pending_kbps: None,
            key_next: true,
            pts: 0,
        })
    }

    fn convert(&mut self, src: &ff::frame::Video) -> anyhow::Result<ff::frame::Video> {
        let dst_fmt = self.encoder.sw_pixel();
        let (w, h) = (self.encoder.width, self.encoder.height);
        let stale = self.scaler.as_ref().is_none_or(|s| s.0.input().width != src.width() || s.0.input().height != src.height() || s.0.input().format != src.format());
        if stale {
            self.scaler = Some(Scaler(scaling::Context::get(src.format(), src.width(), src.height(), dst_fmt, w, h, scaling::Flags::FAST_BILINEAR)?));
        }
        let mut out = ff::frame::Video::new(dst_fmt, w, h);
        self.scaler.as_mut().expect("scaler").0.run(src, &mut out)?;
        Ok(out)
    }
}

impl VideoPipeline for FfmpegVideo {
    fn info(&self) -> VideoInfo {
        self.info.clone()
    }

    fn next_frame(&mut self) -> anyhow::Result<EncodedFrame> {
        if let Some(k) = self.pending_kbps.take() {
            // FFmpeg encoders take a new bitrate only when reopened; the
            // reopened encoder starts with a keyframe.
            let name = encoder_name(self.info.codec, self.family).expect("known");
            self.encoder = Encoder::open(name, self.family, self.info.width, self.info.height, self.info.fps, k)?;
            self.kbps = k;
            self.key_next = true;
        }
        loop {
            let mut pkt = ff::Packet::empty();
            pkt.read(&mut self.input).context("capture read")?;
            if pkt.stream() != self.stream {
                continue;
            }
            let capture_us = now_us();
            self.decoder.send_packet(&pkt)?;
            let mut raw = ff::frame::Video::empty();
            if self.decoder.receive_frame(&mut raw).is_err() {
                continue;
            }
            let mut frame = self.convert(&raw)?;
            frame.set_pts(Some(self.pts));
            self.pts += 1;
            let key = std::mem::take(&mut self.key_next);
            let (data, is_key) = self.encoder.encode(&mut frame, key)?;
            if data.is_empty() {
                continue;
            }
            return Ok(EncodedFrame { data, keyframe: is_key, capture_us });
        }
    }

    fn force_keyframe(&mut self) {
        self.key_next = true;
    }

    fn set_bitrate(&mut self, kbps: u32) -> anyhow::Result<()> {
        if kbps != self.kbps {
            self.pending_kbps = Some(kbps);
        }
        Ok(())
    }
}

struct FfmpegAudio {
    input: ff::format::context::Input,
    stream: usize,
    decoder: ff::decoder::Audio,
    encoder: ff::encoder::audio::Encoder,
    /// Interleaved stereo s16 samples waiting for a full 5 ms frame.
    fifo: Vec<i16>,
    fifo_start_us: u64,
    out: std::collections::VecDeque<AudioPacket>,
    pts: i64,
}

const OPUS_RATE: u32 = 48_000;
const OPUS_FRAME: usize = 240; // 5 ms

impl FfmpegAudio {
    fn new(source: &str) -> anyhow::Result<Self> {
        let fmt = find_input_format("pulse")?;
        let mut o = ff::Dictionary::new();
        o.set("sample_rate", &OPUS_RATE.to_string());
        o.set("channels", "2");
        o.set("fragment_size", "1920"); // 10 ms of s16 stereo: keeps capture latency low
        let input = ff::format::open_with(&source.to_string(), &fmt, o).with_context(|| format!("pulse {source}"))?.input();
        let stream = input.streams().best(ff::media::Type::Audio).ok_or_else(|| anyhow!("no audio stream"))?;
        let idx = stream.index();
        let decoder = ff::codec::context::Context::from_parameters(stream.parameters())?.decoder().audio()?;
        let codec = ff::encoder::find_by_name("libopus").ok_or_else(|| anyhow!("FFmpeg has no libopus"))?;
        let mut ctx = ff::codec::context::Context::new_with_codec(codec).encoder().audio()?;
        ctx.set_rate(OPUS_RATE as i32);
        ctx.set_channel_layout(ff::ChannelLayout::STEREO);
        ctx.set_format(ff::format::Sample::I16(ff::format::sample::Type::Packed));
        ctx.set_bit_rate(128_000);
        ctx.set_time_base((1, OPUS_RATE as i32));
        let mut o = ff::Dictionary::new();
        o.set("application", "lowdelay");
        o.set("frame_duration", "5");
        o.set("vbr", "off");
        let encoder = ctx.open_with(o).context("open libopus")?;
        Ok(FfmpegAudio {
            input,
            stream: idx,
            decoder,
            encoder,
            fifo: Vec::new(),
            fifo_start_us: 0,
            out: Default::default(),
            pts: 0,
        })
    }

    fn push_samples(&mut self, f: &ff::frame::Audio, capture_us: u64) -> anyhow::Result<()> {
        let want = ff::format::Sample::I16(ff::format::sample::Type::Packed);
        // The pulse device was opened at 48 kHz stereo s16, exactly what Opus
        // takes, so samples are copied as they are: no resampler.
        if f.format() != want || f.rate() != OPUS_RATE || f.channels() != 2 {
            bail!("pulse gave {:?} {} Hz {} channels; want s16 48 kHz stereo", f.format(), f.rate(), f.channels());
        }
        let n = f.samples() * 2;
        let bytes = f.data(0).get(..n * 2).ok_or_else(|| anyhow!("short audio frame"))?;
        if self.fifo.is_empty() {
            self.fifo_start_us = capture_us;
        }
        self.fifo.extend(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])));
        while self.fifo.len() >= OPUS_FRAME * 2 {
            let mut frame = ff::frame::Audio::new(want, OPUS_FRAME, ff::ChannelLayout::STEREO);
            frame.set_rate(OPUS_RATE);
            {
                let dst = frame.data_mut(0);
                for (i, s) in self.fifo.drain(..OPUS_FRAME * 2).enumerate() {
                    dst[i * 2..i * 2 + 2].copy_from_slice(&s.to_le_bytes());
                }
            }
            frame.set_pts(Some(self.pts));
            self.pts += OPUS_FRAME as i64;
            self.encoder.send_frame(&frame).context("opus encode")?;
            let mut pkt = ff::Packet::empty();
            while self.encoder.receive_packet(&mut pkt).is_ok() {
                self.out.push_back(AudioPacket { data: pkt.data().unwrap_or_default().to_vec(), capture_us: self.fifo_start_us });
            }
            self.fifo_start_us += 5_000;
        }
        Ok(())
    }
}

impl AudioPipeline for FfmpegAudio {
    fn next_packet(&mut self) -> anyhow::Result<AudioPacket> {
        loop {
            if let Some(p) = self.out.pop_front() {
                return Ok(p);
            }
            let mut pkt = ff::Packet::empty();
            pkt.read(&mut self.input).context("audio read")?;
            if pkt.stream() != self.stream {
                continue;
            }
            let capture_us = now_us();
            self.decoder.send_packet(&pkt).context("pcm decode")?;
            let mut f = ff::frame::Audio::empty();
            while self.decoder.receive_frame(&mut f).is_ok() {
                self.push_samples(&f, capture_us)?;
            }
        }
    }
}

/// VAAPI hardware frames: the encoder needs its input in GPU surfaces.
mod vaapi {
    use super::ff;
    use anyhow::{bail, Context as _};
    use ff::ffi;
    use std::ptr;

    pub struct Frames {
        device: *mut ffi::AVBufferRef,
        frames: *mut ffi::AVBufferRef,
    }

    // The buffers are reference counted by FFmpeg and only touched from the
    // pipeline's own thread.
    unsafe impl Send for Frames {}

    impl Frames {
        pub fn new(width: u32, height: u32) -> anyhow::Result<Frames> {
            let node = std::env::var("DCL_STREAM_RENDER_NODE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
            let cnode = std::ffi::CString::new(node.clone()).context("render node path")?;
            unsafe {
                let mut device = ptr::null_mut();
                let r = ffi::av_hwdevice_ctx_create(&mut device, ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, cnode.as_ptr(), ptr::null_mut(), 0);
                if r < 0 {
                    bail!("VAAPI device {node}: {}", ff::Error::from(r));
                }
                let frames = ffi::av_hwframe_ctx_alloc(device);
                if frames.is_null() {
                    ffi::av_buffer_unref(&mut device);
                    bail!("av_hwframe_ctx_alloc");
                }
                let fc = (*frames).data as *mut ffi::AVHWFramesContext;
                (*fc).format = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
                (*fc).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
                (*fc).width = width as i32;
                (*fc).height = height as i32;
                (*fc).initial_pool_size = 8;
                // From here Drop releases both buffers on every path.
                let out = Frames { device, frames };
                let r = ffi::av_hwframe_ctx_init(out.frames);
                if r < 0 {
                    bail!("VAAPI frames init: {}", ff::Error::from(r));
                }
                Ok(out)
            }
        }

        /// A new reference for `AVCodecContext.hw_frames_ctx` (the codec owns it).
        pub fn new_ref(&self) -> *mut ffi::AVBufferRef {
            unsafe { ffi::av_buffer_ref(self.frames) }
        }

        pub fn upload(&self, sw: &ff::frame::Video) -> anyhow::Result<ff::frame::Video> {
            unsafe {
                let mut hw = ff::frame::Video::empty();
                let r = ffi::av_hwframe_get_buffer(self.frames, hw.as_mut_ptr(), 0);
                if r < 0 {
                    bail!("VAAPI surface: {}", ff::Error::from(r));
                }
                let r = ffi::av_hwframe_transfer_data(hw.as_mut_ptr(), sw.as_ptr(), 0);
                if r < 0 {
                    bail!("VAAPI upload: {}", ff::Error::from(r));
                }
                Ok(hw)
            }
        }
    }

    impl Drop for Frames {
        fn drop(&mut self) {
            unsafe {
                if !self.frames.is_null() {
                    ffi::av_buffer_unref(&mut self.frames);
                }
                ffi::av_buffer_unref(&mut self.device);
            }
        }
    }
}
