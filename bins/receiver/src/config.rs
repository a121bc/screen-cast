use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Parser;
use serde::Deserialize;
use shared::{AppError, AppResult};

const DEFAULT_ADDRESS: &str = "127.0.0.1:5000";
const DEFAULT_JITTER_CAPACITY: usize = 32;
const DEFAULT_MAX_LATENCY_MS: u64 = 500;
const DEFAULT_DECODE_QUEUE: usize = 8;
const DEFAULT_RENDER_QUEUE: usize = 6;
const DEFAULT_FRAME_WIDTH: u32 = 1920;
const DEFAULT_FRAME_HEIGHT: u32 = 1080;
const DEFAULT_REFRESH_RATE: u32 = 60;
const DEFAULT_METRICS_INTERVAL_SECS: u64 = 5;
const DEFAULT_METRICS_CHANNEL_CAPACITY: usize = 16;
const DEFAULT_WINDOW_TITLE: &str = "UDP Receiver";

#[derive(Debug, Parser)]
#[command(name = "receiver", about = "UDP decoder and renderer pipeline")]
pub struct CliArgs {
    /// Optional path to a JSON configuration file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Override the network endpoint in HOST:PORT format.
    #[arg(long, value_name = "HOST:PORT")]
    pub address: Option<String>,

    /// Bind the local UDP socket to a specific address.
    #[arg(long, value_name = "HOST:PORT")]
    pub bind: Option<String>,

    /// Override the jitter buffer capacity negotiated during handshake.
    #[arg(long, value_name = "FRAMES")]
    pub jitter_capacity: Option<usize>,

    /// Maximum render queue latency in milliseconds before frames are dropped.
    #[arg(long, value_name = "MILLISECONDS")]
    pub max_latency: Option<u64>,

    /// Override the decoder queue capacity.
    #[arg(long, value_name = "FRAMES")]
    pub decode_queue: Option<usize>,

    /// Override the render queue capacity.
    #[arg(long, value_name = "FRAMES")]
    pub render_queue: Option<usize>,

    /// Expected frame width in pixels.
    #[arg(long, value_name = "PIXELS")]
    pub width: Option<u32>,

    /// Expected frame height in pixels.
    #[arg(long, value_name = "PIXELS")]
    pub height: Option<u32>,

    /// Refresh rate hint used to pace presentation.
    #[arg(long, value_name = "HZ")]
    pub refresh_rate: Option<u32>,

    /// Metrics logging interval in seconds.
    #[arg(long, value_name = "SECONDS")]
    pub metrics_interval: Option<u64>,

    /// Customise the receiver window title.
    #[arg(long, value_name = "TITLE")]
    pub window_title: Option<String>,

    /// Stop the pipeline after presenting the specified number of frames.
    #[arg(long, value_name = "COUNT")]
    pub frame_limit: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReceiverConfig {
    #[serde(default)]
    pub network: NetworkSettings,
    #[serde(default)]
    pub pipeline: PipelineSettings,
    #[serde(default)]
    pub render: RenderSettings,
    #[serde(default)]
    pub metrics: MetricsSettings,
}

impl Default for ReceiverConfig {
    fn default() -> Self {
        Self {
            network: NetworkSettings::default(),
            pipeline: PipelineSettings::default(),
            render: RenderSettings::default(),
            metrics: MetricsSettings::default(),
        }
    }
}

impl ReceiverConfig {
    pub fn from_cli(args: CliArgs) -> AppResult<Self> {
        let mut config = if let Some(path) = args.config.as_ref() {
            Self::from_file(path)?
        } else {
            Self::default()
        };

        if let Some(address) = args.address {
            config.network.address = address;
        }

        if let Some(bind) = args.bind {
            config.network.bind = Some(bind);
        }

        if let Some(capacity) = args.jitter_capacity {
            config.network.jitter_buffer_capacity = capacity;
        }

        if let Some(max_latency) = args.max_latency {
            config.pipeline.max_latency_ms = max_latency;
        }

        if let Some(queue) = args.decode_queue {
            config.pipeline.decode_queue = queue;
        }

        if let Some(queue) = args.render_queue {
            config.pipeline.render_queue = queue;
        }

        if let Some(width) = args.width {
            config.render.width = width;
        }

        if let Some(height) = args.height {
            config.render.height = height;
        }

        if let Some(refresh_rate) = args.refresh_rate {
            config.render.refresh_rate = refresh_rate;
        }

        if let Some(interval) = args.metrics_interval {
            config.metrics.log_interval_secs = interval;
        }

        if let Some(title) = args.window_title {
            config.render.window_title = title;
        }

        if let Some(limit) = args.frame_limit {
            config.pipeline.frame_limit = Some(limit);
        }

        config.validate()?;
        Ok(config)
    }

