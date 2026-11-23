//! Reconnect delay: exponential from 1 s to 60 s with full jitter, reset after a session that
//! delivered data. Pure: the random draw is an argument.

use std::time::Duration;

const BASE_S: f64 = 1.0;
const CAP_S: f64 = 60.0;
/// Never reconnect faster than this, even on a zero draw.
const FLOOR_S: f64 = 0.5;

#[derive(Clone, Debug, Default)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    /// The next delay for a uniform draw `unit` in `[0, 1)`.
    pub fn next(&mut self, unit: f64) -> Duration {
        let ceiling = (BASE_S * 2f64.powi(self.attempt.min(16) as i32)).min(CAP_S);
        self.attempt = self.attempt.saturating_add(1);
        Duration::from_secs_f64((ceiling * unit.clamp(0.0, 1.0)).max(FLOOR_S))
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_caps_and_resets() {
        let mut b = Backoff::default();
        let ceilings: Vec<f64> = (0..9).map(|_| b.next(0.999_999).as_secs_f64()).collect();
        assert!((ceilings[0] - 1.0).abs() < 0.01 && (ceilings[3] - 8.0).abs() < 0.01);
        assert!(ceilings[8] <= 60.0 && ceilings[8] > 59.9);
        assert_eq!(b.next(0.0), Duration::from_millis(500));
        b.reset();
        assert!(b.next(0.999).as_secs_f64() < 1.0);
    }
}
