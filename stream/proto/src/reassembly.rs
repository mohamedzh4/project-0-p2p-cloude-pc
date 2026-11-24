//! Receiver side: collect shards, rebuild frames with Reed-Solomon, decide
//! early when a frame is lost, and hand frames out in order.
//!
//! Loss is declared as soon as it is certain or very likely:
//! - **speculative** (Moonlight's idea): shards of a group arrive in order, so
//!   when more shards are already missing than the group has parity, the frame
//!   cannot be rebuilt and we report it at once, before its last shard is due.
//!   Any out-of-order shard turns this off for a cool-down period.
//! - a shard of a frame `reorder_frames` ahead arrived, or a later frame is complete;
//! - the frame has waited longer than `timeout_us`.
//!
//! Memory is bounded: frames in flight, bytes buffered and every count in a
//! header are capped, whatever the peer sends.

use crate::wire::{Kind, ShardHeader, MAX_FRAME_BYTES};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct AssemblerConfig {
    /// Hand frames out in frame-id order (video). Audio uses false: each packet
    /// is independent and goes out as soon as it is complete.
    pub ordered: bool,
    pub reorder_frames: u32,
    pub timeout_us: u64,
    pub max_frames_in_flight: usize,
    pub max_buffered_bytes: usize,
    pub speculative: bool,
    pub speculative_cooldown_us: u64,
}

impl AssemblerConfig {
    pub fn video() -> Self {
        AssemblerConfig {
            ordered: true,
            reorder_frames: 2,
            timeout_us: 150_000,
            max_frames_in_flight: 16,
            max_buffered_bytes: 48 * 1024 * 1024,
            speculative: true,
            speculative_cooldown_us: 300_000_000,
        }
    }

