use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use codec::Decoder;
use network::{NetworkReceiver, ReceivedFrame};
use shared::{AppError, AppResult};
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::task::{JoinError, JoinSet};
use tracing::{info, instrument, warn};

use crate::config::{ReceiverConfig, RenderSettings};
use crate::metrics::{MetricEvent, MetricsCollector, MetricsHub};
use crate::render::{run_renderer, RenderFrame};

const NETWORK_BACKOFF: Duration = Duration::from_millis(15);
const INVALID_FRAME_GRACE: Duration = Duration::from_millis(5);

pub struct ReceiverPipeline {
    config: ReceiverConfig,
    metrics: MetricsHub,
}

#[derive(Debug, Clone, Default)]
pub struct PipelineReport {
    pub frames_received: u64,
    pub frames_decoded: u64,
    pub frames_dispatched: u64,
    pub frames_rendered: u64,
    pub frames_dropped: u64,
    pub avg_latency_ms: f64,
    pub max_latency_ms: f64,
}

#[derive(Debug, Default)]
pub(crate) struct PipelineCounters {
    frames_received: AtomicU64,
    frames_decoded: AtomicU64,
    frames_dispatched: AtomicU64,
    frames_rendered: AtomicU64,
    frames_dropped: AtomicU64,
    latency_total_micros: AtomicU64,
    latency_max_micros: AtomicU64,
}

impl PipelineCounters {
    fn record_received(&self) {
        self.frames_received.fetch_add(1, Ordering::Relaxed);
    }

    fn record_decoded(&self) {
        self.frames_decoded.fetch_add(1, Ordering::Relaxed);
    }

