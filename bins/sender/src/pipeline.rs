#![cfg(target_os = "windows")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use capture::{CapturedFrame, FrameMetadata, MonitorId};
use shared::{AppError, AppResult};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time;
use tracing::{info, warn};

use crate::config::{MetricsSettings, PipelineSettings, ScalingMethod, ScalingSettings, SenderConfig, DEFAULT_FRAME_RATE, DEFAULT_MOCK_FRAME_COUNT};
use crate::metrics::{MetricEvent, MetricsCollector};

const DEFAULT_MOCK_WIDTH: u32 = 1920;
const DEFAULT_MOCK_HEIGHT: u32 = 1080;
const METRICS_CHANNEL_CAPACITY: usize = 32;

pub struct SenderPipeline {
    config: SenderConfig,
}

#[derive(Debug, Default)]
pub struct PipelineCounters {
    frames_sent: AtomicU64,
    capture_errors: AtomicU64,
    dropped_frames: AtomicU64,
}

impl PipelineCounters {
    fn record_frame(&self) {
        self.frames_sent.fetch_add(1, Ordering::Relaxed);
    }

    fn record_capture_error(&self) {
        self.capture_errors.fetch_add(1, Ordering::Relaxed);
    }

    fn record_drop(&self) {
        self.dropped_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub fn frames_sent(&self) -> u64 {
        self.frames_sent.load(Ordering::Relaxed)
    }

    pub fn capture_errors(&self) -> u64 {
        self.capture_errors.load(Ordering::Relaxed)
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PipelineReport {
    pub frames_transmitted: u64,
    pub capture_errors: u64,
    pub dropped_frames: u64,
}

type FrameResult = AppResult<CapturedFrame>;
type FrameSender = mpsc::Sender<FrameResult>;
type FrameReceiver = mpsc::Receiver<FrameResult>;
type MetricsSender = mpsc::Sender<MetricEvent>;
type MetricsReceiver = mpsc::Receiver<MetricEvent>;

enum CaptureMode {
    Real {
        capture_config: capture::CaptureConfig,
        pipeline: PipelineSettings,
    },
    Mock {
        frame_count: usize,
        width: u32,
        height: u32,
        frame_rate: u32,
    },
}

impl SenderPipeline {
    pub fn new(config: SenderConfig) -> Self {
        Self { config }
    }

    pub async fn run(self) -> AppResult<PipelineReport> {
        let mut config = self.config;
        config.validate()?;

        let capture_config = config.capture_config()?;
        let codec_config = config.codec_config()?;
        let scaling = config.scaling.clone();
        let network_settings = config.network.clone();
        let pipeline_settings = config.pipeline.clone();
        let metrics_settings = config.metrics.clone();
        let capture_settings = config.capture.clone();

        let counters = Arc::new(PipelineCounters::default());

        let (frame_tx, frame_rx) = mpsc::channel::<FrameResult>(pipeline_settings.channel_capacity);
        let (metrics_tx, metrics_rx) = mpsc::channel::<MetricEvent>(METRICS_CHANNEL_CAPACITY);

        let capture_mode = if pipeline_settings.use_mock_components {
            let mock_frames = pipeline_settings
                .mock_frame_count
                .unwrap_or(DEFAULT_MOCK_FRAME_COUNT);
            let (width, height) = scaling
                .as_ref()
                .map(|settings| (settings.width, settings.height))
                .unwrap_or((DEFAULT_MOCK_WIDTH, DEFAULT_MOCK_HEIGHT));
            let frame_rate = capture_settings
                .frame_rate
                .unwrap_or(DEFAULT_FRAME_RATE);
            info!(
                frames = mock_frames,
                width,
                height,
                frame_rate,
                "starting sender pipeline with mock capture"
            );
            CaptureMode::Mock {
                frame_count: mock_frames,
                width,
                height,
                frame_rate,
            }
        } else {
            info!(
                monitor = %capture_config.monitor,
                fps = capture_config.frame_rate.get(),
                "starting sender pipeline with DXGI capture"
            );
            CaptureMode::Real {
                capture_config,
                pipeline: pipeline_settings.clone(),
            }
        };

        let mut join_set = JoinSet::new();

        let capture_counters = Arc::clone(&counters);
        let capture_metrics_tx = metrics_tx.clone();
        join_set.spawn(async move {
            run_capture_loop(capture_mode, frame_tx, capture_metrics_tx, capture_counters).await
        });

        let processor_counters = Arc::clone(&counters);
        let processor_metrics_tx = metrics_tx.clone();
        join_set.spawn(async move {
            process_frames(
                frame_rx,
                scaling,
                codec_config,
                network_settings,
                processor_metrics_tx,
                processor_counters,
            )
            .await
        });

        let metrics_counters = Arc::clone(&counters);
        join_set.spawn(async move { observe_metrics(metrics_rx, metrics_settings, metrics_counters).await });

        drop(metrics_tx);

        let mut error: Option<AppError> = None;

        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    error = Some(err);
                    break;
                }
                Err(err) => {
                    error = Some(join_error(err));
                    break;
                }
            }
        }

        if let Some(err) = error {
            join_set.shutdown().await;
            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(inner)) => warn!(error = %inner, "pipeline task returned error during shutdown"),
                    Err(join_err) => warn!(error = %join_err, "pipeline task aborted during shutdown"),
                }
            }
            return Err(err);
        }

        while let Some(result) = join_set.join_next().await {
            result.map_err(join_error)??;
        }

        Ok(PipelineReport {
            frames_transmitted: counters.frames_sent(),
            capture_errors: counters.capture_errors(),
            dropped_frames: counters.dropped_frames(),
        })
    }
}

