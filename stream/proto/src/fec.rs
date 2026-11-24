//! Sender side: split one encoded frame into FEC groups of equal-size shards,
//! add Reed-Solomon parity, and write a datagram per shard.
//!
//! Unlike GameStream (GF(2^8), at most 4 blocks of 255 shards, FEC turned off
//! for frames that do not fit), `reed-solomon-simd` works in GF(2^16): a group
//! can hold up to 1024 data shards here, and a frame up to 16 groups, so
//! keyframes keep their parity.

use crate::wire::{Kind, ShardHeader, HEADER_LEN, MAX_DATA_SHARDS, MAX_FRAME_BYTES, MAX_GROUPS, MAX_SHARD_BYTES};
use thiserror::Error;

/// Data shards per group. Smaller groups decode faster and a burst loses
/// fewer shards per group; 256 keeps a 1080p keyframe in one or two groups.
pub const DEFAULT_GROUP_DATA_SHARDS: u16 = 256;

#[derive(Clone, Copy, Debug)]
pub struct FrameMeta {
    pub kind: Kind,
    pub flags: u8,
    pub frame_id: u32,
    pub capture_us: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct FecParams {
    /// Payload bytes per shard (even, at most MAX_SHARD_BYTES).
    pub shard_bytes: usize,
    /// Parity as a percentage of data shards, rounded up. 0 turns FEC off.
    pub parity_pct: u8,
    /// Parity floor per group when parity_pct > 0 (so a 1-shard frame still gets one).
    pub min_parity: u16,
    pub group_data_shards: u16,
}

impl FecParams {
    pub fn new(shard_bytes: usize, parity_pct: u8) -> Self {
        FecParams { shard_bytes, parity_pct, min_parity: 1, group_data_shards: DEFAULT_GROUP_DATA_SHARDS }
    }

    /// Largest even shard that fits a datagram of `max_datagram` bytes.
    pub fn shard_bytes_for_datagram(max_datagram: usize) -> usize {
        let room = max_datagram.saturating_sub(HEADER_LEN).min(MAX_SHARD_BYTES);
        room & !1
    }

