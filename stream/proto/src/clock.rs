//! Time measurements on the client.
//!
//! `QueueDelay` needs no clock sync: it compares each frame's
//! (first-shard arrival − capture) with the smallest one seen in the last few
//! seconds. The unknown clock offset is in both and cancels out; what is left
//! is how much longer frames waited than the fastest one, which grows when a
//! queue builds on the path. The first shard is used, not the last, so a big
//! keyframe taking longer to send is not mistaken for a queue; and the
//! interval's minimum is used, not its mean, so one late frame is not either.
//!
//! `ClockSync` estimates the server clock from ping/pong (NTP style, keeping
//! the sample with the lowest RTT), for absolute latency in the stats.

use std::collections::VecDeque;

pub struct QueueDelay {
    window_us: u64,
    samples: VecDeque<(u64, i64)>,
    interval_min: Option<i64>,
}

impl QueueDelay {
    pub fn new(window_us: u64) -> Self {
        QueueDelay { window_us, samples: VecDeque::new(), interval_min: None }
    }

    pub fn add(&mut self, arrival_us: u64, capture_server_us: u64) {
        let d = arrival_us as i64 - capture_server_us as i64;
        self.samples.push_back((arrival_us, d));
        while self.samples.front().is_some_and(|(t, _)| arrival_us.saturating_sub(*t) > self.window_us) {
            self.samples.pop_front();
        }
        self.interval_min = Some(self.interval_min.map_or(d, |m| m.min(d)));
    }

    /// Queue delay since the last call (the interval's fastest frame against
    /// the window's fastest), in microseconds.
    pub fn take_interval(&mut self) -> u32 {
        let Some(min) = self.samples.iter().map(|(_, d)| *d).min() else { return 0 };
        let Some(i) = self.interval_min.take() else { return 0 };
        (i - min).clamp(0, u32::MAX as i64) as u32
    }
}

#[derive(Default)]
pub struct ClockSync {
    samples: VecDeque<(u64, i64)>,
}

impl ClockSync {
    /// `server_us` was read by the server between `sent_us` and `recv_us` (client clock).
    pub fn add(&mut self, sent_us: u64, server_us: u64, recv_us: u64) {
        if recv_us < sent_us {
            return;
        }
        let rtt = recv_us - sent_us;
        let offset = server_us as i64 - (sent_us + rtt / 2) as i64;
        self.samples.push_back((rtt, offset));
        if self.samples.len() > 16 {
            self.samples.pop_front();
        }
    }

    pub fn rtt_us(&self) -> Option<u64> {
        self.samples.iter().map(|(r, _)| *r).min()
    }

    /// server clock − client clock.
    pub fn offset_us(&self) -> Option<i64> {
        self.samples.iter().min_by_key(|(r, _)| *r).map(|(_, o)| *o)
    }

    pub fn to_client(&self, server_us: u64) -> Option<u64> {
        self.offset_us().map(|o| (server_us as i64 - o).max(0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_delay_ignores_the_clock_offset() {
        let mut q = QueueDelay::new(5_000_000);
        // server clock is 1 hour ahead; base path delay 20 ms
        let off = 3_600_000_000u64;
        for i in 0..10u64 {
            q.add(i * 16_000 + 20_000, i * 16_000 + off);
        }
        assert_eq!(q.take_interval(), 0);
        // queue builds: frames now take 30 ms more
        for i in 10..20u64 {
            q.add(i * 16_000 + 50_000, i * 16_000 + off);
        }
        assert_eq!(q.take_interval(), 30_000);
        assert_eq!(q.take_interval(), 0, "nothing new");
        // one slow frame among fast ones is not a queue
        q.add(400_000 + 20_000, 400_000 + off);
        q.add(416_000 + 90_000, 416_000 + off);
        assert_eq!(q.take_interval(), 0);
    }

    #[test]
    fn old_samples_leave_the_window() {
        let mut q = QueueDelay::new(1_000_000);
        q.add(0, 0); // very fast sample
        q.add(2_000_000, 2_000_000 - 10_000);
        q.take_interval();
        q.add(2_100_000, 2_100_000 - 10_000);
        assert_eq!(q.take_interval(), 0, "the 0 µs sample has expired");
    }

    #[test]
    fn clock_sync_keeps_the_fastest_sample() {
        let mut c = ClockSync::default();
        c.add(1_000, 501_000 + 50, 1_100); // rtt 100, offset 500_000
        c.add(2_000, 9_999_999, 12_000); // rtt 10_000, junk offset
        assert_eq!(c.rtt_us(), Some(100));
        assert_eq!(c.offset_us(), Some(500_000));
        assert_eq!(c.to_client(600_000), Some(100_000));
        c.add(10, 0, 5); // reversed times are ignored
        assert_eq!(c.rtt_us(), Some(100));
    }
}