fn join_error(err: tokio::task::JoinError) -> AppError {
    AppError::Message(format!("sender task failed: {err}"))
}

async fn run_capture_loop(
    mode: CaptureMode,
    frame_tx: FrameSender,
    metrics_tx: MetricsSender,
    counters: Arc<PipelineCounters>,
) -> AppResult<()> {
    match mode {
        CaptureMode::Real {
            capture_config,
            pipeline,
        } => run_real_capture(capture_config, pipeline, frame_tx, metrics_tx, counters).await,
        CaptureMode::Mock {
            frame_count,
            width,
            height,
            frame_rate,
        } => run_mock_capture(frame_count, width, height, frame_rate, frame_tx).await,
    }
}

async fn run_real_capture(
    capture_config: capture::CaptureConfig,
    pipeline: PipelineSettings,
    mut frame_tx: FrameSender,
    metrics_tx: MetricsSender,
    counters: Arc<PipelineCounters>,
) -> AppResult<()> {
    let mut attempts = 0usize;
    loop {
        match capture::FrameCapture::new(capture_config.clone()) {
            Ok(capture) => {
                attempts = 0;
                let mut stream = match capture.into_stream() {
                    Ok(stream) => stream,
                    Err(err) => {
                        counters.record_capture_error();
                        warn!(error = %err, "failed to create capture stream; retrying");
                        let _ = metrics_tx.send(MetricEvent::CaptureError).await;
                        let message = AppError::Message(format!("capture stream error: {err}"));
                        if frame_tx.send(Err(message)).await.is_err() {
                            return Ok(());
                        }
                        attempts = attempts.saturating_add(1);
                        if !should_retry(attempts, pipeline.max_retries) {
                            return Err(AppError::Message(
                                "capture stream initialisation failed repeatedly".into(),
                            ));
                        }
                        time::sleep(pipeline.retry_backoff()).await;
                        continue;
                    }
                };

                while let Some(frame_result) = stream.next_frame().await {
                    match frame_result {
                        Ok(frame) => {
                            attempts = 0;
                            if frame_tx.send(Ok(frame)).await.is_err() {
                                return Ok(());
                            }
                        }
                        Err(err) => {
                            counters.record_capture_error();
                            warn!(error = %err, "capture stream produced error; attempting recovery");
                            let _ = metrics_tx.send(MetricEvent::CaptureError).await;
                            let message = AppError::Message(format!("capture error: {err}"));
                            if frame_tx.send(Err(message)).await.is_err() {
                                return Ok(());
                            }
                            attempts += 1;
                            if !should_retry(attempts, pipeline.max_retries) {
                                return Err(AppError::Message(
                                    "capture failed repeatedly; aborting pipeline".into(),
                                ));
                            }
                            time::sleep(pipeline.retry_backoff()).await;
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                counters.record_capture_error();
                warn!(error = %err, "failed to initialise capture; retrying");
                let _ = metrics_tx.send(MetricEvent::CaptureError).await;
                attempts += 1;
                if !should_retry(attempts, pipeline.max_retries) {
                    return Err(AppError::Message(format!(
                        "initial capture failed after {attempts} attempts: {err}"
                    )));
                }
                time::sleep(pipeline.retry_backoff()).await;
            }
        }
    }
}

async fn run_mock_capture(
    frame_count: usize,
    width: u32,
    height: u32,
    frame_rate: u32,
    mut frame_tx: FrameSender,
) -> AppResult<()> {
    let interval = if frame_rate == 0 {
        Duration::from_millis(16)
    } else {
        Duration::from_secs_f64(1.0 / frame_rate as f64)
    };

    for index in 0..frame_count {
        if frame_tx.is_closed() {
            break;
        }
        let frame = build_mock_frame(width, height, index as u64);
        if frame_tx.send(Ok(frame)).await.is_err() {
            break;
        }
        if !interval.is_zero() {
            time::sleep(interval).await;
        }
    }

    Ok(())
}

async fn process_frames(
    mut frame_rx: FrameReceiver,
    scaling: Option<ScalingSettings>,
    codec_config: codec::CodecConfig,
    network_settings: crate::config::NetworkSettings,
    metrics_tx: MetricsSender,
    counters: Arc<PipelineCounters>,
) -> AppResult<()> {
    let encoder = codec::Encoder::new(codec_config)?;
    let endpoint = network::EndpointConfig::new(network_settings.address.clone());
    let sender = network::NetworkSender::new(endpoint);

    while let Some(message) = frame_rx.recv().await {
        match message {
            Ok(frame) => {
                let processed = if let Some(settings) = scaling.as_ref() {
                    apply_scaling(frame, settings)?
                } else {
                    frame
                };

                let encode_start = Instant::now();
                let encoded = encoder.encode(&processed.bytes)?;
                let encode_latency = encode_start.elapsed();

                sender.send(&encoded).await?;
                counters.record_frame();
                let _ = metrics_tx.send(MetricEvent::Frame { encode_latency }).await;
            }
            Err(err) => {
                counters.record_drop();
                warn!(error = %err, "processor received capture error frame");
                let _ = metrics_tx.send(MetricEvent::CaptureError).await;
            }
        }
    }

    Ok(())
}

async fn observe_metrics(
    mut metrics_rx: MetricsReceiver,
    metrics_settings: MetricsSettings,
    counters: Arc<PipelineCounters>,
) -> AppResult<()> {
    let mut collector = MetricsCollector::new(metrics_settings.interval());

    while let Some(event) = metrics_rx.recv().await {
        match event {
            MetricEvent::Frame { encode_latency } => {
                if let Some(snapshot) = collector.record_frame(encode_latency) {
                    info!(
                        fps = format!("{:.2}", snapshot.fps),
                        avg_encode_ms = format!("{:.2}", snapshot.avg_encode_ms),
                        max_encode_ms = format!("{:.2}", snapshot.max_encode_ms),
                        frames = snapshot.frames,
                        sent_total = counters.frames_sent(),
                        capture_errors = counters.capture_errors(),
                        "sender throughput metrics"
                    );
                }
            }
            MetricEvent::CaptureError => {
                collector.record_error();
            }
        }
    }

    info!(
        sent_total = counters.frames_sent(),
        capture_errors = counters.capture_errors(),
        dropped_frames = counters.dropped_frames(),
        "sender metrics loop terminating"
    );

    Ok(())
}

fn should_retry(attempts: usize, max_retries: usize) -> bool {
    max_retries == 0 || attempts <= max_retries
}

fn apply_scaling(frame: CapturedFrame, scaling: &ScalingSettings) -> AppResult<CapturedFrame> {
    match scaling.method {
        ScalingMethod::Software => scale_software(frame, scaling.width, scaling.height),
        ScalingMethod::Dxgi => {
            warn!(
                width = scaling.width,
                height = scaling.height,
                "DXGI scaling path not yet implemented; falling back to software"
            );
            scale_software(frame, scaling.width, scaling.height)
        }
    }
}

fn scale_software(mut frame: CapturedFrame, width: u32, height: u32) -> AppResult<CapturedFrame> {
    if width == 0 || height == 0 {
        return Err(AppError::Message("scaling dimensions must be positive".into()));
    }

    if frame.width == width && frame.height == height {
        return Ok(frame);
    }

    let src_width = frame.width as usize;
    let src_height = frame.height as usize;
    let src_stride = frame.stride as usize;

    if src_width == 0 || src_height == 0 {
        return Err(AppError::Message("source frame dimensions are invalid".into()));
    }

    let dest_stride = width as usize * 4;
    let mut scaled = vec![0_u8; dest_stride * height as usize];

    for y in 0..height as usize {
        let src_y = y * src_height / height as usize;
        let src_row_offset = src_y * src_stride;
        let dst_row_offset = y * dest_stride;

        for x in 0..width as usize {
            let src_x = x * src_width / width as usize;
            let src_index = src_row_offset + src_x * 4;
            let dst_index = dst_row_offset + x * 4;
            if src_index + 4 <= frame.bytes.len() && dst_index + 4 <= scaled.len() {
                scaled[dst_index..dst_index + 4].copy_from_slice(&frame.bytes[src_index..src_index + 4]);
            }
        }
    }

    frame.bytes = scaled;
    frame.width = width;
    frame.height = height;
    frame.stride = dest_stride as u32;

    Ok(frame)
}

fn build_mock_frame(width: u32, height: u32, frame_index: u64) -> CapturedFrame {
    let width_usize = width as usize;
    let height_usize = height as usize;
    let stride = width_usize * 4;
    let mut bytes = vec![0_u8; stride * height_usize];

    for y in 0..height_usize {
        for x in 0..width_usize {
            let offset = y * stride + x * 4;
            bytes[offset] = (x as u8).wrapping_add(frame_index as u8);
            bytes[offset + 1] = (y as u8).wrapping_add(frame_index as u8);
            bytes[offset + 2] = (frame_index as u8).wrapping_mul(3);
            bytes[offset + 3] = 255;
        }
    }

    let metadata = FrameMetadata {
        timestamp: SystemTime::now(),
        monitor: MonitorId::PRIMARY,
        frame_index,
        accumulated_frames: 1,
        metadata_size: 0,
        pointer_shape_size: 0,
        pointer_position: None,
        pointer_visible: false,
        rects_coalesced: false,
        protected_content_masked: false,
        last_present_qpc: 0,
        last_mouse_update_qpc: 0,
    };

    CapturedFrame {
        bytes,
        width,
        height,
        stride: stride as u32,
        metadata,
    }
}