    pub fn endpoint_config(&self) -> network::EndpointConfig {
        let mut endpoint = network::EndpointConfig::new(self.network.address.clone());
        if let Some(bind) = self.network.bind.as_ref() {
            endpoint = endpoint.with_bind_address(bind.clone());
        }
        if let Some(bitrate) = self.network.target_bitrate {
            endpoint = endpoint.with_target_bitrate(bitrate);
        }
        if let Some(max_payload) = self.network.max_packet_payload {
            endpoint = endpoint.with_max_packet_payload(max_payload);
        }
        endpoint = endpoint.with_jitter_capacity(self.network.jitter_buffer_capacity);
        if let Some(timeout) = self.network.handshake_timeout_ms {
            endpoint = endpoint.with_handshake_timeout(Duration::from_millis(timeout.max(1)));
        }
        endpoint
    }

    pub fn validate(&mut self) -> AppResult<()> {
        self.normalise()?;
        self.network.validate()?;
        self.pipeline.validate()?;
        self.render.validate()?;
        Ok(())
    }

    pub fn expected_frame_bytes(&self) -> usize {
        self.render.expected_frame_bytes()
    }

    pub fn max_latency(&self) -> Duration {
        self.pipeline.max_latency()
    }

    pub fn metrics_interval(&self) -> Duration {
        self.metrics.interval()
    }

    pub fn metrics_channel_capacity(&self) -> usize {
        self.metrics.channel_capacity
    }

    pub fn sender_presets(&self) -> &[String] {
        &self.network.sender_presets
    }

    fn from_file(path: &Path) -> AppResult<Self> {
        let contents = fs::read_to_string(path)
            .map_err(|err| AppError::Message(format!("failed to read config {path:?}: {err}")))?;
        serde_json::from_str(&contents)
            .map_err(|err| AppError::Message(format!("invalid receiver config JSON: {err}")))
    }

    fn normalise(&mut self) -> AppResult<()> {
        if self.network.address.trim().is_empty() {
            self.network.address = DEFAULT_ADDRESS.to_string();
        }

        if self.pipeline.decode_queue == 0 {
            self.pipeline.decode_queue = DEFAULT_DECODE_QUEUE;
        }

        if self.pipeline.render_queue == 0 {
            self.pipeline.render_queue = DEFAULT_RENDER_QUEUE;
        }

        if self.render.width == 0 {
            self.render.width = DEFAULT_FRAME_WIDTH;
        }

        if self.render.height == 0 {
            self.render.height = DEFAULT_FRAME_HEIGHT;
        }

        if self.render.refresh_rate == 0 {
            self.render.refresh_rate = DEFAULT_REFRESH_RATE;
        }

        if self.pipeline.max_latency_ms == 0 {
            self.pipeline.max_latency_ms = DEFAULT_MAX_LATENCY_MS;
        }

        if self.metrics.channel_capacity == 0 {
            self.metrics.channel_capacity = DEFAULT_METRICS_CHANNEL_CAPACITY;
        }

        self.network.normalise_presets();

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkSettings {
    #[serde(default = "default_address")]
    pub address: String,
    #[serde(default)]
    pub bind: Option<String>,
    #[serde(default = "default_jitter_capacity")]
    pub jitter_buffer_capacity: usize,
    #[serde(default)]
    pub target_bitrate: Option<u64>,
    #[serde(default)]
    pub max_packet_payload: Option<usize>,
    #[serde(default)]
    pub handshake_timeout_ms: Option<u64>,
    #[serde(default)]
    pub sender_presets: Vec<String>,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            address: default_address(),
            bind: None,
            jitter_buffer_capacity: default_jitter_capacity(),
            target_bitrate: None,
            max_packet_payload: None,
            handshake_timeout_ms: None,
            sender_presets: Vec::new(),
        }
    }
}

impl NetworkSettings {
    fn validate(&self) -> AppResult<()> {
        network::validate_address(&self.address)?;
        if let Some(bind) = self.bind.as_ref() {
            network::validate_address(bind)?;
        }
        if self.jitter_buffer_capacity == 0 {
            return Err(AppError::Message("jitter buffer capacity must be greater than zero".into()));
        }
        Ok(())
    }

