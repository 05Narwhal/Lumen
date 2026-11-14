//! Time left for an import (round 3): bytes per second, smoothed so one slow file does not
//! swing the estimate, and nothing said until there is enough to go on.

use std::time::{Duration, Instant};

/// Too early to say before this much time and this many files.
const WARMUP: Duration = Duration::from_secs(5);
const WARMUP_FILES: u64 = 3;
/// Rate samples cover at least this long, so a burst of small files is one sample.
const WINDOW: Duration = Duration::from_secs(2);
/// Weight of the newest sample.
const ALPHA: f64 = 0.2;
/// Slower than this is a stall, not a rate to divide by.
const STALL_BYTES_PER_S: f64 = 1_000.0;

#[derive(Debug, Clone)]
pub struct Eta {
    started: Instant,
    window_start: Instant,
    window_bytes: u64,
    files: u64,
    rate: Option<f64>,
}

impl Eta {
    pub fn new(now: Instant) -> Self {
        Self { started: now, window_start: now, window_bytes: 0, files: 0, rate: None }
    }

    /// A file of `bytes` has been brought in.
    pub fn record(&mut self, bytes: u64, now: Instant) {
        self.files += 1;
        self.window_bytes += bytes;
        let span = now.saturating_duration_since(self.window_start);
        if span >= WINDOW {
            let sample = self.window_bytes as f64 / span.as_secs_f64();
            self.rate = Some(match self.rate {
                Some(r) => ALPHA * sample + (1.0 - ALPHA) * r,
                None => sample,
            });
            self.window_start = now;
            self.window_bytes = 0;
        }
    }

    /// Milliseconds to bring in `left` more bytes, when it can be said.
    pub fn remaining_ms(&self, left: u64, now: Instant) -> Option<u64> {
        if now.saturating_duration_since(self.started) < WARMUP || self.files < WARMUP_FILES {
            return None;
        }
        let rate = self.rate.filter(|r| *r >= STALL_BYTES_PER_S)?;
        Some((left as f64 / rate * 1000.0).round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1_000_000;

    #[test]
    fn a_steady_rate_gives_a_steady_estimate() {
        let t0 = Instant::now();
        let mut eta = Eta::new(t0);
        // 10 MB/s: a 5 MB file every half second for 20 s.
        for i in 1..=40 {
            eta.record(5 * MB, t0 + Duration::from_millis(500 * i));
        }
        let ms = eta.remaining_ms(800 * MB, t0 + Duration::from_secs(20)).unwrap();
        assert!((76_000..=84_000).contains(&ms), "{ms}");
    }

    #[test]
    fn nothing_is_said_too_early() {
        let t0 = Instant::now();
        let mut eta = Eta::new(t0);
        eta.record(50 * MB, t0 + Duration::from_secs(1));
        assert_eq!(eta.remaining_ms(100 * MB, t0 + Duration::from_secs(1)), None, "one second, one file");
        for i in 2..=4 {
            eta.record(50 * MB, t0 + Duration::from_secs(i));
        }
        assert_eq!(eta.remaining_ms(100 * MB, t0 + Duration::from_secs(4)), None, "not five seconds yet");
    }

    #[test]
    fn a_stall_is_not_infinity() {
        let t0 = Instant::now();
        let mut eta = Eta::new(t0);
        for i in 1..=4 {
            eta.record(10, t0 + Duration::from_secs(3 * i));
        }
        assert_eq!(eta.remaining_ms(100 * MB, t0 + Duration::from_secs(12)), None);
    }
}