    fn parity_for(&self, data: u16) -> u16 {
        if self.parity_pct == 0 {
            return 0;
        }
        let p = (data as u32 * self.parity_pct as u32).div_ceil(100) as u16;
        p.max(self.min_parity).min(crate::wire::MAX_PARITY_SHARDS)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FecError {
    #[error("shard size {0} must be even and between 2 and {MAX_SHARD_BYTES}")]
    ShardSize(usize),
    #[error("empty frame")]
    Empty,
    #[error("frame of {0} bytes is over the protocol limit")]
    TooLarge(usize),
    #[error("Reed-Solomon: {0}")]
    Rs(String),
}

/// Returns one datagram (header + shard) per shard, data shards first in each group.
pub fn encode_frame(frame: &[u8], meta: FrameMeta, p: &FecParams) -> Result<Vec<Vec<u8>>, FecError> {
    let s = p.shard_bytes;
    if s < 2 || s % 2 != 0 || s > MAX_SHARD_BYTES {
        return Err(FecError::ShardSize(s));
    }
    if frame.is_empty() {
        return Err(FecError::Empty);
    }
    if frame.len() > MAX_FRAME_BYTES {
        return Err(FecError::TooLarge(frame.len()));
    }
    let group_cap = p.group_data_shards.clamp(1, MAX_DATA_SHARDS) as usize;
    let total_shards = frame.len().div_ceil(s);
    let groups = total_shards.div_ceil(group_cap);
    if groups > MAX_GROUPS as usize {
        return Err(FecError::TooLarge(frame.len()));
    }
    // Spread shards evenly so the last group is not a tiny one with weak parity.
    let per_group = total_shards.div_ceil(groups);

    let mut out = Vec::with_capacity(total_shards + total_shards * p.parity_pct as usize / 100 + groups);
    let mut offset = 0usize;
    for g in 0..groups {
        let start_shard = g * per_group;
        let data = per_group.min(total_shards - start_shard) as u16;
        let group_bytes = (data as usize * s).min(frame.len() - offset);
        let bytes = &frame[offset..offset + group_bytes];
        offset += group_bytes;

        let mut shards: Vec<Vec<u8>> = (0..data as usize)
            .map(|i| {
                let a = (i * s).min(bytes.len());
                let b = ((i + 1) * s).min(bytes.len());
                let mut v = bytes[a..b].to_vec();
                v.resize(s, 0);
                v
            })
            .collect();
        let parity = p.parity_for(data);
        if parity > 0 {
            let rec = reed_solomon_simd::encode(data as usize, parity as usize, &shards)
                .map_err(|e| FecError::Rs(e.to_string()))?;
            shards.extend(rec);
        }
        for (i, shard) in shards.iter().enumerate() {
            let h = ShardHeader {
                kind: meta.kind,
                flags: meta.flags,
                frame_id: meta.frame_id,
                group: g as u8,
                group_count: groups as u8,
                shard: i as u16,
                data_shards: data,
                parity_shards: parity,
                group_len: group_bytes as u32,
                capture_us: meta.capture_us,
            };
            let mut d = vec![0u8; HEADER_LEN + s];
            let mut head = [0u8; HEADER_LEN];
            h.write(&mut head);
            d[..HEADER_LEN].copy_from_slice(&head);
            d[HEADER_LEN..].copy_from_slice(shard);
            out.push(d);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> FrameMeta {
        FrameMeta { kind: Kind::Video, flags: 0, frame_id: 5, capture_us: 42 }
    }

    #[test]
    fn small_frame_gets_one_group_and_min_parity() {
        let d = encode_frame(&[7u8; 100], meta(), &FecParams::new(1000, 20)).unwrap();
        assert_eq!(d.len(), 2); // 1 data + 1 parity (floor)
        let (h, payload) = ShardHeader::parse(&d[0]).unwrap();
        assert_eq!((h.data_shards, h.parity_shards, h.group_len, h.group_count), (1, 1, 100, 1));
        assert_eq!(&payload[..100], &[7u8; 100]);
        assert!(payload[100..].iter().all(|&b| b == 0));
    }

    #[test]
    fn parity_rounds_up_and_zero_turns_it_off() {
        let frame = vec![1u8; 10 * 1000];
        let d = encode_frame(&frame, meta(), &FecParams::new(1000, 15)).unwrap();
        assert_eq!(d.len(), 10 + 2); // ceil(1.5)
        let d = encode_frame(&frame, meta(), &FecParams::new(1000, 0)).unwrap();
        assert_eq!(d.len(), 10);
    }

    #[test]
    fn large_frame_splits_into_even_groups() {
        let mut p = FecParams::new(1000, 10);
        p.group_data_shards = 100;
        let frame = vec![3u8; 250 * 1000 + 1];
        let d = encode_frame(&frame, meta(), &p).unwrap();
        let groups: Vec<_> = d.iter().map(|x| ShardHeader::parse(x).unwrap().0).collect();
        assert!(groups.iter().all(|h| h.group_count == 3));
        let g0 = groups.iter().find(|h| h.group == 0).unwrap();
        let g2 = groups.iter().find(|h| h.group == 2).unwrap();
        assert_eq!(g0.data_shards, 84); // 251 shards over 3 groups
        assert_eq!(g2.data_shards, 83);
        let total: u32 = (0..3).map(|g| groups.iter().find(|h| h.group == g).unwrap().group_len).sum();
        assert_eq!(total as usize, frame.len());
    }

    #[test]
    fn keyframe_sized_frames_keep_fec() {
        // A 2 MB keyframe at 1100-byte shards: 1907 shards, 8 groups, all with parity.
        let frame = vec![9u8; 2 * 1024 * 1024];
        let d = encode_frame(&frame, meta(), &FecParams::new(1100, 20)).unwrap();
        let hs: Vec<_> = d.iter().map(|x| ShardHeader::parse(x).unwrap().0).collect();
        assert!(hs.iter().all(|h| h.parity_shards > 0));
    }

    #[test]
    fn rejects_bad_params() {
        assert_eq!(encode_frame(&[1], meta(), &FecParams::new(999, 0)).unwrap_err(), FecError::ShardSize(999));
        assert_eq!(encode_frame(&[1], meta(), &FecParams::new(0, 0)).unwrap_err(), FecError::ShardSize(0));
        assert_eq!(encode_frame(&[], meta(), &FecParams::new(1000, 0)).unwrap_err(), FecError::Empty);
        let huge = vec![0u8; MAX_FRAME_BYTES + 1];
        assert!(matches!(encode_frame(&huge, meta(), &FecParams::new(1000, 0)), Err(FecError::TooLarge(_))));
    }

    #[test]
    fn shard_size_fits_the_datagram() {
        assert_eq!(FecParams::shard_bytes_for_datagram(1200), 1176);
        assert_eq!(FecParams::shard_bytes_for_datagram(1201), 1176);
        assert_eq!(FecParams::shard_bytes_for_datagram(9000), MAX_SHARD_BYTES);
        assert_eq!(FecParams::shard_bytes_for_datagram(10), 0);
    }
}
