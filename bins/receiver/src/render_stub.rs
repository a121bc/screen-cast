use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use shared::{AppError, AppResult};
use tokio::sync::{broadcast, mpsc, Notify};

use crate::config::RenderSettings;
use crate::metrics::MetricEvent;
use crate::pipeline::{ControlEvent, PipelineCounters, PipelineStatus};

#[derive(Debug, Clone)]
pub struct BufferingPresetConfig {
    pub label: String,
    pub max_frames: usize,
    pub max_latency_ms: u64,
}

impl BufferingPresetConfig {
    pub fn new(label: impl Into<String>, max_frames: usize, max_latency_ms: u64) -> Self {
        Self {
            label: label.into(),
            max_frames: max_frames.max(1),
            max_latency_ms: max_latency_ms.max(1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: SystemTime,
    pub arrival: SystemTime,
    pub frame_id: u64,
    pub session_id: u64,
    pub decode_latency: Duration,
    pub queue_latency: Duration,
    pub queue_depth: usize,
    pub dropped_total: u64,
}

#[allow(clippy::too_many_arguments)]
pub fn run_renderer(
    _frame_rx: mpsc::UnboundedReceiver<RenderFrame>,
    _metrics_tx: mpsc::UnboundedSender<MetricEvent>,
    _counters: Arc<PipelineCounters>,
    _running: Arc<AtomicBool>,
    _shutdown: Arc<Notify>,
    _settings: RenderSettings,
    _frame_limit: Option<u64>,
    _control_tx: broadcast::Sender<ControlEvent>,
    _status_rx: mpsc::UnboundedReceiver<PipelineStatus>,
    _sender_presets: Vec<String>,
    _buffering_presets: Vec<BufferingPresetConfig>,
) -> AppResult<()> {
    Err(AppError::Message(
        "receiver was built without GUI support. Re-run with `--features gui` to enable the interface.".into(),
    ))
}