    fn record_dispatched(&self) {
        self.frames_dispatched.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_rendered(&self, latency: Duration) {
        self.frames_rendered.fetch_add(1, Ordering::Relaxed);
        let micros = latency.as_micros() as u64;
        self.latency_total_micros.fetch_add(micros, Ordering::Relaxed);
        let mut current = self.latency_max_micros.load(Ordering::Relaxed);
        while micros > current {
            match self
                .latency_max_micros
                .compare_exchange(current, micros, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    fn record_dropped(&self) {
        self.frames_dropped.fetch_add(1, Ordering::Relaxed);
    }

    fn record_dropped_many(&self, count: u64) {
        if count > 0 {
            self.frames_dropped.fetch_add(count, Ordering::Relaxed);
        }
    }

    fn frames_received(&self) -> u64 {
        self.frames_received.load(Ordering::Relaxed)
    }

    fn frames_decoded(&self) -> u64 {
        self.frames_decoded.load(Ordering::Relaxed)
    }

    fn frames_dispatched(&self) -> u64 {
        self.frames_dispatched.load(Ordering::Relaxed)
    }

    fn frames_rendered(&self) -> u64 {
        self.frames_rendered.load(Ordering::Relaxed)
    }

    fn frames_dropped(&self) -> u64 {
        self.frames_dropped.load(Ordering::Relaxed)
    }

    fn avg_latency_ms(&self) -> f64 {
        let rendered = self.frames_rendered();
        if rendered == 0 {
            return 0.0;
        }
        let total = self.latency_total_micros.load(Ordering::Relaxed) as f64;
        (total / rendered as f64) / 1_000.0
    }

    fn max_latency_ms(&self) -> f64 {
        self.latency_max_micros.load(Ordering::Relaxed) as f64 / 1_000.0
    }
}

impl ReceiverPipeline {
    pub fn new(mut config: ReceiverConfig) -> AppResult<Self> {
        config.validate()?;
        let metrics = MetricsHub::new(config.metrics_channel_capacity());
        Ok(Self { config, metrics })
    }

    pub fn metrics_handle(&self) -> crate::MetricsHandle {
        self.metrics.handle()
    }

    pub async fn run(self) -> AppResult<PipelineReport> {
        let Self { config, metrics } = self;
        let render_stage = Arc::new(RenderStage::new(
            config.pipeline.render_queue,
            config.max_latency(),
        ));
        let counters = Arc::new(PipelineCounters::default());
        let running = Arc::new(AtomicBool::new(true));
        let shutdown = Arc::new(Notify::new());

        let expected_bytes = config.expected_frame_bytes();
        let metrics_interval = config.metrics_interval();

        let (decode_tx, decode_rx) = mpsc::channel::<ReceivedFrame>(config.pipeline.decode_queue);
        let (render_tx, render_rx) = mpsc::unbounded_channel::<RenderFrame>();
        let (metrics_tx, metrics_rx) = mpsc::unbounded_channel::<MetricEvent>();

        let mut join_set = JoinSet::new();

        {
            let receiver = NetworkReceiver::new(config.endpoint_config());
            let decode_tx = decode_tx.clone();
            let counters = Arc::clone(&counters);
            let metrics_tx = metrics_tx.clone();
            let running = Arc::clone(&running);
            let shutdown = Arc::clone(&shutdown);
            join_set.spawn(async move {
                run_network_loop(receiver, decode_tx, counters, metrics_tx, running, shutdown).await
            });
        }

        {
            let render_stage = Arc::clone(&render_stage);
            let counters = Arc::clone(&counters);
            let metrics_tx = metrics_tx.clone();
            let running = Arc::clone(&running);
            let shutdown = Arc::clone(&shutdown);
            let render_settings = config.render.clone();
            join_set.spawn(async move {
                run_decode_loop(
                    decode_rx,
                    render_stage,
                    render_settings,
                    counters,
                    metrics_tx,
                    running,
                    shutdown,
                    expected_bytes,
                )
                .await
            });
        }

        {
            let render_stage = Arc::clone(&render_stage);
            let render_tx = render_tx.clone();
            let counters = Arc::clone(&counters);
            let metrics_tx = metrics_tx.clone();
            let running = Arc::clone(&running);
            let shutdown = Arc::clone(&shutdown);
            let frame_limit = config.pipeline.frame_limit;
            join_set.spawn(async move {
                run_present_loop(
                    render_stage,
                    render_tx,
                    counters,
                    metrics_tx,
                    running,
                    shutdown,
                    frame_limit,
                )
                .await
            });
        }

        {
            let counters = Arc::clone(&counters);
            let metrics_hub = metrics.clone();
            let running = Arc::clone(&running);
            let shutdown = Arc::clone(&shutdown);
            join_set.spawn(async move {
                observe_metrics(metrics_rx, metrics_interval, counters, metrics_hub, running, shutdown).await
            });
        }

        drop(decode_tx);
        drop(render_tx);

        let ui_running = Arc::clone(&running);
        let ui_shutdown = Arc::clone(&shutdown);
        let ui_counters = Arc::clone(&counters);
        let ui_metrics_tx = metrics_tx.clone();
        let ui_settings = config.render.clone();
        let ui_frame_limit = config.pipeline.frame_limit;
        let render_handle = tokio::task::spawn_blocking(move || {
            run_renderer(
                render_rx,
                ui_metrics_tx,
                ui_counters,
                ui_running,
                ui_shutdown,
                ui_settings,
                ui_frame_limit,
            )
        });

        drop(metrics_tx);

        let mut error: Option<AppError> = None;

        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    if error.is_none() {
                        error = Some(err);
                    }
                    running.store(false, Ordering::SeqCst);
                    shutdown.notify_waiters();
                }
                Err(join_err) => {
                    if error.is_none() {
                        error = Some(join_error(join_err));
                    }
                    running.store(false, Ordering::SeqCst);
                    shutdown.notify_waiters();
                }
            }
        }

        running.store(false, Ordering::SeqCst);
        shutdown.notify_waiters();

        match render_handle.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                if error.is_none() {
                    error = Some(err);
                }
            }
            Err(join_err) => {
                if error.is_none() {
                    error = Some(join_error(join_err));
                }
            }
        }

        if let Some(err) = error {
            return Err(err);
        }

        Ok(PipelineReport {
            frames_received: counters.frames_received(),
            frames_decoded: counters.frames_decoded(),
            frames_dispatched: counters.frames_dispatched(),
            frames_rendered: counters.frames_rendered(),
            frames_dropped: counters.frames_dropped(),
            avg_latency_ms: counters.avg_latency_ms(),
            max_latency_ms: counters.max_latency_ms(),
        })
    }
}

