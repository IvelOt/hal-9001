use std::collections::VecDeque;
use std::time::{Duration, Instant};

const WINDOW_HORIZON: Duration = Duration::from_secs(3);
const EWMA_ALPHA: f64 = 0.35;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateSample {
    pub speed_mbps: f64,
    pub avg_mbps: f64,
    pub eta_secs: u64,
}

/// Computes physical transfer throughput (MiB/s) and ETA based strictly on
/// bytes confirmed persisted on the destination device over a sliding time
/// window with EWMA smoothing.
pub struct RateMeter {
    started_at: Instant,
    samples: VecDeque<(Instant, u64)>,
    ewma_bps: Option<f64>,
}

impl RateMeter {
    pub fn new(now: Instant) -> Self {
        let mut samples = VecDeque::with_capacity(32);
        samples.push_back((now, 0));
        Self {
            started_at: now,
            samples,
            ewma_bps: None,
        }
    }

    pub fn record(&mut self, now: Instant, persisted_bytes: u64, total_bytes: u64) -> RateSample {
        self.samples.push_back((now, persisted_bytes));

        while self.samples.len() > 2 {
            if let Some(&(front_t, _)) = self.samples.front() {
                if now.saturating_duration_since(front_t) > WINDOW_HORIZON {
                    self.samples.pop_front();
                } else {
                    break;
                }
            }
        }

        let (base_t, base_b) = self.samples.front().copied().unwrap_or((now, 0));
        let dt_win = now
            .saturating_duration_since(base_t)
            .as_secs_f64()
            .max(1e-3);
        let db_win = persisted_bytes.saturating_sub(base_b) as f64;
        let inst_bps = db_win / dt_win;

        let smoothed_bps = match self.ewma_bps {
            Some(prev) if inst_bps > 0.0 => prev * (1.0 - EWMA_ALPHA) + inst_bps * EWMA_ALPHA,
            Some(prev) => prev * (1.0 - EWMA_ALPHA * 0.5),
            None => inst_bps,
        };
        if inst_bps > 0.0 || self.ewma_bps.is_some() {
            self.ewma_bps = Some(smoothed_bps);
        }

        let dt_total = now
            .saturating_duration_since(self.started_at)
            .as_secs_f64()
            .max(1e-3);
        let avg_bps = (persisted_bytes as f64) / dt_total;

        let effective_bps = if smoothed_bps > 1.0 {
            smoothed_bps
        } else {
            avg_bps
        };
        let remaining = total_bytes.saturating_sub(persisted_bytes) as f64;
        let eta_secs = if effective_bps > 1.0 && remaining > 0.0 {
            (remaining / effective_bps).round() as u64
        } else {
            0
        };

        RateSample {
            speed_mbps: smoothed_bps / 1_048_576.0,
            avg_mbps: avg_bps / 1_048_576.0,
            eta_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_meter_computes_accurate_physical_speed_and_eta() {
        let t0 = Instant::now();
        let mut meter = RateMeter::new(t0);
        let total = 100 * 1_048_576; // 100 MiB

        // After 1 second, 20 MiB persisted -> ~20 MiB/s, 80 MiB left -> ~4s ETA
        let s1 = meter.record(t0 + Duration::from_secs(1), 20 * 1_048_576, total);
        assert!(
            (s1.speed_mbps - 20.0).abs() < 0.05,
            "expected ~20 MB/s, got {}",
            s1.speed_mbps
        );
        assert_eq!(s1.eta_secs, 4);

        // After 2 seconds, 40 MiB persisted -> still ~20 MiB/s, 60 MiB left -> ~3s ETA
        let s2 = meter.record(t0 + Duration::from_secs(2), 40 * 1_048_576, total);
        assert!(
            (s2.speed_mbps - 20.0).abs() < 0.05,
            "expected ~20 MB/s, got {}",
            s2.speed_mbps
        );
        assert_eq!(s2.eta_secs, 3);

        // At 100 MiB persisted, ETA is 0
        let s_done = meter.record(t0 + Duration::from_secs(5), total, total);
        assert_eq!(s_done.eta_secs, 0);
    }

    #[test]
    fn rate_meter_sliding_window_adapts_when_write_speed_drops() {
        let t0 = Instant::now();
        let mut meter = RateMeter::new(t0);
        let total = 200 * 1_048_576;

        // Fast initial burst: 50 MiB in 1s
        let _ = meter.record(t0 + Duration::from_secs(1), 50 * 1_048_576, total);

        // Then slow USB flash media: 10 MiB/s for the next 5 seconds (t=2..6)
        for sec in 2..=6 {
            let persisted = (50 + (sec - 1) * 10) * 1_048_576;
            let _ = meter.record(t0 + Duration::from_secs(sec), persisted, total);
        }
        let s_late = meter.record(t0 + Duration::from_secs(7), 110 * 1_048_576, total);
        // Should converge near 10 MiB/s rather than staying near the initial 50 MiB/s
        assert!(
            s_late.speed_mbps < 15.0 && s_late.speed_mbps > 8.0,
            "expected sliding window to adapt to ~10 MB/s, got {}",
            s_late.speed_mbps
        );
    }
}
