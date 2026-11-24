//! Datagram header for video and audio shards (24 bytes, little-endian).
//!
//! ```text
//!  0  kind           1   Video = 1, Audio = 2 (Input = 3 has no shard header)
//!  1  flags          1   KEYFRAME, RECOVERY (first frame after a reference-frame invalidation)
//!  2  frame_id       4   +1 per frame, per kind
//!  6  group          1   FEC group of this shard, 0..group_count
//!  7  group_count    1   FEC groups in this frame (1..=MAX_GROUPS)
//!  8  shard          2   0..data_shards+parity_shards
//! 10  data_shards    2   k
//! 12  parity_shards  2   m
//! 14  group_len      4   real bytes in this group (the last data shard is zero-padded)
//! 18  capture_us     6   server clock, microseconds, when the frame was captured
//! ```
//!
//! Every field is checked on parse; a datagram that fails is dropped and
//! counted, never trusted.

use thiserror::Error;

pub const PROTOCOL_VERSION: u16 = 1;
/// QUIC ALPN. A peer that does not offer it is refused during the handshake.
pub const ALPN: &[u8] = b"dcl-stream/1";
pub const HEADER_LEN: usize = 24;

/// Hard limits. They bound memory per frame on the receiver whatever a peer sends.
pub const MAX_GROUPS: u8 = 16;
pub const MAX_DATA_SHARDS: u16 = 1024;
pub const MAX_PARITY_SHARDS: u16 = 1024;
pub const MAX_SHARD_BYTES: usize = 1452;
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_CAPTURE_US: u64 = (1 << 48) - 1;

pub mod flags {
    pub const KEYFRAME: u8 = 0x01;
    /// The encoder stopped referencing lost frames before this one (RFI).
    pub const RECOVERY: u8 = 0x02;
    pub const ALL: u8 = KEYFRAME | RECOVERY;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Video = 1,
    Audio = 2,
    /// Latest-wins input (mouse motion, gamepad axes): a postcard body, no shard header.
    Input = 3,
}

