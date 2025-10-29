#![cfg(target_os = "windows")]

use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum MetricEvent {
    Frame {
        encode_latency: Duration,
    },
    CaptureError,
}

#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub fps: f64,
    pub avg_encode_ms: f64,
    pub max_encode_ms: f64,
    pub frames: u64,
    pub errors: u64,
    pub window: Duration,
}

#[derive(Debug)]
pub struct MetricsCollector {
    interval: Duration,
    last_tick: Instant,
    frame_counter: u64,
    encode_total: Duration,
    encode_max: Duration,
    error_counter: u64,
}

impl MetricsCollector {
    pub fn new(interval: Duration) -> Self {
        let clamped = if interval.is_zero() {
            Duration::from_millis(100)
        } else {
            interval
        };
        Self {
            interval: clamped,
            last_tick: Instant::now(),
            frame_counter: 0,
            encode_total: Duration::ZERO,
            encode_max: Duration::ZERO,
            error_counter: 0,
        }
    }

    pub fn record_frame(&mut self, encode_latency: Duration) -> Option<MetricsSnapshot> {
        self.frame_counter = self.frame_counter.saturating_add(1);
        self.encode_total += encode_latency;
        if encode_latency > self.encode_max {
            self.encode_max = encode_latency;
        }

        let elapsed = self.last_tick.elapsed();
        if elapsed >= self.interval && elapsed > Duration::ZERO {
            let frames = self.frame_counter.max(1);
            let fps = self.frame_counter as f64 / elapsed.as_secs_f64().max(f64::EPSILON);
            let avg_encode_ms = (self.encode_total.as_secs_f64() * 1_000.0) / frames as f64;
            let max_encode_ms = self.encode_max.as_secs_f64() * 1_000.0;
            let snapshot = MetricsSnapshot {
                fps,
                avg_encode_ms,
                max_encode_ms,
                frames: self.frame_counter,
                errors: self.error_counter,
                window: elapsed,
            };

            self.reset_window();
            return Some(snapshot);
        }

        None
    }

    pub fn record_error(&mut self) {
        self.error_counter = self.error_counter.saturating_add(1);
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    fn reset_window(&mut self) {
        self.last_tick = Instant::now();
        self.frame_counter = 0;
        self.encode_total = Duration::ZERO;
        self.encode_max = Duration::ZERO;
        self.error_counter = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_snapshot_generated() {
        let mut collector = MetricsCollector::new(Duration::from_millis(10));
        collector.record_frame(Duration::from_millis(5));
        std::thread::sleep(Duration::from_millis(20));
        let snapshot = collector.record_frame(Duration::from_millis(5)).expect("snapshot");
        assert!(snapshot.fps > 0.0);
        assert_eq!(snapshot.errors, 0);
    }

    #[test]
    fn metrics_error_count() {
        let mut collector = MetricsCollector::new(Duration::from_millis(10));
        collector.record_error();
        std::thread::sleep(Duration::from_millis(20));
        let snapshot = collector.record_frame(Duration::from_millis(5)).expect("snapshot");
        assert_eq!(snapshot.errors, 1);
    }
}
