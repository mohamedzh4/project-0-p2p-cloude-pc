//! DCL Stream protocol core: everything that has no OS dependency, so it can
//! be tested anywhere and fuzzed. See docs/streaming-protocol-design.md.

pub mod auth;
pub mod clock;
pub mod fec;
pub mod msg;
pub mod pacing;
pub mod rate;
pub mod reassembly;
pub mod wire;

#[cfg(feature = "net")]
pub mod net;

/// Microseconds on a monotonic clock local to this process.
pub fn now_us() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_micros() as u64
}
