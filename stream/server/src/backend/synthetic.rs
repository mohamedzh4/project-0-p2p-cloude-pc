//! A backend with no capture device: frames of realistic size whose bytes
//! can be checked on the client (`verify_frame`). Used by the tests and to
//! measure the network path alone, without a GPU.

use super::*;
use dcl_stream_proto::now_us;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct Synthetic {
    pub rfi: bool,
    pub audio: bool,
    /// Every input the client sent, for tests.
    pub input_log: Arc<Mutex<Vec<String>>>,
    /// Every bitrate the driver asked for, for tests.
    pub bitrate_log: Arc<Mutex<Vec<u32>>>,
}

impl Backend for Synthetic {
    fn caps(&self) -> Capabilities {
        Capabilities {
            codecs: vec![Codec::H264, Codec::Hevc, Codec::Av1],
            max_width: 3840,
            max_height: 2160,
            max_fps: 240,
            encoder: "synthetic".into(),
            audio: self.audio,
            rfi: self.rfi,
        }
    }

    fn open_video(&self, s: &StreamSettings, codec: Codec, kbps: u32) -> anyhow::Result<Box<dyn VideoPipeline>> {
        Ok(Box::new(SyntheticVideo {
            info: VideoInfo { codec, width: s.width, height: s.height, fps: s.fps, encoder: "synthetic".into(), rfi: self.rfi },
            kbps,
            index: 0,
            key_next: true,
            next_due: Instant::now(),
            bitrate_log: self.bitrate_log.clone(),
        }))
    }

    fn open_audio(&self) -> anyhow::Result<Option<Box<dyn AudioPipeline>>> {
        Ok(self.audio.then(|| Box::new(SyntheticAudio { index: 0, next_due: Instant::now() }) as Box<dyn AudioPipeline>))
    }

    fn open_input(&self) -> anyhow::Result<Box<dyn InputSink>> {
        Ok(Box::new(LogInput(self.input_log.clone())))
    }
}

struct SyntheticVideo {
    info: VideoInfo,
    kbps: u32,
    index: u64,
    key_next: bool,
    next_due: Instant,
    bitrate_log: Arc<Mutex<Vec<u32>>>,
}

/// Frame bytes: index (8 bytes LE), keyframe flag, then a stream seeded by the index.
pub fn frame_bytes(index: u64, keyframe: bool, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len.max(9));
    out.extend_from_slice(&index.to_le_bytes());
    out.push(keyframe as u8);
    let mut x = index.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    while out.len() < len.max(9) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push(x as u8);
    }
    out
}

/// True when `data` is exactly what `frame_bytes` made for its index.
pub fn verify_frame(data: &[u8]) -> Option<u64> {
    if data.len() < 9 {
        return None;
    }
    let index = u64::from_le_bytes(data[..8].try_into().ok()?);
    (frame_bytes(index, data[8] != 0, data.len()) == data).then_some(index)
}

impl VideoPipeline for SyntheticVideo {
    fn info(&self) -> VideoInfo {
        self.info.clone()
    }

    fn next_frame(&mut self) -> anyhow::Result<EncodedFrame> {
        let now = Instant::now();
        if self.next_due > now {
            std::thread::sleep(self.next_due - now);
        }
        self.next_due += Duration::from_secs_f64(1.0 / self.info.fps as f64);
        let capture_us = now_us();
        let p_size = (self.kbps as usize * 1000 / 8 / self.info.fps as usize).max(200);
        let key = std::mem::take(&mut self.key_next);
        let len = if key { p_size * 6 } else { p_size };
        let data = frame_bytes(self.index, key, len);
        self.index += 1;
        Ok(EncodedFrame { data, keyframe: key, capture_us })
    }

    fn force_keyframe(&mut self) {
        self.key_next = true;
    }

    fn invalidate(&mut self, _first: u64, _last: u64) -> bool {
        self.info.rfi
    }

    fn set_bitrate(&mut self, kbps: u32) -> anyhow::Result<()> {
        self.kbps = kbps;
        self.bitrate_log.lock().unwrap().push(kbps);
        Ok(())
    }
}

struct SyntheticAudio {
    index: u64,
    next_due: Instant,
}

impl AudioPipeline for SyntheticAudio {
    fn next_packet(&mut self) -> anyhow::Result<AudioPacket> {
        let now = Instant::now();
        if self.next_due > now {
            std::thread::sleep(self.next_due - now);
        }
        self.next_due += Duration::from_millis(5);
        let data = frame_bytes(self.index, false, 100);
        self.index += 1;
        Ok(AudioPacket { data, capture_us: now_us() })
    }
}

struct LogInput(Arc<Mutex<Vec<String>>>);

impl InputSink for LogInput {
    fn event(&mut self, e: InputEvent) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(format!("{e:?}"));
        Ok(())
    }
    fn motion(&mut self, d: InputDatagram) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(format!("{d:?}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_verify_and_tampering_is_caught() {
        let f = frame_bytes(42, true, 5000);
        assert_eq!(verify_frame(&f), Some(42));
        let mut bad = f.clone();
        bad[4000] ^= 1;
        assert_eq!(verify_frame(&bad), None);
        // a prefix is still valid on its own: lengths come from the shard headers
        assert_eq!(verify_frame(&f[..4999]), Some(42));
        assert_eq!(verify_frame(&f[..5]), None);
    }

    #[test]
    fn keyframes_are_bigger_and_bitrate_sets_size() {
        let b = Synthetic::default();
        let s = StreamSettings { width: 1280, height: 720, fps: 100, codecs: vec![Codec::H264], max_bitrate_kbps: 10_000, audio: false };
        let mut v = b.open_video(&s, Codec::H264, 8_000).unwrap();
        let k = v.next_frame().unwrap();
        let p = v.next_frame().unwrap();
        assert!(k.keyframe && !p.keyframe);
        assert_eq!(p.data.len(), 10_000);
        assert_eq!(k.data.len(), 60_000);
        v.set_bitrate(4_000).unwrap();
        assert_eq!(v.next_frame().unwrap().data.len(), 5_000);
        v.force_keyframe();
        assert!(v.next_frame().unwrap().keyframe);
        assert_eq!(*b.bitrate_log.lock().unwrap(), vec![4_000]);
    }
}
