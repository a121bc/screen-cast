use std::fs;
use std::path::Path;

use serde::Deserialize;
use shared::{AppError, AppResult};

#[derive(Debug, Clone, Deserialize)]
pub struct BenchmarkConfig {
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default)]
    pub scenarios: Vec<ScenarioDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioDefinition {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    #[serde(default = "default_bitrate")]
    pub bitrate_mbps: f64,
    #[serde(default = "default_duration")]
    pub duration_secs: u64,
    #[serde(default = "default_latency_budget")]
    pub latency_budget_ms: f64,
    #[serde(default = "default_buffer_target")]
    pub buffer_target_ms: f64,
    #[serde(default)]
    pub stress_levels: Vec<StressLevelDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StressLevelDefinition {
    pub label: String,
    #[serde(default = "default_jitter_multiplier")]
    pub jitter_multiplier: f64,
    #[serde(default = "default_processing_multiplier")]
    pub processing_multiplier: f64,
    #[serde(default)]
    pub packet_loss: f64,
}

impl BenchmarkConfig {
    pub fn from_path(path: &Path) -> AppResult<Self> {
        let contents = fs::read_to_string(path)
            .map_err(|err| AppError::Message(format!("failed to read benchmark config {path:?}: {err}")))?;
        let mut config: Self = serde_json::from_str(&contents)
            .map_err(|err| AppError::Message(format!("invalid benchmark config JSON: {err}")))?;
        config.normalise()?;
        Ok(config)
    }

    pub fn builtin() -> Self {
        let mut config = Self {
            seed: default_seed(),
            scenarios: default_scenarios(),
        };
        let _ = config.normalise();
        config
    }

    pub fn normalise(&mut self) -> AppResult<()> {
        if self.scenarios.is_empty() {
            self.scenarios = default_scenarios();
        }

        for scenario in &mut self.scenarios {
            scenario.ensure_defaults()?;
        }

        Ok(())
    }
}

impl ScenarioDefinition {
    pub fn stress_profiles(&self) -> Vec<StressLevelDefinition> {
        if self.stress_levels.is_empty() {
            default_stress_levels()
        } else {
            self.stress_levels.clone()
        }
    }

    pub fn ensure_defaults(&mut self) -> AppResult<()> {
        if self.fps == 0 {
            return Err(AppError::Message(format!("scenario '{}' must declare fps > 0", self.name)));
        }
        if self.width == 0 || self.height == 0 {
            return Err(AppError::Message(format!(
                "scenario '{}' must declare a positive resolution",
                self.name
            )));
        }
        if self.duration_secs == 0 {
            self.duration_secs = default_duration();
        }
        if self.latency_budget_ms <= 0.0 {
            self.latency_budget_ms = default_latency_budget();
        }
        if self.buffer_target_ms <= 0.0 {
            self.buffer_target_ms = default_buffer_target();
        }
        if self.bitrate_mbps <= 0.0 {
            self.bitrate_mbps = default_bitrate();
        }
        if self.stress_levels.is_empty() {
            self.stress_levels = default_stress_levels();
        } else {
            for profile in &mut self.stress_levels {
                profile.ensure_defaults();
            }
        }
        Ok(())
    }

    pub fn pixel_complexity(&self) -> f64 {
        let pixels = (self.width as f64) * (self.height as f64);
        let baseline = 1_280.0 * 720.0; // 720p baseline
        (pixels / baseline).max(0.1)
    }
}

impl StressLevelDefinition {
    fn ensure_defaults(&mut self) {
        if self.jitter_multiplier <= 0.0 {
            self.jitter_multiplier = default_jitter_multiplier();
        }
        if self.processing_multiplier <= 0.0 {
            self.processing_multiplier = default_processing_multiplier();
        }
        if !(0.0..=0.5).contains(&self.packet_loss) {
            self.packet_loss = self.packet_loss.clamp(0.0, 0.5);
        }
    }
}

fn default_seed() -> u64 {
    0xC0DEC0DE_u64
}

fn default_bitrate() -> f64 {
    4.0
}

fn default_duration() -> u64 {
    60
}

fn default_latency_budget() -> f64 {
    500.0
}

fn default_buffer_target() -> f64 {
    160.0
}

fn default_jitter_multiplier() -> f64 {
    1.0
}

fn default_processing_multiplier() -> f64 {
    1.0
}

fn default_stress_levels() -> Vec<StressLevelDefinition> {
    vec![
        StressLevelDefinition {
            label: "nominal".to_string(),
            jitter_multiplier: 1.0,
            processing_multiplier: 1.0,
            packet_loss: 0.001,
        },
        StressLevelDefinition {
            label: "elevated".to_string(),
            jitter_multiplier: 1.3,
            processing_multiplier: 1.1,
            packet_loss: 0.004,
        },
        StressLevelDefinition {
            label: "extreme".to_string(),
            jitter_multiplier: 1.6,
            processing_multiplier: 1.25,
            packet_loss: 0.012,
        },
    ]
}

fn default_scenarios() -> Vec<ScenarioDefinition> {
    vec![
        ScenarioDefinition {
            name: "1080p_60fps_6mbps".to_string(),
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 6.0,
            duration_secs: 90,
            latency_budget_ms: 500.0,
            buffer_target_ms: 160.0,
            stress_levels: default_stress_levels(),
        },
        ScenarioDefinition {
            name: "1440p_60fps_10mbps".to_string(),
            width: 2560,
            height: 1440,
            fps: 60,
            bitrate_mbps: 10.0,
            duration_secs: 90,
            latency_budget_ms: 500.0,
            buffer_target_ms: 170.0,
            stress_levels: default_stress_levels(),
        },
        ScenarioDefinition {
            name: "2160p_60fps_18mbps".to_string(),
            width: 3840,
            height: 2160,
            fps: 60,
            bitrate_mbps: 18.0,
            duration_secs: 75,
            latency_budget_ms: 500.0,
            buffer_target_ms: 190.0,
            stress_levels: default_stress_levels(),
        },
    ]
}