#[derive(Debug)]
struct RenderStage {
    queue: Mutex<RenderQueue>,
    notify: Notify,
}

impl RenderStage {
    fn new(max_frames: usize, max_latency: Duration) -> Self {
        Self {
            queue: Mutex::new(RenderQueue::new(max_frames, max_latency)),
            notify: Notify::new(),
        }
    }

    async fn push(&self, frame: DecodedFrame) -> QueuePushOutcome {
        let mut queue = self.queue.lock().await;
        let outcome = queue.push(frame, SystemTime::now());
        if outcome.accepted {
            self.notify.notify_one();
        }
        outcome
    }

    async fn pop(&self, running: &Arc<AtomicBool>, shutdown: &Arc<Notify>) -> Option<QueuePopOutcome> {
        loop {
            let outcome = {
                let mut queue = self.queue.lock().await;
                queue.pop(SystemTime::now())
            };
            if outcome.is_some() {
                return outcome;
            }
            if !running.load(Ordering::Relaxed) {
                let final_outcome = {
                    let mut queue = self.queue.lock().await;
                    queue.pop(SystemTime::now())
                };
                return final_outcome;
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = shutdown.notified() => return None,
            }
        }
    }
}

#[derive(Debug)]
struct RenderQueue {
    frames: VecDeque<DecodedFrame>,
    max_frames: usize,
    max_latency: Duration,
    dropped_total: u64,
}

impl RenderQueue {
    fn new(max_frames: usize, max_latency: Duration) -> Self {
        Self {
            frames: VecDeque::new(),
            max_frames: max_frames.max(1),
            max_latency,
            dropped_total: 0,
        }
    }

    fn push(&mut self, mut frame: DecodedFrame, now: SystemTime) -> QueuePushOutcome {
        let stale_dropped = self.trim_stale(now);
        let mut overflow_dropped = 0usize;

        if let Ok(elapsed) = now.duration_since(frame.timestamp) {
            if elapsed > self.max_latency + INVALID_FRAME_GRACE {
                self.dropped_total = self.dropped_total.saturating_add(1);
                return QueuePushOutcome {
                    accepted: false,
                    queue_depth: self.frames.len(),
                    stale_dropped,
                    overflow_dropped,
                    dropped_new_frame: true,
                };
            }
        }

        frame.enqueue_instant = Instant::now();

        if self.frames.len() >= self.max_frames {
            self.frames.pop_front();
            self.dropped_total = self.dropped_total.saturating_add(1);
            overflow_dropped += 1;
        }

        self.frames.push_back(frame);
        QueuePushOutcome {
            accepted: true,
            queue_depth: self.frames.len(),
            stale_dropped,
            overflow_dropped,
            dropped_new_frame: false,
        }
    }

    fn pop(&mut self, now: SystemTime) -> Option<QueuePopOutcome> {
        let stale_dropped = self.trim_stale(now);
        let frame = self.frames.pop_front()?;
        let remaining = self.frames.len();
        let queue_latency = frame.enqueue_instant.elapsed();
        Some(QueuePopOutcome {
            frame,
            queue_latency,
            remaining,
            stale_dropped,
            dropped_total: self.dropped_total,
        })
    }

    fn trim_stale(&mut self, now: SystemTime) -> usize {
        let mut dropped = 0usize;
        while let Some(frame) = self.frames.front() {
            match now.duration_since(frame.timestamp) {
                Ok(age) if age > self.max_latency + INVALID_FRAME_GRACE => {
                    self.frames.pop_front();
                    self.dropped_total = self.dropped_total.saturating_add(1);
                    dropped += 1;
                }
                _ => break,
            }
        }
        dropped
    }
}

#[derive(Debug, Clone)]
struct DecodedFrame {
    frame_id: u64,
    session_id: u64,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    timestamp: SystemTime,
    arrival: SystemTime,
    decode_latency: Duration,
    enqueue_instant: Instant,
}

