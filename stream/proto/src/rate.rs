//! Rate control: turns the client's network reports into a bitrate and an FEC
//! percentage. Sunshine keeps both fixed for the whole session and only logs
//! these reports; this is the part we do differently.
//!
//! Two kinds of loss get two answers:
//! - **random loss** (Wi-Fi, a lossy link) with no queue growth: keep the
//!   bitrate, raise FEC so the frames still arrive;
//! - **congestion** (queue delay growing, or loss beyond what FEC can carry):
//!   cut the bitrate fast, then grow it back slowly.

use crate::msg::NetworkReport;

#[derive(Clone, Copy, Debug)]
pub struct RateConfig {
    pub min_kbps: u32,
    pub max_kbps: u32,
    pub start_kbps: u32,
    pub min_fec_pct: u8,
    pub max_fec_pct: u8,
    /// Queue delay that counts as congestion.
    pub congested_queue_us: u32,
    /// Queue delay under which we may grow.
    pub calm_queue_us: u32,
}

impl RateConfig {
    pub fn new(max_kbps: u32) -> Self {
        RateConfig {
            min_kbps: 1_500.min(max_kbps),
            max_kbps,
            start_kbps: (max_kbps * 3 / 4).max(1_500.min(max_kbps)),
            min_fec_pct: 5,
            max_fec_pct: 40,
            congested_queue_us: 20_000,
            calm_queue_us: 5_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// What goes to the encoder.
    pub video_kbps: u32,
    pub fec_pct: u8,
    /// Video plus parity: what goes on the wire.
    pub total_kbps: u32,
}

pub struct RateController {
    cfg: RateConfig,
    total_kbps: f64,
    /// Slow average: decides congestion.
    loss_ewma: f64,
    /// Rises fast, falls slowly: decides FEC, so a lossy patch is covered at once.
    loss_peak: f64,
    fec_pct: u8,
    last_decrease_us: Option<u64>,
    last_increase_us: u64,
}

impl RateController {
    pub fn new(cfg: RateConfig) -> Self {
        RateController {
            cfg,
            total_kbps: cfg.start_kbps as f64,
            loss_ewma: 0.0,
            // matches the starting 20 % FEC until reports say otherwise
            loss_peak: (20.0 - cfg.min_fec_pct as f64).max(0.0) / 300.0,
            fec_pct: 20.clamp(cfg.min_fec_pct, cfg.max_fec_pct),
            last_decrease_us: None,
            last_increase_us: 0,
        }
    }

    pub fn current(&self) -> Decision {
        let total = self.total_kbps.round() as u32;
        Decision {
            video_kbps: (self.total_kbps * 100.0 / (100.0 + self.fec_pct as f64)).round() as u32,
            fec_pct: self.fec_pct,
            total_kbps: total,
        }
    }

    pub fn on_report(&mut self, r: &NetworkReport, now_us: u64) -> Decision {
        let seen = r.shards_received as u64 + r.shards_missing as u64;
        let loss = if seen == 0 { 0.0 } else { r.shards_missing as f64 / seen as f64 };
        // About half a second of memory at one report per 50 ms.
        self.loss_ewma = 0.9 * self.loss_ewma + 0.1 * loss;
        let a = if loss > self.loss_peak { 0.3 } else { 0.02 };
        self.loss_peak = (1.0 - a) * self.loss_peak + a * loss;

        // Parity: three times the smoothed loss plus a margin. A P-frame is
        // only ~10 shards, and loss clusters within one, so the average alone
        // is far too little.
        let want = (self.loss_peak * 300.0 + self.cfg.min_fec_pct as f64 - 1e-3).ceil();
        self.fec_pct = (want as u8).clamp(self.cfg.min_fec_pct, self.cfg.max_fec_pct);

        // Smoothed loss, not this interval's: 50 ms holds only a few dozen
        // shards, so 5 % random loss often looks like 10 % in one interval.
        let congested = r.queue_delay_us >= self.cfg.congested_queue_us
            || self.loss_ewma > 0.10
            || (r.frames_lost > 0 && self.loss_ewma * 100.0 > self.fec_pct as f64);
        let since_decrease = self.last_decrease_us.map_or(u64::MAX, |t| now_us.saturating_sub(t));

        if congested {
            // Wait about an RTT plus a frame before cutting again, so one queue is not counted twice.
            if since_decrease >= 500_000 {
                let factor = if r.queue_delay_us >= self.cfg.congested_queue_us * 3 || loss > 0.25 { 0.6 } else { 0.8 };
                self.total_kbps *= factor;
                self.last_decrease_us = Some(now_us);
            }
        } else if r.queue_delay_us <= self.cfg.calm_queue_us
            && loss < 0.02
            && since_decrease >= 2_000_000
            && now_us.saturating_sub(self.last_increase_us) >= 1_000_000
        {
            self.total_kbps += self.cfg.max_kbps as f64 * 0.05;
            self.last_increase_us = now_us;
        }
        self.total_kbps = self.total_kbps.clamp(self.cfg.min_kbps as f64, self.cfg.max_kbps as f64);
        self.current()
    }
}

/// Keyframes cost 5-10x a normal frame. On a bad network, asking for one per
/// loss turns into a storm; this allows at most one per `min_gap_us` and keeps
/// one pending request.
pub struct KeyframeLimiter {
    min_gap_us: u64,
    last_us: Option<u64>,
    pending: bool,
}

impl KeyframeLimiter {
    pub fn new(min_gap_us: u64) -> Self {
        KeyframeLimiter { min_gap_us, last_us: None, pending: false }
    }

    /// A request arrived. True means: make a keyframe now.
    pub fn request(&mut self, now_us: u64) -> bool {
        self.pending = true;
        self.poll(now_us)
    }

    /// True when a held request may go out now.
    pub fn poll(&mut self, now_us: u64) -> bool {
        if !self.pending {
            return false;
        }
        if self.last_us.is_some_and(|t| now_us.saturating_sub(t) < self.min_gap_us) {
            return false;
        }
        self.pending = false;
        self.last_us = Some(now_us);
        true
    }

    /// The encoder made a keyframe on its own (start, resolution change).
    pub fn sent(&mut self, now_us: u64) {
        self.pending = false;
        self.last_us = Some(now_us);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(received: u32, missing: u32, queue_us: u32, lost: u32) -> NetworkReport {
        NetworkReport { interval_ms: 50, shards_received: received, shards_missing: missing, frames_lost: lost, queue_delay_us: queue_us, ..Default::default() }
    }

    #[test]
    fn video_plus_fec_fits_the_total() {
        let c = RateController::new(RateConfig::new(40_000));
        let d = c.current();
        assert_eq!(d.total_kbps, 30_000);
        assert_eq!(d.fec_pct, 20);
        assert_eq!(d.video_kbps, 25_000);
    }

    #[test]
    fn random_loss_raises_fec_but_keeps_bitrate() {
        let mut c = RateController::new(RateConfig::new(40_000));
        let mut d = c.current();
        for i in 0..40 {
            d = c.on_report(&rep(950, 50, 1_000, 0), i * 50_000); // 5 % loss, no queue
        }
        assert_eq!(d.total_kbps, 30_000);
        assert!(d.fec_pct >= 15, "{d:?}");
    }

    #[test]
    fn queue_growth_cuts_bitrate_then_it_recovers_slowly() {
        let mut c = RateController::new(RateConfig::new(40_000));
        // one unlucky interval at 5 % average loss does not count as congestion
        assert_eq!(c.on_report(&rep(85, 15, 0, 1), 0).total_kbps, 30_000);
        let mut c = RateController::new(RateConfig::new(40_000));
        let d = c.on_report(&rep(1000, 0, 30_000, 0), 0);
        assert_eq!(d.total_kbps, 24_000);
        // a second report 50 ms later about the same queue does not cut again
        assert_eq!(c.on_report(&rep(1000, 0, 30_000, 0), 50_000).total_kbps, 24_000);
        // heavy queue: deeper cut
        assert_eq!(c.on_report(&rep(1000, 0, 70_000, 0), 600_000).total_kbps, 14_400);
        // calm: nothing for 2 s, then +5 % of max per second
        assert_eq!(c.on_report(&rep(1000, 0, 0, 0), 1_000_000).total_kbps, 14_400);
        assert_eq!(c.on_report(&rep(1000, 0, 0, 0), 2_700_000).total_kbps, 16_400);
        assert_eq!(c.on_report(&rep(1000, 0, 0, 0), 3_000_000).total_kbps, 16_400);
        assert_eq!(c.on_report(&rep(1000, 0, 0, 0), 3_700_000).total_kbps, 18_400);
    }

    #[test]
    fn fec_rises_fast_and_falls_slowly() {
        let mut c = RateController::new(RateConfig::new(40_000));
        assert_eq!(c.on_report(&rep(1000, 0, 0, 0), 0).fec_pct, 20, "a clean report barely moves it");
        let d = c.on_report(&rep(800, 200, 0, 0), 50_000);
        assert!(d.fec_pct >= 30, "one 20 % interval lifts FEC at once: {d:?}");
        for i in 2..22 {
            c.on_report(&rep(1000, 0, 0, 0), i * 50_000);
        }
        assert!(c.current().fec_pct >= 20, "one clean second does not drop it: {:?}", c.current());
    }

    #[test]
    fn stays_within_limits() {
        let mut c = RateController::new(RateConfig::new(10_000));
        for i in 0..100 {
            c.on_report(&rep(500, 500, 100_000, 5), i * 1_000_000);
        }
        let d = c.current();
        assert_eq!(d.total_kbps, 1_500);
        assert_eq!(d.fec_pct, 40);
        for i in 100..900 {
            c.on_report(&rep(1000, 0, 0, 0), i * 1_000_000);
        }
        assert_eq!(c.current().total_kbps, 10_000);
        assert_eq!(c.current().fec_pct, 5);
    }

    #[test]
    fn keyframes_are_rate_limited() {
        let mut k = KeyframeLimiter::new(200_000);
        assert!(k.request(0));
        assert!(!k.request(50_000));
        assert!(!k.request(100_000));
        assert!(!k.poll(199_999));
        assert!(k.poll(200_000), "the held request goes out once");
        assert!(!k.poll(500_000));
        k.sent(600_000);
        assert!(!k.request(700_000));
        assert!(k.poll(800_000));
    }
}