    pub fn audio() -> Self {
        AssemblerConfig {
            ordered: false,
            reorder_frames: 8,
            timeout_us: 60_000,
            max_frames_in_flight: 64,
            max_buffered_bytes: 1024 * 1024,
            speculative: false,
            speculative_cooldown_us: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteFrame {
    pub kind: Kind,
    pub frame_id: u32,
    pub flags: u8,
    pub capture_us: u64,
    pub data: Vec<u8>,
    /// Parity was needed to rebuild at least one group.
    pub recovered: bool,
    pub first_shard_us: u64,
    pub complete_us: u64,
}

impl CompleteFrame {
    pub fn is_keyframe(&self) -> bool {
        self.flags & crate::wire::flags::KEYFRAME != 0
    }
    pub fn is_recovery(&self) -> bool {
        self.flags & crate::wire::flags::RECOVERY != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Frame(CompleteFrame),
    /// Frames `first..=last` will never be complete.
    Lost { first: u32, last: u32, speculative: bool },
}

/// Counters since the last `take_interval`. `shards_missing` is loss before FEC,
/// which is what the sender's FEC and bitrate control need.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IntervalStats {
    pub shards_received: u64,
    pub shards_missing: u64,
    pub frames_ok: u64,
    pub frames_recovered: u64,
    pub frames_lost: u64,
    pub malformed: u64,
    pub late: u64,
}

struct Group {
    data: u16,
    parity: u16,
    group_len: u32,
    shard_len: usize,
    shards: Vec<Option<Vec<u8>>>,
    received: u16,
    highest: Option<u16>,
    done: Option<Vec<u8>>,
    used_parity: bool,
}

impl Group {
    /// Shards before the highest index seen that never arrived (in-order delivery assumed).
    fn missing_so_far(&self) -> u16 {
        self.highest.map_or(0, |h| h + 1 - self.received)
    }
}

enum Slot {
    Collecting {
        flags: u8,
        capture_us: u64,
        group_count: u8,
        groups: BTreeMap<u8, Group>,
        bytes: usize,
    },
    Ready(CompleteFrame),
    /// Delivered or lost; kept so late shards are ignored, not restarted.
    Finished,
}

struct FrameEntry {
    first_seen_us: u64,
    slot: Slot,
}

pub struct Assembler {
    kind: Kind,
    cfg: AssemblerConfig,
    frames: BTreeMap<u32, FrameEntry>,
    next: Option<u32>,
    newest: Option<u32>,
    buffered: usize,
    spec_off_until_us: u64,
    stats: IntervalStats,
}

impl Assembler {
    pub fn new(kind: Kind, cfg: AssemblerConfig) -> Self {
        Assembler {
            kind,
            cfg,
            frames: BTreeMap::new(),
            next: None,
            newest: None,
            buffered: 0,
            spec_off_until_us: 0,
            stats: IntervalStats::default(),
        }
    }

    pub fn take_interval(&mut self) -> IntervalStats {
        std::mem::take(&mut self.stats)
    }

    pub fn speculative_active(&self, now_us: u64) -> bool {
        self.cfg.speculative && now_us >= self.spec_off_until_us
    }

    /// Feeds one parsed datagram.
    pub fn push(&mut self, h: &ShardHeader, payload: &[u8], now_us: u64) -> Vec<Event> {
        let mut events = Vec::new();
        if h.kind != self.kind {
            self.stats.malformed += 1;
            return events;
        }
        let floor = self.floor();
        if floor.is_some_and(|f| h.frame_id < f) {
            self.stats.late += 1;
            return events;
        }
        if self.next.is_none() {
            self.next = Some(h.frame_id);
        }
        if let Some(n) = self.newest {
            if h.frame_id < n && self.is_collecting(h.frame_id) {
                self.out_of_order(now_us);
            }
        }
        self.newest = Some(self.newest.map_or(h.frame_id, |n| n.max(h.frame_id)));

        self.stats.shards_received += 1;
        match self.store(h, payload, now_us) {
            Stored::Malformed => {
                self.stats.shards_received -= 1;
                self.stats.malformed += 1;
            }
            Stored::Late => self.stats.late += 1,
            Stored::Pending => {
                if self.speculative_active(now_us) && self.group_hopeless(h) {
                    self.finish_lost(h.frame_id, true, &mut events);
                }
            }
            Stored::FrameDone => {
                if !self.cfg.ordered {
                    if let Some(FrameEntry { slot, .. }) = self.frames.get_mut(&h.frame_id) {
                        if let Slot::Ready(f) = std::mem::replace(slot, Slot::Finished) {
                            events.push(Event::Frame(f));
                        }
                    }
                }
            }
        }
        self.advance(now_us, &mut events);
        events
    }

    /// Call regularly (every few ms) so timeouts fire without new traffic.
    pub fn poll(&mut self, now_us: u64) -> Vec<Event> {
        let mut events = Vec::new();
        self.advance(now_us, &mut events);
        events
    }

    fn floor(&self) -> Option<u32> {
        if self.cfg.ordered {
            self.next
        } else {
            self.newest.map(|n| n.saturating_sub(256))
        }
    }

    fn is_collecting(&self, id: u32) -> bool {
        matches!(self.frames.get(&id), Some(FrameEntry { slot: Slot::Collecting { .. }, .. }))
    }

    fn out_of_order(&mut self, now_us: u64) {
        if self.cfg.speculative {
            self.spec_off_until_us = now_us + self.cfg.speculative_cooldown_us;
        }
    }

    fn store(&mut self, h: &ShardHeader, payload: &[u8], now_us: u64) -> Stored {
        let Assembler { frames, cfg, buffered, spec_off_until_us, stats, .. } = self;
        let entry = frames.entry(h.frame_id).or_insert_with(|| FrameEntry {
            first_seen_us: now_us,
            slot: Slot::Collecting {
                flags: h.flags,
                capture_us: h.capture_us,
                group_count: h.group_count,
                groups: BTreeMap::new(),
                bytes: 0,
            },
        });
        let first_seen_us = entry.first_seen_us;
        let Slot::Collecting { flags, capture_us, group_count, groups, bytes } = &mut entry.slot else {
            return Stored::Late;
        };
        if *flags != h.flags || *capture_us != h.capture_us || *group_count != h.group_count {
            return Stored::Malformed;
        }
        let total = (h.data_shards + h.parity_shards) as usize;
        let group = groups.entry(h.group).or_insert_with(|| Group {
            data: h.data_shards,
            parity: h.parity_shards,
            group_len: h.group_len,
            shard_len: payload.len(),
            shards: vec![None; total],
            received: 0,
            highest: None,
            done: None,
            used_parity: false,
        });
        if group.data != h.data_shards
            || group.parity != h.parity_shards
            || group.group_len != h.group_len
            || group.shard_len != payload.len()
        {
            return Stored::Malformed;
        }
        if group.done.is_some() {
            return Stored::Late;
        }
        let idx = h.shard as usize;
        if group.shards[idx].is_some() {
            return Stored::Late;
        }
        if group.highest.is_some_and(|hi| h.shard < hi) && cfg.speculative {
            // reordering inside a group: speculation would guess wrong
            *spec_off_until_us = now_us + cfg.speculative_cooldown_us;
        }
        if *bytes + payload.len() > MAX_FRAME_BYTES * 2 || *buffered + payload.len() > cfg.max_buffered_bytes {
            return Stored::Malformed;
        }
        group.shards[idx] = Some(payload.to_vec());
        group.received += 1;
        group.highest = Some(group.highest.map_or(h.shard, |hi| hi.max(h.shard)));
        *bytes += payload.len();
        *buffered += payload.len();

        if group.received < group.data {
            return Stored::Pending;
        }
        // Enough shards: rebuild the group.
        let freed: usize = group.shards.iter().flatten().map(Vec::len).sum();
        match rebuild(group) {
            Some(data) => {
                stats.shards_missing += group.missing_so_far() as u64;
                group.used_parity = group.shards[..group.data as usize].iter().any(Option::is_none);
                group.done = Some(data);
                group.shards = Vec::new();
                *bytes -= freed;
                *buffered -= freed;
            }
            None => return Stored::Malformed,
        }
        if groups.len() < *group_count as usize || groups.values().any(|g| g.done.is_none()) {
            return Stored::Pending;
        }
        let recovered = groups.values().any(|g| g.used_parity);
        let mut data = Vec::with_capacity(groups.values().map(|g| g.group_len as usize).sum());
        for g in groups.values_mut() {
            data.extend_from_slice(&g.done.take().unwrap_or_default());
        }
        stats.frames_ok += 1;
        if recovered {
            stats.frames_recovered += 1;
        }
        entry.slot = Slot::Ready(CompleteFrame {
            kind: h.kind,
            frame_id: h.frame_id,
            flags: h.flags,
            capture_us: h.capture_us,
            data,
            recovered,
            first_shard_us: first_seen_us,
            complete_us: now_us,
        });
        Stored::FrameDone
    }

    fn group_hopeless(&self, h: &ShardHeader) -> bool {
        let Some(FrameEntry { slot: Slot::Collecting { groups, .. }, .. }) = self.frames.get(&h.frame_id) else {
            return false;
        };
        groups.get(&h.group).is_some_and(|g| g.done.is_none() && g.missing_so_far() > g.parity)
    }

    fn finish_lost(&mut self, id: u32, speculative: bool, events: &mut Vec<Event>) {
        if let Some(e) = self.frames.get_mut(&id) {
            if let Slot::Collecting { groups, .. } = &e.slot {
                let (missing, freed) = groups.values().fold((0u64, 0usize), |(m, f), g| {
                    let total = (g.data + g.parity) as u64;
                    let have = g.shards.iter().flatten().map(Vec::len).sum::<usize>();
                    // a rebuilt group already counted its missing shards
                    let miss = if g.done.is_some() { 0 } else { total - g.received as u64 };
                    (m + miss, f + have)
                });
                self.stats.shards_missing += missing;
                self.buffered -= freed;
            }
            e.slot = Slot::Finished;
        } else {
            self.frames.insert(id, FrameEntry { first_seen_us: 0, slot: Slot::Finished });
        }
        self.stats.frames_lost += 1;
        push_lost(events, id, id, speculative);
    }

    fn advance(&mut self, now_us: u64, events: &mut Vec<Event>) {
        if self.cfg.ordered {
            self.advance_ordered(now_us, events);
        } else {
            self.advance_unordered(now_us, events);
        }
    }

    fn advance_ordered(&mut self, now_us: u64, events: &mut Vec<Event>) {
        while let Some(n) = self.next {
            let decision = match self.frames.get(&n) {
                Some(FrameEntry { slot: Slot::Ready(_), .. }) => Next::Deliver,
                Some(FrameEntry { slot: Slot::Finished, .. }) => Next::Skip,
                Some(FrameEntry { first_seen_us, .. }) => {
                    if self.give_up(n, Some(*first_seen_us), now_us) {
                        Next::Lose
                    } else {
                        Next::Wait
                    }
                }
                None => {
                    if self.give_up(n, None, now_us) {
                        Next::LoseGap
                    } else {
                        Next::Wait
                    }
                }
            };
            match decision {
                Next::Wait => break,
                Next::Deliver => {
                    if let Some(FrameEntry { slot: Slot::Ready(f), .. }) = self.frames.remove(&n) {
                        events.push(Event::Frame(f));
                    }
                    self.next = Some(n + 1);
                }
                Next::Skip => {
                    self.frames.remove(&n);
                    self.next = Some(n + 1);
                }
                Next::Lose => {
                    self.finish_lost(n, false, events);
                    self.frames.remove(&n);
                    self.next = Some(n + 1);
                }
                Next::LoseGap => {
                    // No shard of frames n.. arrived at all: lose up to the next known frame.
                    let upto = self.frames.range(n..).next().map(|(k, _)| *k).unwrap_or_else(|| self.newest.map_or(n, |x| x + 1));
                    let last = upto.saturating_sub(1).max(n);
                    self.stats.frames_lost += (last - n + 1) as u64;
                    push_lost(events, n, last, false);
                    self.next = Some(last + 1);
                }
            }
        }
    }

    fn give_up(&self, n: u32, first_seen: Option<u64>, now_us: u64) -> bool {
        let Some(newest) = self.newest else { return false };
        if newest > n && newest - n >= self.cfg.reorder_frames {
            return true;
        }
        if self.frames.len() > self.cfg.max_frames_in_flight {
            return true;
        }
        let later = self.frames.range(n + 1..);
        let mut later_first_seen = None;
        for (_, e) in later {
            if matches!(e.slot, Slot::Ready(_)) {
                return true;
            }
            later_first_seen = Some(later_first_seen.map_or(e.first_seen_us, |t: u64| t.min(e.first_seen_us)));
        }
        let since = first_seen.or(later_first_seen);
        since.is_some_and(|t| now_us.saturating_sub(t) >= self.cfg.timeout_us)
    }

    fn advance_unordered(&mut self, now_us: u64, events: &mut Vec<Event>) {
        let timed_out: Vec<u32> = self
            .frames
            .iter()
            .filter(|(_, e)| matches!(e.slot, Slot::Collecting { .. }) && now_us.saturating_sub(e.first_seen_us) >= self.cfg.timeout_us)
            .map(|(k, _)| *k)
            .collect();
        for id in timed_out {
            self.finish_lost(id, false, events);
        }
        if let Some(floor) = self.floor() {
            let below: Vec<u32> = self.frames.range(..floor).map(|(k, _)| *k).collect();
            for id in below {
                if self.is_collecting(id) {
                    self.finish_lost(id, false, events);
                }
                self.frames.remove(&id);
            }
        }
    }
}

enum Stored {
    Pending,
    FrameDone,
    Late,
    Malformed,
}

enum Next {
    Wait,
    Deliver,
    Skip,
    Lose,
    LoseGap,
}

fn push_lost(events: &mut Vec<Event>, first: u32, last: u32, speculative: bool) {
    if let Some(Event::Lost { last: l, speculative: s, .. }) = events.last_mut() {
        if *s == speculative && *l + 1 == first {
            *l = last;
            return;
        }
    }
    events.push(Event::Lost { first, last, speculative });
}

/// Rebuilds a group's bytes from any `data` of its shards.
fn rebuild(g: &Group) -> Option<Vec<u8>> {
    let k = g.data as usize;
    let mut out = Vec::with_capacity(k * g.shard_len);
    if g.shards[..k].iter().all(Option::is_some) {
        for s in g.shards[..k].iter().flatten() {
            out.extend_from_slice(s);
        }
    } else {
        let originals = g.shards[..k].iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i, s)));
        let recovery = g.shards[k..].iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i, s)));
        let restored = reed_solomon_simd::decode(k, g.parity as usize, originals, recovery).ok()?;
        for (i, s) in g.shards[..k].iter().enumerate() {
            match s {
                Some(s) => out.extend_from_slice(s),
                None => out.extend_from_slice(restored.get(&i)?),
            }
        }
    }
    out.truncate(g.group_len as usize);
    Some(out)
}

