//! Messages on the QUIC control stream, the input stream and input datagrams.
//!
//! Encoding is postcard (compact serde). On streams every message is framed
//! as a u32 little-endian length plus body, capped at `MAX_MSG_BYTES`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_MSG_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub codecs: Vec<Codec>,
    pub max_width: u32,
    pub max_height: u32,
    pub max_fps: u16,
    /// Encoder in use, for the stats overlay ("h264_nvenc", "libx264", "synthetic").
    pub encoder: String,
    pub audio: bool,
    /// The encoder can invalidate reference frames instead of sending a keyframe.
    pub rfi: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u16,
    /// Codecs the client can decode, best first.
    pub codecs: Vec<Codec>,
    pub max_bitrate_kbps: u32,
    pub audio: bool,
}

impl StreamSettings {
    /// Clamps a client's wishes to what the server allows.
    pub fn clamp_to(&self, caps: &Capabilities, max_bitrate_kbps: u32) -> StreamSettings {
        StreamSettings {
            width: self.width.clamp(320, caps.max_width.max(320)) & !1,
            height: self.height.clamp(240, caps.max_height.max(240)) & !1,
            fps: self.fps.clamp(10, caps.max_fps.max(10)),
            codecs: self.codecs.iter().copied().filter(|c| caps.codecs.contains(c)).take(3).collect(),
            max_bitrate_kbps: self.max_bitrate_kbps.clamp(1_000, max_bitrate_kbps.max(1_000)),
            audio: self.audio && caps.audio,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkReport {
    pub interval_ms: u16,
    pub shards_received: u32,
    /// Shards that never arrived (loss before FEC).
    pub shards_missing: u32,
    pub frames_ok: u32,
    pub frames_recovered: u32,
    pub frames_lost: u32,
    /// How much longer frames took than the fastest frame in the last seconds:
    /// growth means a queue is building on the path.
    pub queue_delay_us: u32,
    pub rtt_us: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMsg {
    Hello { version: u16, nonce: [u8; 32], caps: Capabilities },
    AuthOk,
    Start { codec: Codec, width: u32, height: u32, fps: u16, shard_bytes: u16, bitrate_kbps: u32, audio: bool },
    Pong { client_us: u64, server_us: u64 },
    /// Sent when rate control changes bitrate or FEC (for the client's stats).
    Rate { video_kbps: u32, fec_pct: u8 },
    Rumble { pad: u8, low: u16, high: u16 },
    End { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMsg {
    /// First message on the control stream.
    Open { version: u16 },
    Auth { proof: [u8; 32] },
    Settings(StreamSettings),
    FrameLost { first: u32, last: u32 },
    RequestKeyframe,
    Report(NetworkReport),
    Ping { client_us: u64 },
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// Ordered, reliable input (QUIC uni stream): nothing here may be lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputEvent {
    /// `code` is a Linux evdev key code (KEY_A = 30); clients map their keys to it.
    Key { code: u16, down: bool },
    MouseButton { button: MouseButton, down: bool },
    /// 120 units = one wheel notch, like Windows and Linux high-res scroll.
    Scroll { dx: i16, dy: i16 },
    GamepadConnect { pad: u8 },
    GamepadDisconnect { pad: u8 },
    /// `button` is an evdev BTN_* code (BTN_SOUTH = 0x130).
    GamepadButton { pad: u8, button: u16, down: bool },
}

/// Latest-wins input sent as datagrams. A newer `seq` replaces older ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputDatagram {
    MouseMove { seq: u32, dx: i16, dy: i16 },
    /// Position on the stream, 0..=65535 on each axis.
    MouseAbs { seq: u32, x: u16, y: u16 },
    /// Left X/Y, right X/Y, left/right trigger, d-pad.
    GamepadAxes { seq: u32, pad: u8, axes: [i16; 6], hat_x: i8, hat_y: i8 },
}

impl InputDatagram {
    pub fn seq(&self) -> u32 {
        match *self {
            InputDatagram::MouseMove { seq, .. } | InputDatagram::MouseAbs { seq, .. } | InputDatagram::GamepadAxes { seq, .. } => seq,
        }
    }

    pub fn to_datagram(&self) -> Vec<u8> {
        let mut out = vec![crate::wire::Kind::Input as u8];
        out.extend(postcard::to_stdvec(self).expect("input datagram serializes"));
        out
    }

    pub fn from_datagram(d: &[u8]) -> Result<Self, MsgError> {
        match d.split_first() {
            Some((&k, body)) if k == crate::wire::Kind::Input as u8 => Ok(postcard::from_bytes(body)?),
            _ => Err(MsgError::NotInput),
        }
    }
}

#[derive(Debug, Error)]
pub enum MsgError {
    #[error("message of {0} bytes is over the {MAX_MSG_BYTES}-byte limit")]
    TooLarge(usize),
    #[error("not an input datagram")]
    NotInput,
    #[error("decode: {0}")]
    Decode(#[from] postcard::Error),
}

/// Length-prefixed bytes for a stream.
pub fn frame<T: Serialize>(msg: &T) -> Result<Vec<u8>, MsgError> {
    let body = postcard::to_stdvec(msg)?;
    if body.len() > MAX_MSG_BYTES {
        return Err(MsgError::TooLarge(body.len()));
    }
    let mut out = (body.len() as u32).to_le_bytes().to_vec();
    out.extend(body);
    Ok(out)
}

pub fn decode<'a, T: Deserialize<'a>>(body: &'a [u8]) -> Result<T, MsgError> {
    if body.len() > MAX_MSG_BYTES {
        return Err(MsgError::TooLarge(body.len()));
    }
    Ok(postcard::from_bytes(body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> Capabilities {
        Capabilities {
            codecs: vec![Codec::H264, Codec::Hevc],
            max_width: 1920,
            max_height: 1080,
            max_fps: 120,
            encoder: "x".into(),
            audio: true,
            rfi: false,
        }
    }

    #[test]
    fn messages_round_trip_through_frames() {
        let msgs = vec![
            ClientMsg::Auth { proof: [7; 32] },
            ClientMsg::FrameLost { first: 3, last: 9 },
            ClientMsg::Report(NetworkReport { interval_ms: 50, shards_missing: 2, ..Default::default() }),
            ClientMsg::End,
        ];
        for m in msgs {
            let f = frame(&m).unwrap();
            let len = u32::from_le_bytes(f[..4].try_into().unwrap()) as usize;
            assert_eq!(len, f.len() - 4);
            assert_eq!(decode::<ClientMsg>(&f[4..]).unwrap(), m);
        }
        let hello = ServerMsg::Hello { version: 1, nonce: [1; 32], caps: caps() };
        let f = frame(&hello).unwrap();
        assert_eq!(decode::<ServerMsg>(&f[4..]).unwrap(), hello);
    }

    #[test]
    fn junk_does_not_decode() {
        assert!(decode::<ClientMsg>(&[0xFF, 0xFF, 0xFF]).is_err());
        assert!(decode::<ClientMsg>(&vec![0u8; MAX_MSG_BYTES + 1]).is_err());
        assert!(InputDatagram::from_datagram(&[1, 2, 3]).is_err());
        assert!(InputDatagram::from_datagram(&[3, 0xFF]).is_err());
    }

    #[test]
    fn input_datagrams_round_trip() {
        let m = InputDatagram::MouseMove { seq: 9, dx: -5, dy: 300 };
        let d = m.to_datagram();
        assert_eq!(d[0], 3);
        assert_eq!(InputDatagram::from_datagram(&d).unwrap(), m);
        assert_eq!(m.seq(), 9);
    }

    #[test]
    fn settings_are_clamped_to_the_server() {
        let want = StreamSettings {
            width: 7681,
            height: 99,
            fps: 500,
            codecs: vec![Codec::Av1, Codec::Hevc, Codec::H264],
            max_bitrate_kbps: 900_000,
            audio: true,
        };
        let got = want.clamp_to(&caps(), 80_000);
        assert_eq!((got.width, got.height, got.fps), (1920, 240, 120));
        assert_eq!(got.codecs, vec![Codec::Hevc, Codec::H264]);
        assert_eq!(got.max_bitrate_kbps, 80_000);
    }
}