#[derive(Debug)]
struct QueuePushOutcome {
    accepted: bool,
    queue_depth: usize,
    stale_dropped: usize,
    overflow_dropped: usize,
    dropped_new_frame: bool,
}

#[derive(Debug)]
struct QueuePopOutcome {
    frame: DecodedFrame,
    queue_latency: Duration,
    remaining: usize,
    stale_dropped: usize,
    dropped_total: u64,
}

#[instrument(skip(receiver, decode_tx, counters, metrics_tx))]
async fn run_network_loop(
    receiver: NetworkReceiver,
    mut decode_tx: mpsc::Sender<ReceivedFrame>,
    counters: Arc<PipelineCounters>,
    metrics_tx: mpsc::UnboundedSender<MetricEvent>,
    running: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
) -> AppResult<()> {
    while running.load(Ordering::Relaxed) {
        tokio::select! {
            _ = shutdown.notified() => break,
            result = receiver.receive() => {
                match result {
                    Ok(frame) => {
                        counters.record_received();
                        if decode_tx.send(frame).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        warn!(error = %err, "network receive failed; backing off");
                        let _ = metrics_tx.send(MetricEvent::FrameDropped { count: 1 });
                        counters.record_dropped();
                        tokio::time::sleep(NETWORK_BACKOFF).await;
                    }
                }
            }
        }
    }
    Ok(())
}

#[instrument(skip(render_stage, counters, metrics_tx))]
async fn run_decode_loop(
    mut decode_rx: mpsc::Receiver<ReceivedFrame>,
    render_stage: Arc<RenderStage>,
    render_settings: RenderSettings,
    counters: Arc<PipelineCounters>,
    metrics_tx: mpsc::UnboundedSender<MetricEvent>,
    running: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
    expected_bytes: usize,
) -> AppResult<()> {
    let decoder = Decoder::new()?;

    while running.load(Ordering::Relaxed) {
        tokio::select! {
            _ = shutdown.notified() => break,
            maybe_frame = decode_rx.recv() => {
                let Some(frame) = maybe_frame else { break; };
                match decode_frame(&decoder, frame, &render_settings, expected_bytes) {
                    Ok(decoded) => {
                        counters.record_decoded();
                        let outcome = render_stage.push(decoded).await;
                        if outcome.stale_dropped > 0 {
                            counters.record_dropped_many(outcome.stale_dropped as u64);
                            let _ = metrics_tx.send(MetricEvent::FrameDropped { count: outcome.stale_dropped as u64 });
                        }
                        if outcome.overflow_dropped > 0 {
                            counters.record_dropped_many(outcome.overflow_dropped as u64);
                            let _ = metrics_tx.send(MetricEvent::FrameDropped { count: outcome.overflow_dropped as u64 });
                        }
                        if outcome.dropped_new_frame {
                            counters.record_dropped();
                            let _ = metrics_tx.send(MetricEvent::FrameDropped { count: 1 });
                        }
                    }
                    Err(err) => {
                        warn!(error = %err, "decoder rejected frame");
                        counters.record_dropped();
                        let _ = metrics_tx.send(MetricEvent::FrameDropped { count: 1 });
                    }
                }
            }
        }
    }

    Ok(())
}

#[instrument(skip(render_stage, render_tx, counters, metrics_tx))]
async fn run_present_loop(
    render_stage: Arc<RenderStage>,
    render_tx: mpsc::UnboundedSender<RenderFrame>,
    counters: Arc<PipelineCounters>,
    metrics_tx: mpsc::UnboundedSender<MetricEvent>,
    running: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
    frame_limit: Option<u64>,
) -> AppResult<()> {
    while running.load(Ordering::Relaxed) {
        let Some(outcome) = render_stage.pop(&running, &shutdown).await else { break };

        if outcome.stale_dropped > 0 {
            counters.record_dropped_many(outcome.stale_dropped as u64);
            let _ = metrics_tx.send(MetricEvent::FrameDropped { count: outcome.stale_dropped as u64 });
        }

        let frame = outcome.frame;
        let render_frame = RenderFrame {
            pixels: frame.pixels,
            width: frame.width,
            height: frame.height,
            timestamp: frame.timestamp,
            arrival: frame.arrival,
            frame_id: frame.frame_id,
            session_id: frame.session_id,
            decode_latency: frame.decode_latency,
            queue_latency: outcome.queue_latency,
            queue_depth: outcome.remaining,
            dropped_total: outcome.dropped_total,
        };

        if render_tx.send(render_frame).is_err() {
            running.store(false, Ordering::SeqCst);
            shutdown.notify_waiters();
            break;
        }
        counters.record_dispatched();

        if let Some(limit) = frame_limit {
            if counters.frames_dispatched() >= limit {
                running.store(false, Ordering::SeqCst);
                shutdown.notify_waiters();
                break;
            }
        }
    }

    Ok(())
}

