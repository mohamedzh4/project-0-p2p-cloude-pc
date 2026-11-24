//! Send pacing. Sunshine paces at a fixed 80 % of 1 Gbit/s; over the internet
//! and through WireGuard that burst overflows home-router queues. We pace at a
//! multiple of the current target rate, so a frame leaves in a fraction of
//! the frame interval but never as one burst.

pub struct Pacer {
    bytes_per_us: f64,
    burst_bytes: f64,
    tokens: f64,
    last_us: u64,
}

impl Pacer {
    /// `gain` = pacing rate / target rate (2.0 sends an average frame in half its interval).
    pub fn new(total_kbps: u32, gain: f64, burst_bytes: usize) -> Self {
        let mut p = Pacer { bytes_per_us: 0.0, burst_bytes: burst_bytes as f64, tokens: burst_bytes as f64, last_us: 0 };
        p.set_rate(total_kbps, gain);
        p
    }

    pub fn set_rate(&mut self, total_kbps: u32, gain: f64) {
        // kbit/s -> bytes/µs: * 1000 / 8 / 1e6
        self.bytes_per_us = (total_kbps.max(100) as f64 * gain) / 8_000.0;
    }

    /// How long to wait before sending `bytes`. The caller sleeps, then sends.
    pub fn delay_us(&mut self, bytes: usize, now_us: u64) -> u64 {
        if now_us > self.last_us {
            self.tokens = (self.tokens + (now_us - self.last_us) as f64 * self.bytes_per_us).min(self.burst_bytes);
            self.last_us = now_us;
        }
        self.tokens -= bytes as f64;
        if self.tokens >= 0.0 {
            return 0;
        }
        let wait = (-self.tokens / self.bytes_per_us).ceil() as u64;
        // The send happens at now + wait; the bucket is empty then.
        self.tokens = 0.0;
        self.last_us = now_us + wait;
        wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreads_a_frame_over_time() {
        // 8 Mbit/s * gain 1 = 1000 bytes/ms
        let mut p = Pacer::new(8_000, 1.0, 2_000);
        let mut t = 0;
        let mut waits = vec![];
        for _ in 0..6 {
            let w = p.delay_us(1_000, t);
            t += w;
            waits.push(w);
        }
        assert_eq!(waits, vec![0, 0, 1_000, 1_000, 1_000, 1_000]);
    }

    #[test]
    fn idle_time_refills_only_up_to_the_burst() {
        let mut p = Pacer::new(8_000, 1.0, 2_000);
        p.delay_us(2_000, 0);
        assert_eq!(p.delay_us(1_000, 10_000_000), 0);
        assert_eq!(p.delay_us(1_000, 10_000_000), 0);
        assert_eq!(p.delay_us(1_000, 10_000_000), 1_000);
    }
}