impl Kind {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(Kind::Video),
            2 => Some(Kind::Audio),
            3 => Some(Kind::Input),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShardHeader {
    pub kind: Kind,
    pub flags: u8,
    pub frame_id: u32,
    pub group: u8,
    pub group_count: u8,
    pub shard: u16,
    pub data_shards: u16,
    pub parity_shards: u16,
    pub group_len: u32,
    pub capture_us: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WireError {
    #[error("datagram shorter than the {HEADER_LEN}-byte header")]
    Short,
    #[error("unknown datagram kind {0}")]
    UnknownKind(u8),
    #[error("input datagrams carry no shard header")]
    NotShard,
    #[error("unknown flag bits {0:#04x}")]
    Flags(u8),
    #[error("bad FEC layout: {0}")]
    Layout(&'static str),
    #[error("shard payload of {0} bytes is empty, odd or over the limit")]
    ShardSize(usize),
}

impl ShardHeader {
    pub fn write(&self, out: &mut [u8; HEADER_LEN]) {
        out[0] = self.kind as u8;
        out[1] = self.flags;
        out[2..6].copy_from_slice(&self.frame_id.to_le_bytes());
        out[6] = self.group;
        out[7] = self.group_count;
        out[8..10].copy_from_slice(&self.shard.to_le_bytes());
        out[10..12].copy_from_slice(&self.data_shards.to_le_bytes());
        out[12..14].copy_from_slice(&self.parity_shards.to_le_bytes());
        out[14..18].copy_from_slice(&self.group_len.to_le_bytes());
        let cap = (self.capture_us & MAX_CAPTURE_US).to_le_bytes();
        out[18..24].copy_from_slice(&cap[..6]);
    }

    /// Parses and validates a datagram. Returns the header and the shard payload.
    pub fn parse(datagram: &[u8]) -> Result<(ShardHeader, &[u8]), WireError> {
        let first = *datagram.first().ok_or(WireError::Short)?;
        let kind = Kind::from_byte(first).ok_or(WireError::UnknownKind(first))?;
        if kind == Kind::Input {
            return Err(WireError::NotShard);
        }
        if datagram.len() < HEADER_LEN {
            return Err(WireError::Short);
        }
        let d = datagram;
        let u16_at = |i: usize| u16::from_le_bytes([d[i], d[i + 1]]);
        let flags = d[1];
        if flags & !flags::ALL != 0 {
            return Err(WireError::Flags(flags));
        }
        let mut cap = [0u8; 8];
        cap[..6].copy_from_slice(&d[18..24]);
        let h = ShardHeader {
            kind,
            flags,
            frame_id: u32::from_le_bytes([d[2], d[3], d[4], d[5]]),
            group: d[6],
            group_count: d[7],
            shard: u16_at(8),
            data_shards: u16_at(10),
            parity_shards: u16_at(12),
            group_len: u32::from_le_bytes([d[14], d[15], d[16], d[17]]),
            capture_us: u64::from_le_bytes(cap),
        };
        h.validate()?;
        let payload = &d[HEADER_LEN..];
        if payload.is_empty() || payload.len() % 2 != 0 || payload.len() > MAX_SHARD_BYTES {
            return Err(WireError::ShardSize(payload.len()));
        }
        if h.group_len as usize > h.data_shards as usize * payload.len() {
            return Err(WireError::Layout("group_len larger than its data shards"));
        }
        Ok((h, payload))
    }

    fn validate(&self) -> Result<(), WireError> {
        if self.group_count == 0 || self.group_count > MAX_GROUPS {
            return Err(WireError::Layout("group_count"));
        }
        if self.group >= self.group_count {
            return Err(WireError::Layout("group index"));
        }
        if self.data_shards == 0 || self.data_shards > MAX_DATA_SHARDS {
            return Err(WireError::Layout("data_shards"));
        }
        if self.parity_shards > MAX_PARITY_SHARDS {
            return Err(WireError::Layout("parity_shards"));
        }
        if self.shard >= self.data_shards + self.parity_shards {
            return Err(WireError::Layout("shard index"));
        }
        if self.group_len == 0 {
            return Err(WireError::Layout("empty group"));
        }
        Ok(())
    }

    pub fn is_keyframe(&self) -> bool {
        self.flags & flags::KEYFRAME != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ShardHeader {
        ShardHeader {
            kind: Kind::Video,
            flags: flags::KEYFRAME,
            frame_id: 0xDEAD_BEEF,
            group: 1,
            group_count: 3,
            shard: 7,
            data_shards: 10,
            parity_shards: 2,
            group_len: 9000,
            capture_us: 0x0000_1234_5678_9ABC,
        }
    }

    fn datagram(h: &ShardHeader, payload_len: usize) -> Vec<u8> {
        let mut head = [0u8; HEADER_LEN];
        h.write(&mut head);
        let mut d = head.to_vec();
        d.resize(HEADER_LEN + payload_len, 0xAB);
        d
    }

    #[test]
    fn header_round_trips() {
        let h = sample();
        let d = datagram(&h, 1000);
        let (back, payload) = ShardHeader::parse(&d).unwrap();
        assert_eq!(back, h);
        assert_eq!(payload.len(), 1000);
    }

    #[test]
    fn capture_time_keeps_48_bits() {
        let mut h = sample();
        h.capture_us = u64::MAX;
        let (back, _) = ShardHeader::parse(&datagram(&h, 1000)).unwrap();
        assert_eq!(back.capture_us, MAX_CAPTURE_US);
    }

    #[test]
    fn rejects_bad_datagrams() {
        let h = sample();
        assert_eq!(ShardHeader::parse(&[]), Err(WireError::Short));
        assert_eq!(ShardHeader::parse(&[1, 0, 0]), Err(WireError::Short));
        assert_eq!(ShardHeader::parse(&[9; 40]).unwrap_err(), WireError::UnknownKind(9));
        assert_eq!(ShardHeader::parse(&[3; 40]).unwrap_err(), WireError::NotShard);

        let bad = |f: &dyn Fn(&mut ShardHeader)| {
            let mut x = h;
            f(&mut x);
            ShardHeader::parse(&datagram(&x, 1000)).unwrap_err()
        };
        assert!(matches!(bad(&|x| x.flags = 0x80), WireError::Flags(_)));
        assert!(matches!(bad(&|x| x.group_count = 0), WireError::Layout(_)));
        assert!(matches!(bad(&|x| x.group_count = MAX_GROUPS + 1), WireError::Layout(_)));
        assert!(matches!(bad(&|x| x.group = 3), WireError::Layout(_)));
        assert!(matches!(bad(&|x| x.data_shards = 0), WireError::Layout(_)));
        assert!(matches!(bad(&|x| x.data_shards = MAX_DATA_SHARDS + 1), WireError::Layout(_)));
        assert!(matches!(bad(&|x| x.parity_shards = MAX_PARITY_SHARDS + 1), WireError::Layout(_)));
        assert!(matches!(bad(&|x| x.shard = 12), WireError::Layout(_)));
        assert!(matches!(bad(&|x| x.group_len = 0), WireError::Layout(_)));
        assert!(matches!(bad(&|x| x.group_len = 10_001), WireError::Layout(_)));

        assert_eq!(ShardHeader::parse(&datagram(&h, 0)).unwrap_err(), WireError::ShardSize(0));
        assert_eq!(ShardHeader::parse(&datagram(&h, 999)).unwrap_err(), WireError::ShardSize(999));
        assert_eq!(
            ShardHeader::parse(&datagram(&h, MAX_SHARD_BYTES + 2)).unwrap_err(),
            WireError::ShardSize(MAX_SHARD_BYTES + 2)
        );
    }
}