#[instrument(skip(metrics_rx, counters, metrics_hub))]
async fn observe_metrics(
    mut metrics_rx: mpsc::UnboundedReceiver<MetricEvent>,
    interval: Duration,
    counters: Arc<PipelineCounters>,
    metrics_hub: MetricsHub,
    running: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
) -> AppResult<()> {
    let mut collector = MetricsCollector::new(interval);

    loop {
        tokio::select! {
            _ = shutdown.notified() => break,
            maybe_event = metrics_rx.recv() => {
                let Some(event) = maybe_event else { break; };
                match event {
                    MetricEvent::FramePresented { latency, queue_latency, decode_latency, queue_depth, dropped_total } => {
                        if let Some(snapshot) = collector.record_presented(latency, queue_latency, decode_latency, queue_depth, dropped_total) {
                            metrics_hub.publish(snapshot.clone());
                            info!(
                                fps = format!("{:.2}", snapshot.fps),
                                avg_latency_ms = format!("{:.2}", snapshot.avg_latency_ms),
                                max_latency_ms = format!("{:.2}", snapshot.max_latency_ms),
                                avg_queue_ms = format!("{:.2}", snapshot.avg_queue_ms),
                                frames_rendered = counters.frames_rendered(),
                                dropped_frames = counters.frames_dropped(),
                                queue_depth = snapshot.queue_depth,
                                "receiver metrics snapshot"
                            );
                        }
                    }
                    MetricEvent::FrameDropped { count } => {
                        if let Some(snapshot) = collector.record_dropped(count) {
                            metrics_hub.publish(snapshot.clone());
                            info!(
                                fps = format!("{:.2}", snapshot.fps),
                                avg_latency_ms = format!("{:.2}", snapshot.avg_latency_ms),
                                dropped_frames = counters.frames_dropped(),
                                queue_depth = snapshot.queue_depth,
                                "receiver metrics snapshot"
                            );
                        }
                    }
                }
            }
        }

        if !running.load(Ordering::Relaxed) {
            break;
        }
    }

    Ok(())
}

fn join_error(err: JoinError) -> AppError {
    AppError::Message(format!("receiver task failed: {err}"))
}

fn decode_frame(
    decoder: &Decoder,
    frame: ReceivedFrame,
    render_settings: &RenderSettings,
    expected_bytes: usize,
) -> AppResult<DecodedFrame> {
    let decode_start = Instant::now();
    let mut payload = decoder.decode(&frame.data)?;
    let decode_latency = decode_start.elapsed();

    if payload.len() >= 4 {
        let marker = &payload[payload.len() - 4..];
        if marker == b"H264" || marker == b"HEVC" {
            payload.truncate(payload.len() - 4);
        }
    }

    if payload.len() < expected_bytes {
        return Err(AppError::Message(format!(
            "decoded frame too small: expected {expected_bytes} bytes, got {}",
            payload.len()
        )));
    }

    if payload.len() > expected_bytes {
        payload.truncate(expected_bytes);
    }

    Ok(DecodedFrame {
        frame_id: frame.frame_id,
        session_id: frame.session_id,
        pixels: payload,
        width: render_settings.width,
        height: render_settings.height,
        timestamp: frame.timestamp,
        arrival: frame.arrival,
        decode_latency,
        enqueue_instant: Instant::now(),
    })
}
