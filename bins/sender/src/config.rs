#![cfg(target_os = "windows")]

use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Duration;

use capture::{CaptureConfig, MonitorId};
use clap::{Parser, ValueEnum};
use codec::{CodecConfig, CodecKind};
use serde::Deserialize;
use shared::{AppError, AppResult};

pub(crate) const DEFAULT_FRAME_RATE: u32 = 60;
const DEFAULT_METRICS_INTERVAL_SECS: u64 = 5;
const DEFAULT_CHANNEL_CAPACITY: usize = 4;
const DEFAULT_RETRY_ATTEMPTS: usize = 5;
const DEFAULT_RETRY_BACKOFF_MS: u64 = 1_000;
const DEFAULT_BITRATE_LOW: u32 = 4_000_000;
const DEFAULT_BITRATE_MEDIUM: u32 = 8_000_000;
const DEFAULT_BITRATE_HIGH: u32 = 12_000_000;
pub(crate) const DEFAULT_MOCK_FRAME_COUNT: usize = 120;

#[derive(Debug, Parser)]
#[command(name = "sender", about = "Desktop capture sender pipeline")]
pub struct CliArgs {
    /// Optional path to a JSON configuration file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Override the monitor using "adapter:output" notation (e.g. 0:1).
    #[arg(long, value_name = "ADAPTER:OUTPUT")]
    pub monitor: Option<String>,

    /// Override the capture frame rate (frames per second).
    #[arg(long, value_name = "FPS")]
    pub frame_rate: Option<u32>,

    /// Enable scaling to the provided resolution expressed as WIDTHxHEIGHT.
    #[arg(long, value_parser = parse_resolution, value_name = "WIDTHxHEIGHT")]
    pub scale: Option<ResolutionOverride>,

    /// Disable any scaling configured via file or CLI.
    #[arg(long)]
    pub disable_scaling: bool,

    /// Override the encoder bitrate preset.
    #[arg(long, value_enum)]
    pub bitrate_preset: Option<BitratePreset>,

    /// Override the encoder bitrate in bits per second.
    #[arg(long, value_name = "BPS")]
    pub bitrate: Option<u32>,

    /// Override the network destination in HOST:PORT format.
    #[arg(long, value_name = "HOST:PORT")]
    pub address: Option<String>,

    /// Run the pipeline using mock capture components (useful for testing).
    #[arg(long)]
    pub use_mocks: bool,

    /// When using mocks, limit the number of frames to emit before shutting down.
    #[arg(long, value_name = "COUNT")]
    pub mock_frames: Option<usize>,

    /// Launch the interactive GUI instead of running headless.
    #[arg(long)]
    pub gui: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SenderConfig {
    #[serde(default)]
    pub capture: CaptureSettings,
    #[serde(default)]
    pub scaling: Option<ScalingSettings>,
    #[serde(default)]
    pub encoder: EncoderSettings,
    #[serde(default)]
    pub network: NetworkSettings,
    #[serde(default)]
    pub pipeline: PipelineSettings,
    #[serde(default)]
    pub metrics: MetricsSettings,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            capture: CaptureSettings::default(),
            scaling: None,
            encoder: EncoderSettings::default(),
            network: NetworkSettings::default(),
            pipeline: PipelineSettings::default(),
            metrics: MetricsSettings::default(),
        }
    }
}

impl SenderConfig {
    pub fn from_cli(args: CliArgs) -> AppResult<Self> {
        let mut config = if let Some(path) = args.config.as_ref() {
            Self::from_file(path)?
        } else {
            Self::default()
        };

        if let Some(raw_monitor) = args.monitor {
            let selection = parse_monitor(&raw_monitor)?;
            config.capture.adapter_index = Some(selection.adapter_index);
            config.capture.output_index = Some(selection.output_index);
        }

        if let Some(frame_rate) = args.frame_rate {
            config.capture.frame_rate = Some(frame_rate);
        }

        if let Some(resolution) = args.scale {
            let mut scaling = config.scaling.unwrap_or_else(|| ScalingSettings {
                width: resolution.width,
                height: resolution.height,
                method: ScalingMethod::Software,
            });
            scaling.width = resolution.width;
            scaling.height = resolution.height;
            config.scaling = Some(scaling);
        }

        if args.disable_scaling {
            config.scaling = None;
        }

        if let Some(preset) = args.bitrate_preset {
            config.encoder.preset = preset;
        }

        if let Some(bitrate) = args.bitrate {
            config.encoder.bitrate = Some(bitrate);
        }

        if let Some(address) = args.address {
            config.network.address = address;
        }

        if args.use_mocks {
            config.pipeline.use_mock_components = true;
        }

        if let Some(frames) = args.mock_frames {
            config.pipeline.mock_frame_count = Some(frames);
        }

        config.validate()?;
        Ok(config)
    }

