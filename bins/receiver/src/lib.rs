#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(target_os = "windows")]
pub mod config;
#[cfg(target_os = "windows")]
pub mod metrics;
#[cfg(target_os = "windows")]
pub mod pipeline;
#[cfg(target_os = "windows")]
pub mod render;

#[cfg(target_os = "windows")]
pub use config::{CliArgs, MetricsSettings, NetworkSettings, PipelineSettings, ReceiverConfig, RenderSettings};
#[cfg(target_os = "windows")]
pub use metrics::{MetricsHandle, MetricsSnapshot};
#[cfg(target_os = "windows")]
pub use pipeline::{PipelineReport, ReceiverPipeline};
