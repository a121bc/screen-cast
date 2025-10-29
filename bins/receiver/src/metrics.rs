use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub fps: f64,
    pub avg_latency_ms: f64,
    pub max_latency_ms: f64,
    pub avg_queue_ms: f64,
    pub max_queue_ms: f64,
    pub avg_decode_ms: f64,
    pub max_decode_ms: f64,
    pub frames: u64,
    pub dropped: u64,
    pub queue_depth: usize,
    pub window: Duration,
}

#[derive(Debug)]
pub enum MetricEvent {
    FramePresented {
        latency: Duration,
        queue_latency: Duration,
        decode_latency: Duration,
        queue_depth: usize,
        dropped_total: u64,
    },
    FrameDropped {
        count: u64,
    },
}

#[derive(Debug)]
pub struct MetricsCollector {
    interval: Duration,
    last_tick: Instant,
    frames: u64,
    dropped: u64,
    latency_total: Duration,
    latency_max: Duration,
    queue_total: Duration,
    queue_max: Duration,
    decode_total: Duration,
    decode_max: Duration,
    last_queue_depth: usize,
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
            frames: 0,
            dropped: 0,
            latency_total: Duration::ZERO,
            latency_max: Duration::ZERO,
            queue_total: Duration::ZERO,
            queue_max: Duration::ZERO,
            decode_total: Duration::ZERO,
            decode_max: Duration::ZERO,
            last_queue_depth: 0,
        }
    }

    pub fn record_presented(
        &mut self,
        latency: Duration,
        queue_latency: Duration,
        decode_latency: Duration,
        queue_depth: usize,
        dropped_total: u64,
    ) -> Option<MetricsSnapshot> {
        self.frames = self.frames.saturating_add(1);
        self.latency_total += latency;
        if latency > self.latency_max {
            self.latency_max = latency;
        }
        self.queue_total += queue_latency;
        if queue_latency > self.queue_max {
            self.queue_max = queue_latency;
        }
        self.decode_total += decode_latency;
        if decode_latency > self.decode_max {
            self.decode_max = decode_latency;
        }
        self.last_queue_depth = queue_depth;
        self.dropped = dropped_total;

        self.snapshot()
    }

    pub fn record_dropped(&mut self, count: u64) -> Option<MetricsSnapshot> {
        self.dropped = self.dropped.saturating_add(count);
        self.snapshot()
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    fn snapshot(&mut self) -> Option<MetricsSnapshot> {
        let elapsed = self.last_tick.elapsed();
        if elapsed < self.interval || elapsed.is_zero() {
            return None;
        }

        let frames = self.frames.max(1);
        let fps = self.frames as f64 / elapsed.as_secs_f64().max(f64::EPSILON);
        let avg_latency_ms = duration_ms(self.latency_total) / frames as f64;
        let avg_queue_ms = duration_ms(self.queue_total) / frames as f64;
        let avg_decode_ms = duration_ms(self.decode_total) / frames as f64;
        let snapshot = MetricsSnapshot {
            fps,
            avg_latency_ms,
            max_latency_ms: duration_ms(self.latency_max),
            avg_queue_ms,
            max_queue_ms: duration_ms(self.queue_max),
            avg_decode_ms,
            max_decode_ms: duration_ms(self.decode_max),
            frames: self.frames,
            dropped: self.dropped,
            queue_depth: self.last_queue_depth,
            window: elapsed,
        };
        self.reset_window();
        Some(snapshot)
    }

    fn reset_window(&mut self) {
        self.last_tick = Instant::now();
        self.frames = 0;
        self.latency_total = Duration::ZERO;
        self.latency_max = Duration::ZERO;
        self.queue_total = Duration::ZERO;
        self.queue_max = Duration::ZERO;
        self.decode_total = Duration::ZERO;
        self.decode_max = Duration::ZERO;
        self.dropped = 0;
        self.last_queue_depth = 0;
    }
}

#[derive(Clone, Debug)]
pub struct MetricsHandle {
    inner: Arc<broadcast::Sender<MetricsSnapshot>>,
}

impl MetricsHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<MetricsSnapshot> {
        self.inner.subscribe()
    }
}

#[derive(Debug, Clone)]
pub struct MetricsHub {
    sender: Arc<broadcast::Sender<MetricsSnapshot>>,
}

impl MetricsHub {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self {
            sender: Arc::new(sender),
        }
    }

    pub fn handle(&self) -> MetricsHandle {
        MetricsHandle {
            inner: Arc::clone(&self.sender),
        }
    }

    pub fn publish(&self, snapshot: MetricsSnapshot) {
        let _ = self.sender.send(snapshot);
    }
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