    pub fn validate(&mut self) -> AppResult<()> {
        self.normalise()
    }

    pub fn capture_config(&self) -> AppResult<CaptureConfig> {
        let frame_rate = self.capture.frame_rate()?;
        let monitor = self.capture.monitor_id();
        Ok(CaptureConfig::new(monitor).with_frame_rate(frame_rate))
    }

    pub fn codec_config(&self) -> AppResult<CodecConfig> {
        let source_rate = self.capture.frame_rate.unwrap_or(DEFAULT_FRAME_RATE);
        let bitrate = self.encoder.effective_bitrate()?;
        let framerate = self.encoder.framerate.unwrap_or(source_rate);
        if framerate == 0 {
            return Err(AppError::Message("encoder framerate must be positive".into()));
        }
        Ok(CodecConfig {
            bitrate,
            framerate,
            codec: CodecKind::H264,
        })
    }

    pub fn metrics_interval(&self) -> Duration {
        self.metrics.interval()
    }


    pub fn scaling(&self) -> Option<&ScalingSettings> {
        self.scaling.as_ref()
    }

    fn from_file(path: &Path) -> AppResult<Self> {
        let contents = fs::read_to_string(path)
            .map_err(|err| AppError::Message(format!("failed to read config {path:?}: {err}")))?;
        serde_json::from_str(&contents)
            .map_err(|err| AppError::Message(format!("invalid sender config JSON: {err}")))
    }

    fn normalise(&mut self) -> AppResult<()> {
        if self.capture.frame_rate.unwrap_or(DEFAULT_FRAME_RATE) == 0 {
            return Err(AppError::Message("capture frame rate must be positive".into()));
        }

        if let Some(ref scaling) = self.scaling {
            if scaling.width == 0 || scaling.height == 0 {
                return Err(AppError::Message("scaling dimensions must be positive".into()));
            }
        }

        if self.pipeline.channel_capacity == 0 {
            self.pipeline.channel_capacity = DEFAULT_CHANNEL_CAPACITY;
        }

        if self.pipeline.use_mock_components && self.pipeline.mock_frame_count.is_none() {
            self.pipeline.mock_frame_count = Some(DEFAULT_MOCK_FRAME_COUNT);
        }

        if self.network.address.is_empty() {
            self.network.address = NetworkSettings::default().address;
        }

        network::validate_address(&self.network.address)?;

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaptureSettings {
    pub adapter_index: Option<u32>,
    pub output_index: Option<u32>,
    pub frame_rate: Option<u32>,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            adapter_index: None,
            output_index: None,
            frame_rate: Some(DEFAULT_FRAME_RATE),
        }
    }
}

impl CaptureSettings {
    pub fn monitor_id(&self) -> MonitorId {
        MonitorId {
            adapter_index: self.adapter_index.unwrap_or(MonitorId::PRIMARY.adapter_index),
            output_index: self.output_index.unwrap_or(MonitorId::PRIMARY.output_index),
        }
    }