    fn normalise_presets(&mut self) {
        let mut presets = Vec::new();
        for preset in std::mem::take(&mut self.sender_presets) {
            let trimmed = preset.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !presets.iter().any(|existing: &String| existing == trimmed) {
                presets.push(trimmed.to_string());
            }
        }
        if !presets.iter().any(|existing| existing == &self.address) {
            presets.push(self.address.clone());
        }
        presets.sort();
        presets.dedup();
        presets.sort_by(|a, b| match (a == &self.address, b == &self.address) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => a.cmp(b),
        });
        self.sender_presets = presets;
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PipelineSettings {
    #[serde(default = "default_decode_queue")]
    pub decode_queue: usize,
    #[serde(default = "default_render_queue")]
    pub render_queue: usize,
    #[serde(default = "default_max_latency_ms")]
    pub max_latency_ms: u64,
    #[serde(default)]
    pub frame_limit: Option<u64>,
}

impl Default for PipelineSettings {
    fn default() -> Self {
        Self {
            decode_queue: DEFAULT_DECODE_QUEUE,
            render_queue: DEFAULT_RENDER_QUEUE,
            max_latency_ms: DEFAULT_MAX_LATENCY_MS,
            frame_limit: None,
        }
    }
}

impl PipelineSettings {
    fn validate(&self) -> AppResult<()> {
        if self.decode_queue == 0 {
            return Err(AppError::Message("decode queue capacity must be greater than zero".into()));
        }
        if self.render_queue == 0 {
            return Err(AppError::Message("render queue capacity must be greater than zero".into()));
        }
        if self.max_latency_ms == 0 {
            return Err(AppError::Message("maximum latency must be greater than zero".into()));
        }
        Ok(())
    }

    pub fn max_latency(&self) -> Duration {
        Duration::from_millis(self.max_latency_ms.max(1))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RenderSettings {
    #[serde(default = "default_frame_width")]
    pub width: u32,
    #[serde(default = "default_frame_height")]
    pub height: u32,
    #[serde(default = "default_refresh_rate")]
    pub refresh_rate: u32,
    #[serde(default = "default_window_title")]
    pub window_title: String,
    #[serde(default)]
    pub vsync: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            width: DEFAULT_FRAME_WIDTH,
            height: DEFAULT_FRAME_HEIGHT,
            refresh_rate: DEFAULT_REFRESH_RATE,
            window_title: DEFAULT_WINDOW_TITLE.to_string(),
            vsync: true,
        }
    }
}

impl RenderSettings {
    fn validate(&self) -> AppResult<()> {
        if self.width == 0 || self.height == 0 {
            return Err(AppError::Message("render dimensions must be positive".into()));
        }
        if self.refresh_rate == 0 {
            return Err(AppError::Message("refresh rate must be positive".into()));
        }
        Ok(())
    }

    pub fn expected_frame_bytes(&self) -> usize {
        (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(4)
    }

    pub fn present_interval(&self) -> Duration {
        if self.refresh_rate == 0 {
            Duration::from_millis(16)
        } else {
            Duration::from_secs_f64(1.0 / self.refresh_rate as f64)
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricsSettings {
    #[serde(default = "default_metrics_interval_secs")]
    pub log_interval_secs: u64,
    #[serde(default = "default_metrics_channel_capacity")]
    pub channel_capacity: usize,
}

impl Default for MetricsSettings {
    fn default() -> Self {
        Self {
            log_interval_secs: DEFAULT_METRICS_INTERVAL_SECS,
            channel_capacity: DEFAULT_METRICS_CHANNEL_CAPACITY,
        }
    }
}

impl MetricsSettings {
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.log_interval_secs.max(1))
    }
}

fn default_address() -> String {
    DEFAULT_ADDRESS.to_string()
}

fn default_jitter_capacity() -> usize {
    DEFAULT_JITTER_CAPACITY
}

fn default_max_latency_ms() -> u64 {
    DEFAULT_MAX_LATENCY_MS
}

fn default_decode_queue() -> usize {
    DEFAULT_DECODE_QUEUE
}

fn default_render_queue() -> usize {
    DEFAULT_RENDER_QUEUE
}

fn default_frame_width() -> u32 {
    DEFAULT_FRAME_WIDTH
}

fn default_frame_height() -> u32 {
    DEFAULT_FRAME_HEIGHT
}

fn default_refresh_rate() -> u32 {
    DEFAULT_REFRESH_RATE
}

fn default_metrics_interval_secs() -> u64 {
    DEFAULT_METRICS_INTERVAL_SECS
}

fn default_metrics_channel_capacity() -> usize {
    DEFAULT_METRICS_CHANNEL_CAPACITY
}

fn default_window_title() -> String {
    DEFAULT_WINDOW_TITLE.to_string()
}
