//! What the session needs from the machine: a video pipeline (capture +
//! encode), an audio pipeline and an input sink. The synthetic backend runs
//! anywhere (tests, benchmarks of the network path); the FFmpeg and uinput
//! backends are the real ones on the Linux guest.

use dcl_stream_proto::msg::{Capabilities, Codec, InputDatagram, InputEvent, StreamSettings};

pub mod synthetic;

#[cfg(feature = "ffmpeg")]
pub mod ffmpeg;

#[cfg(all(feature = "uinput", target_os = "linux"))]
pub mod uinput;

pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub keyframe: bool,
    /// `dcl_stream_proto::now_us()` when the picture was captured.
    pub capture_us: u64,
}

#[derive(Clone, Debug)]
pub struct VideoInfo {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u16,
    pub encoder: String,
    pub rfi: bool,
}

/// Runs on its own thread. `next_frame` blocks until the next frame is due
/// and encoded.
pub trait VideoPipeline: Send {
    fn info(&self) -> VideoInfo;
    fn next_frame(&mut self) -> anyhow::Result<EncodedFrame>;
    fn force_keyframe(&mut self);
    /// Stop referencing frames `first..=last` (frame indexes since start).
    /// False when the encoder cannot; the driver then sends a keyframe.
    fn invalidate(&mut self, _first: u64, _last: u64) -> bool {
        false
    }
    fn set_bitrate(&mut self, kbps: u32) -> anyhow::Result<()>;
}

pub struct AudioPacket {
    /// One Opus packet (48 kHz stereo).
    pub data: Vec<u8>,
    pub capture_us: u64,
}

pub trait AudioPipeline: Send {
    fn next_packet(&mut self) -> anyhow::Result<AudioPacket>;
}

pub trait InputSink: Send {
    fn event(&mut self, e: InputEvent) -> anyhow::Result<()>;
    fn motion(&mut self, d: InputDatagram) -> anyhow::Result<()>;
}

pub trait Backend: Send + Sync {
    fn caps(&self) -> Capabilities;
    fn open_video(&self, s: &StreamSettings, codec: Codec, kbps: u32) -> anyhow::Result<Box<dyn VideoPipeline>>;
    fn open_audio(&self) -> anyhow::Result<Option<Box<dyn AudioPipeline>>>;
    fn open_input(&self) -> anyhow::Result<Box<dyn InputSink>>;
}

/// Input sink that drops everything (input turned off, or no uinput).
pub struct NoInput;

impl InputSink for NoInput {
    fn event(&mut self, _e: InputEvent) -> anyhow::Result<()> {
        Ok(())
    }
    fn motion(&mut self, _d: InputDatagram) -> anyhow::Result<()> {
        Ok(())
    }
}
