#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(target_os = "windows")]
pub mod config;
#[cfg(target_os = "windows")]
pub mod metrics;
#[cfg(target_os = "windows")]
pub mod pipeline;

#[cfg(target_os = "windows")]
pub use config::{BitratePreset, CaptureSettings, CliArgs, EncoderSettings, MetricsSettings, NetworkSettings, PipelineSettings, ScalingMethod, ScalingSettings, SenderConfig};
#[cfg(target_os = "windows")]
pub use pipeline::{PipelineReport, SenderPipeline};