/// Decides which complete frames the decoder may use after a loss.
///
/// After frames are lost, later frames reference missing data and would show
/// corruption. They are dropped until a keyframe, or a frame the encoder made
/// after invalidating the lost references (`RECOVERY`), arrives past the loss.
/// Frames older than the loss are still fine.
#[derive(Debug, Default)]
pub struct RecoveryGate {
    broken: Option<(u32, u32)>,
}

impl RecoveryGate {
    pub fn on_lost(&mut self, first: u32, last: u32) {
        self.broken = Some(match self.broken {
            Some((f, l)) => (f.min(first), l.max(last)),
            None => (first, last),
        });
    }

    pub fn is_broken(&self) -> bool {
        self.broken.is_some()
    }

    pub fn admit(&mut self, f: &CompleteFrame) -> bool {
        match self.broken {
            None => true,
            Some((first, _)) if f.frame_id < first => true,
            Some((_, last)) if f.frame_id > last && (f.is_keyframe() || f.is_recovery()) => {
                self.broken = None;
                true
            }
            Some(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fec::{encode_frame, FecParams, FrameMeta};
    use crate::wire::flags;

    fn frame_bytes(id: u32, len: usize) -> Vec<u8> {
        (0..len).map(|i| (i as u32).wrapping_mul(31).wrapping_add(id) as u8).collect()
    }

    fn shards(id: u32, len: usize, pct: u8, fl: u8) -> Vec<Vec<u8>> {
        let meta = FrameMeta { kind: Kind::Video, flags: fl, frame_id: id, capture_us: 1000 + id as u64 };
        encode_frame(&frame_bytes(id, len), meta, &FecParams::new(1000, pct)).unwrap()
    }

    fn feed(a: &mut Assembler, d: &[u8], t: u64) -> Vec<Event> {
        let (h, p) = ShardHeader::parse(d).unwrap();
        a.push(&h, p, t)
    }

    fn frames(ev: &[Event]) -> Vec<u32> {
        ev.iter().filter_map(|e| if let Event::Frame(f) = e { Some(f.frame_id) } else { None }).collect()
    }

    #[test]
    fn whole_frames_come_out_in_order_and_intact() {
        let mut a = Assembler::new(Kind::Video, AssemblerConfig::video());
        let mut got = Vec::new();
        for id in 0..5 {
            for d in shards(id, 5000 + id as usize * 77, 20, 0) {
                got.extend(feed(&mut a, &d, 10));
            }
        }
        assert_eq!(frames(&got), vec![0, 1, 2, 3, 4]);
        for e in &got {
            if let Event::Frame(f) = e {
                assert_eq!(f.data, frame_bytes(f.frame_id, 5000 + f.frame_id as usize * 77));
                assert!(!f.recovered);
            }
        }
        let s = a.take_interval();
        assert_eq!((s.frames_ok, s.frames_lost, s.shards_missing), (5, 0, 0));
    }

    #[test]
    fn parity_rebuilds_lost_data_shards() {
        let mut a = Assembler::new(Kind::Video, AssemblerConfig::video());
        let d = shards(0, 10_000, 20, 0); // 10 data + 2 parity
        let mut ev = Vec::new();
        for (i, s) in d.iter().enumerate() {
            if i == 3 || i == 7 {
                continue;
            }
            ev.extend(feed(&mut a, s, 5));
        }
        let Event::Frame(f) = &ev[0] else { panic!("{ev:?}") };
        assert_eq!(f.data, frame_bytes(0, 10_000));
        assert!(f.recovered);
        let s = a.take_interval();
        assert_eq!((s.frames_recovered, s.shards_missing), (1, 2));
    }

    #[test]
    fn speculative_loss_is_reported_before_the_frame_ends() {
        let mut a = Assembler::new(Kind::Video, AssemblerConfig::video());
        let d = shards(0, 10_000, 20, 0); // parity 2
        // shards 0..=2 lost, shard 3 arrives: 3 missing > 2 parity
        let ev = feed(&mut a, &d[3], 5);
        assert_eq!(ev, vec![Event::Lost { first: 0, last: 0, speculative: true }]);
        // the rest of frame 0 is ignored, frame 1 flows
        for s in &d[4..] {
            assert!(feed(&mut a, s, 6).is_empty());
        }
        let mut ev = Vec::new();
        for s in shards(1, 3000, 20, 0) {
            ev.extend(feed(&mut a, &s, 7));
        }
        assert_eq!(frames(&ev), vec![1]);
    }

    #[test]
    fn out_of_order_turns_speculation_off() {
        let mut a = Assembler::new(Kind::Video, AssemblerConfig::video());
        let mut ev = Vec::new();
        let d0 = shards(0, 3000, 20, 0);
        ev.extend(feed(&mut a, &d0[1], 1));
        ev.extend(feed(&mut a, &d0[0], 1)); // out of order inside a group
        for s in &d0[2..] {
            ev.extend(feed(&mut a, s, 1));
        }
        assert_eq!(frames(&ev), vec![0]);
        assert!(!a.speculative_active(2));
        // frame 1 loses shards 0..=3 (more than its 2 parity): no early report now
        let d1 = shards(1, 10_000, 20, 0);
        ev.clear();
        for s in &d1[4..] {
            ev.extend(feed(&mut a, s, 3));
        }
        assert!(ev.is_empty(), "{ev:?}");
        // frame 2 complete makes frame 1 lost
        for s in shards(2, 2000, 20, 0) {
            ev.extend(feed(&mut a, &s, 4));
        }
        assert_eq!(ev[0], Event::Lost { first: 1, last: 1, speculative: false });
        assert_eq!(frames(&ev), vec![2]);
    }

    #[test]
    fn a_frame_with_no_shards_at_all_is_reported_as_a_gap() {
        let mut a = Assembler::new(Kind::Video, AssemblerConfig::video());
        let mut ev = Vec::new();
        for s in shards(0, 2000, 20, 0) {
            ev.extend(feed(&mut a, &s, 1));
        }
        for id in [3, 4] {
            for s in shards(id, 2000, 20, 0) {
                ev.extend(feed(&mut a, &s, 2));
            }
        }
        assert_eq!(frames(&ev), vec![0, 3, 4]);
        assert!(ev.contains(&Event::Lost { first: 1, last: 2, speculative: false }));
        assert_eq!(a.take_interval().frames_lost, 2);
    }

    #[test]
    fn timeout_loses_a_stuck_frame() {
        let mut a = Assembler::new(Kind::Video, AssemblerConfig::video());
        let d = shards(0, 10_000, 0, 0);
        feed(&mut a, &d[0], 0);
        assert!(a.poll(149_999).is_empty());
        assert_eq!(a.poll(150_000), vec![Event::Lost { first: 0, last: 0, speculative: false }]);
    }

    #[test]
    fn duplicates_and_late_shards_are_harmless() {
        let mut a = Assembler::new(Kind::Video, AssemblerConfig::video());
        let d = shards(0, 3000, 50, 0);
        let mut ev = Vec::new();
        for s in d.iter().chain(d.iter()) {
            ev.extend(feed(&mut a, s, 1));
        }
        assert_eq!(frames(&ev), vec![0]);
        assert!(a.take_interval().late >= 3);
    }

    #[test]
    fn inconsistent_shards_are_dropped() {
        let mut a = Assembler::new(Kind::Video, AssemblerConfig::video());
        let d = shards(0, 3000, 0, 0);
        feed(&mut a, &d[0], 1);
        let mut evil = shards(0, 3000, 0, flags::KEYFRAME)[1].clone();
        let (h, p) = ShardHeader::parse(&evil).unwrap();
        assert!(a.push(&h, p, 1).is_empty());
        evil = shards(0, 3000, 50, 0)[1].clone(); // different parity count
        let (h, p) = ShardHeader::parse(&evil).unwrap();
        a.push(&h, p, 1);
        assert_eq!(a.take_interval().malformed, 2);
    }

    #[test]
    fn audio_is_delivered_unordered_and_ignores_old_ids() {
        let mut a = Assembler::new(Kind::Audio, AssemblerConfig::audio());
        let pkt = |id: u32| {
            let meta = FrameMeta { kind: Kind::Audio, flags: 0, frame_id: id, capture_us: 0 };
            encode_frame(&[id as u8; 120], meta, &FecParams::new(120, 100)).unwrap()
        };
        let mut ev = Vec::new();
        ev.extend(feed(&mut a, &pkt(2)[0], 1));
        ev.extend(feed(&mut a, &pkt(1)[1], 1)); // parity only is enough for 1 data shard
        assert_eq!(frames(&ev), vec![2, 1]);
        let Event::Frame(f) = &ev[1] else { panic!() };
        assert_eq!(f.data, vec![1u8; 120]);
    }

    #[test]
    fn memory_stays_bounded_under_junk() {
        let mut cfg = AssemblerConfig::video();
        cfg.max_buffered_bytes = 20_000;
        cfg.speculative = false;
        let mut a = Assembler::new(Kind::Video, cfg);
        // one huge frame whose shards never finish it
        let d = shards(0, 100_000, 0, 0);
        for s in &d[..60] {
            feed(&mut a, s, 1);
        }
        assert!(a.buffered <= 20_000);
        assert_eq!(a.take_interval().malformed, 40);
        // frames that each send one shard and stop never pile up
        let mut a = Assembler::new(Kind::Video, AssemblerConfig::video());
        for id in 0..200 {
            feed(&mut a, &shards(id, 5000, 0, 0)[0], id as u64);
        }
        assert!(a.frames.len() <= 3, "{}", a.frames.len());
    }

    #[test]
    fn gate_drops_broken_frames_until_recovery() {
        let mk = |id, fl| CompleteFrame {
            kind: Kind::Video,
            frame_id: id,
            flags: fl,
            capture_us: 0,
            data: vec![],
            recovered: false,
            first_shard_us: 0,
            complete_us: 0,
        };
        let mut g = RecoveryGate::default();
        assert!(g.admit(&mk(1, 0)));
        g.on_lost(5, 6);
        assert!(g.admit(&mk(4, 0)), "older than the loss is fine");
        assert!(!g.admit(&mk(7, 0)));
        assert!(!g.admit(&mk(6, flags::KEYFRAME)), "inside the lost range");
        assert!(g.admit(&mk(8, flags::RECOVERY)));
        assert!(g.admit(&mk(9, 0)));
        g.on_lost(12, 12);
        assert!(g.admit(&mk(20, flags::KEYFRAME)));
    }
}