    pub fn frame_rate(&self) -> AppResult<NonZeroU32> {
        let rate = self.frame_rate.unwrap_or(DEFAULT_FRAME_RATE);
        NonZeroU32::new(rate)
            .ok_or_else(|| AppError::Message("capture frame rate must be positive".into()))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EncoderSettings {
    #[serde(default)]
    pub preset: BitratePreset,
    #[serde(default)]
    pub bitrate: Option<u32>,
    #[serde(default)]
    pub framerate: Option<u32>,
}

impl Default for EncoderSettings {
    fn default() -> Self {
        Self {
            preset: BitratePreset::Medium,
            bitrate: None,
            framerate: None,
        }
    }
}

impl EncoderSettings {
    pub fn effective_bitrate(&self) -> AppResult<u32> {
        if let Some(value) = self.bitrate {
            if value == 0 {
                return Err(AppError::Message("encoder bitrate must be positive".into()));
            }
            return Ok(value);
        }

        match self.preset {
            BitratePreset::Low => Ok(DEFAULT_BITRATE_LOW),
            BitratePreset::Medium => Ok(DEFAULT_BITRATE_MEDIUM),
            BitratePreset::High => Ok(DEFAULT_BITRATE_HIGH),
            BitratePreset::Custom => Err(AppError::Message(
                "custom preset requires explicit bitrate override".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkSettings {
    #[serde(default = "default_address")]
    pub address: String,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            address: default_address(),
        }
    }
}

fn default_address() -> String {
    "127.0.0.1:5000".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct PipelineSettings {
    #[serde(default = "default_channel_capacity")]
    pub channel_capacity: usize,
    #[serde(default = "default_retry_attempts")]
    pub max_retries: usize,
    #[serde(default = "default_retry_backoff")]
    pub retry_backoff_ms: u64,
    #[serde(default)]
    pub use_mock_components: bool,
    #[serde(default)]
    pub mock_frame_count: Option<usize>,
}

impl Default for PipelineSettings {
    fn default() -> Self {
        Self {
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
            max_retries: DEFAULT_RETRY_ATTEMPTS,
            retry_backoff_ms: DEFAULT_RETRY_BACKOFF_MS,
            use_mock_components: false,
            mock_frame_count: None,
        }
    }
}

impl PipelineSettings {
    pub fn retry_backoff(&self) -> Duration {
        Duration::from_millis(self.retry_backoff_ms.max(1))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricsSettings {
    #[serde(default = "default_metrics_interval_secs")]
    pub log_interval_secs: u64,
}

impl Default for MetricsSettings {
    fn default() -> Self {
        Self {
            log_interval_secs: DEFAULT_METRICS_INTERVAL_SECS,
        }
    }
}

impl MetricsSettings {
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.log_interval_secs.clamp(1, u64::MAX))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum BitratePreset {
    Low,
    Medium,
    High,
    Custom,
}

impl Default for BitratePreset {
    fn default() -> Self {
        BitratePreset::Medium
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScalingSettings {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub method: ScalingMethod,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalingMethod {
    Dxgi,
    Software,
}

impl Default for ScalingMethod {
    fn default() -> Self {
        ScalingMethod::Software
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResolutionOverride {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy)]
struct MonitorSelection {
    adapter_index: u32,
    output_index: u32,
}

fn parse_resolution(raw: &str) -> Result<ResolutionOverride, String> {
    let parts: Vec<_> = raw.split(['x', 'X']).collect();
    if parts.len() != 2 {
        return Err("expected format WIDTHxHEIGHT".into());
    }
    let width: u32 = parts[0]
        .parse()
        .map_err(|_| "failed to parse width component".to_string())?;
    let height: u32 = parts[1]
        .parse()
        .map_err(|_| "failed to parse height component".to_string())?;
    if width == 0 || height == 0 {
        return Err("dimensions must be positive".into());
    }
    Ok(ResolutionOverride { width, height })
}

fn parse_monitor(raw: &str) -> Result<MonitorSelection, AppError> {
    let parts: Vec<_> = raw.split([':', ',']).collect();
    if parts.len() != 2 {
        return Err(AppError::Message(
            "monitor override must use ADAPTER:OUTPUT format".into(),
        ));
    }
    let adapter_index = parts[0]
        .parse()
        .map_err(|_| AppError::Message("failed to parse adapter index".into()))?;
    let output_index = parts[1]
        .parse()
        .map_err(|_| AppError::Message("failed to parse output index".into()))?;
    Ok(MonitorSelection {
        adapter_index,
        output_index,
    })
}

fn default_metrics_interval_secs() -> u64 {
    DEFAULT_METRICS_INTERVAL_SECS
}

fn default_channel_capacity() -> usize {
    DEFAULT_CHANNEL_CAPACITY
}

fn default_retry_attempts() -> usize {
    DEFAULT_RETRY_ATTEMPTS
}

fn default_retry_backoff() -> u64 {
    DEFAULT_RETRY_BACKOFF_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resolution_valid() {
        let parsed = parse_resolution("1280x720").unwrap();
        assert_eq!(parsed.width, 1280);
        assert_eq!(parsed.height, 720);
    }

    #[test]
    fn parse_resolution_invalid() {
        assert!(parse_resolution("invalid").is_err());
        assert!(parse_resolution("1280x0").is_err());
    }

    #[test]
    fn parse_monitor_valid() {
        let parsed = parse_monitor("1:2").unwrap();
        assert_eq!(parsed.adapter_index, 1);
        assert_eq!(parsed.output_index, 2);
    }
}
