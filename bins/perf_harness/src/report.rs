use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use shared::{AppError, AppResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    LatencyBudget,
    PacketLoss,
    QueueOverflow,
}

#[derive(Debug, Clone, Serialize)]
pub struct DropEvent {
    pub frame_index: u64,
    pub reason: DropReason,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioReport {
    pub scenario: String,
    pub stress_level: String,
    pub resolution: String,
    pub fps: u32,
    pub bitrate_mbps: f64,
    pub duration_secs: u64,
    pub latency_budget_ms: f64,
    pub frames_simulated: u64,
    pub frames_presented: u64,
    pub frames_dropped: u64,
    pub drop_rate: f64,
    pub avg_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub max_latency_ms: f64,
    pub jitter_ms: f64,
    pub average_fps: f64,
    pub passes_latency: bool,
    pub notes: Vec<String>,
    pub drop_events: Vec<DropEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkSummary {
    pub seed: u64,
    pub generated_at_epoch_ms: u128,
    pub overall_avg_latency_ms: f64,
    pub worst_p95_latency_ms: f64,
    pub overall_drop_rate: f64,
    pub passes: bool,
    pub reports: Vec<ScenarioReport>,
}

impl BenchmarkSummary {
    pub fn write_json(&self, path: &Path) -> AppResult<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|err| AppError::Message(format!("failed to serialise benchmark summary: {err}")))?;
        fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))
            .map_err(|err| AppError::Message(format!("failed to create directory for {path:?}: {err}")))?;
        fs::write(path, json)
            .map_err(|err| AppError::Message(format!("failed to write benchmark summary {path:?}: {err}")))
    }

    pub fn write_markdown(&self, path: &Path) -> AppResult<()> {
        let markdown = self.render_markdown();
        fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))
            .map_err(|err| AppError::Message(format!("failed to create directory for {path:?}: {err}")))?;
        fs::write(path, markdown)
            .map_err(|err| AppError::Message(format!("failed to write markdown summary {path:?}: {err}")))
    }

    pub fn render_markdown(&self) -> String {
        let timestamp = format_timestamp(self.generated_at_epoch_ms);
        let mut out = String::new();
        out.push_str("# Synthetic performance benchmark summary\n\n");
        out.push_str(&format!("- Generated: {timestamp}\n"));
        out.push_str(&format!("- Seed: {}\n", self.seed));
        out.push_str(&format!("- Overall average latency: {:.2} ms\n", self.overall_avg_latency_ms));
        out.push_str(&format!(
            "- Worst 95th percentile latency: {:.2} ms\n",
            self.worst_p95_latency_ms
        ));
        out.push_str(&format!("- Overall drop rate: {:.2}%\n", self.overall_drop_rate * 100.0));
        out.push_str(&format!("- Pass status: {}\n\n", if self.passes { "PASS" } else { "FAIL" }));

        out.push_str("| Scenario | Stress | Resolution | FPS | Bitrate (Mbps) | Avg (ms) | P95 (ms) | Max (ms) | Drop % | Pass |\n");
        out.push_str("|---|---|---|---|---|---|---|---|---|---|\n");
        for report in &self.reports {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {:.1} | {:.2} | {:.2} | {:.2} | {:.2} | {} |\n",
                report.scenario,
                report.stress_level,
                report.resolution,
                report.fps,
                report.bitrate_mbps,
                report.avg_latency_ms,
                report.p95_latency_ms,
                report.max_latency_ms,
                report.drop_rate * 100.0,
                if report.passes_latency { "✅" } else { "❌" }
            ));
        }

        for report in &self.reports {
            if report.drop_events.is_empty() && report.notes.is_empty() {
                continue;
            }
            out.push_str(&format!("\n## {} [{}]\n", report.scenario, report.stress_level));
            if !report.notes.is_empty() {
                out.push_str("\n**Notes**:\n");
                for note in &report.notes {
                    out.push_str(&format!("- {}\n", note));
                }
            }
            if !report.drop_events.is_empty() {
                out.push_str("\n**Drop samples**:\n");
                for event in &report.drop_events {
                    out.push_str(&format!(
                        "- Frame {}: {:?} drop at {:.2} ms\n",
                        event.frame_index, event.reason, event.latency_ms
                    ));
                }
            }
        }

        out
    }
}

pub fn build_summary(seed: u64, mut reports: Vec<ScenarioReport>) -> BenchmarkSummary {
    reports.sort_by(|a, b| a.scenario.cmp(&b.scenario).then(a.stress_level.cmp(&b.stress_level)));

    let generated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let mut total_latency = 0.0;
    let mut total_frames = 0u64;
    let mut total_drops = 0u64;
    let mut worst_p95 = 0.0;
    let mut passes = true;

    for report in &reports {
        total_latency += report.avg_latency_ms * report.frames_presented as f64;
        total_frames += report.frames_presented;
        total_drops += report.frames_dropped;
        if report.p95_latency_ms > worst_p95 {
            worst_p95 = report.p95_latency_ms;
        }
        passes &= report.passes_latency;
    }

    let overall_avg_latency = if total_frames > 0 {
        total_latency / total_frames as f64
    } else {
        0.0
    };

    let overall_drop_rate = if total_frames + total_drops > 0 {
        total_drops as f64 / (total_frames + total_drops) as f64
    } else {
        0.0
    };

    BenchmarkSummary {
        seed,
        generated_at_epoch_ms: generated_at,
        overall_avg_latency_ms: overall_avg_latency,
        worst_p95_latency_ms: worst_p95,
        overall_drop_rate,
        passes,
        reports,
    }
}

fn format_timestamp(ms: u128) -> String {
    use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};

    let secs = (ms / 1_000) as i64;
    let nanos = ((ms % 1_000) * 1_000_000) as u32;
    let naive = NaiveDateTime::from_timestamp_opt(secs, nanos)
        .unwrap_or_else(|| NaiveDateTime::from_timestamp_opt(0, 0).expect("unix epoch"));
    let datetime: DateTime<Utc> = DateTime::from_utc(naive, Utc);
    datetime.to_rfc3339_opts(SecondsFormat::Millis, true)
}
