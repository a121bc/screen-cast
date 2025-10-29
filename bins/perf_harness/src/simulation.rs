use tracing::info;

use crate::report::{build_summary, DropEvent, DropReason, ScenarioReport};
use crate::rng::Rng64;
use crate::scenario::{BenchmarkConfig, ScenarioDefinition, StressLevelDefinition};

pub fn run_all_scenarios(config: &BenchmarkConfig) -> crate::report::BenchmarkSummary {
    let mut reports = Vec::new();

    for scenario in &config.scenarios {
        for (index, stress) in scenario.stress_profiles().into_iter().enumerate() {
            let seed = derive_seed(config.seed, &scenario.name, &stress.label, index as u64);
            let report = run_single_scenario(scenario, &stress, seed);
            info!(
                scenario = %scenario.name,
                stress = %stress.label,
                avg_latency = format!("{:.2}", report.avg_latency_ms),
                drop_rate = format!("{:.2}%", report.drop_rate * 100.0),
                "scenario completed"
            );
            reports.push(report);
        }
    }

    build_summary(config.seed, reports)
}

fn derive_seed(base: u64, name: &str, stress: &str, index: u64) -> u64 {
    let mut hash = base ^ index.wrapping_mul(0x9E37_79B9);
    for byte in name.bytes() {
        hash = hash.wrapping_mul(0xC2B2_AE35).wrapping_add(byte as u64);
    }
    for byte in stress.bytes() {
        hash = hash.wrapping_mul(0x1656_67B1).wrapping_add(byte as u64);
    }
    hash
}

fn run_single_scenario(def: &ScenarioDefinition, stress: &StressLevelDefinition, seed: u64) -> ScenarioReport {
    let total_frames = (def.fps.max(1) as u64) * def.duration_secs.max(1);
    let mut rng = Rng64::new(seed);

    let mut latencies = Vec::with_capacity(total_frames as usize);
    let mut drop_events = Vec::new();
    let mut frames_presented = 0u64;
    let mut frames_dropped = 0u64;

    let base_queue_latency = (def.buffer_target_ms * 0.55).max(15.0);
    let queue_std_base = (base_queue_latency * 0.35).max(6.0);
    let bitrate_factor = (def.bitrate_mbps / 4.0).max(0.6);

    for frame_index in 0..total_frames {
        let capture = sample_capture(def, stress, &mut rng);
        let encode = sample_encode(def, stress, &mut rng);
        let network = sample_network(def, stress, &mut rng);
        let decode = sample_decode(def, stress, &mut rng);

        if rng.uniform() < stress.packet_loss {
            frames_dropped += 1;
            record_drop(
                &mut drop_events,
                frame_index,
                DropReason::PacketLoss,
                capture + encode + network,
            );
            continue;
        }

        let jitter_multiplier = stress.jitter_multiplier.max(0.5);
        let queue_mean = base_queue_latency * jitter_multiplier;
        let queue_std = queue_std_base * jitter_multiplier * (0.8 + 0.1 * bitrate_factor);
        let mut queue_latency = rng
            .gaussian(queue_mean, queue_std)
            .clamp(0.0, def.latency_budget_ms * 0.65);

        let overflow_prob = ((jitter_multiplier - 1.0).max(0.0)) * 0.025;
        if rng.uniform() < overflow_prob {
            frames_dropped += 1;
            record_drop(
                &mut drop_events,
                frame_index,
                DropReason::QueueOverflow,
                capture + encode + network + decode + queue_latency,
            );
            continue;
        }

        let spike_prob = 0.01 * jitter_multiplier;
        if rng.uniform() < spike_prob {
            queue_latency += rng.gaussian(queue_mean * 0.6, queue_std).abs();
            queue_latency = queue_latency.min(def.latency_budget_ms * 0.8);
        }

        let total_latency = capture + encode + network + decode + queue_latency;

        if total_latency > def.latency_budget_ms {
            frames_dropped += 1;
            record_drop(
                &mut drop_events,
                frame_index,
                DropReason::LatencyBudget,
                total_latency,
            );
            continue;
        }

        frames_presented += 1;
        latencies.push(total_latency);
    }

    let frames_simulated = total_frames;
    let drop_rate = if frames_simulated > 0 {
        frames_dropped as f64 / frames_simulated as f64
    } else {
        0.0
    };

    let (avg_latency, max_latency, p95_latency, jitter) = summarise_latencies(&latencies);

    let average_fps = if def.duration_secs > 0 {
        frames_presented as f64 / def.duration_secs as f64
    } else {
        def.fps as f64
    };

    let passes_latency = max_latency <= def.latency_budget_ms && drop_rate <= 0.05;

    let mut notes = Vec::new();
    if drop_rate > 0.03 {
        notes.push(format!(
            "Drop rate {:.2}% exceeded the 3% advisory threshold",
            drop_rate * 100.0
        ));
    }
    if p95_latency > def.latency_budget_ms * 0.9 {
        notes.push(format!(
            "P95 latency {:.2} ms is within 10% of the budget ({:.0} ms)",
            p95_latency, def.latency_budget_ms
        ));
    }

    ScenarioReport {
        scenario: def.name.clone(),
        stress_level: stress.label.clone(),
        resolution: format!("{}x{}", def.width, def.height),
        fps: def.fps,
        bitrate_mbps: def.bitrate_mbps,
        duration_secs: def.duration_secs,
        latency_budget_ms: def.latency_budget_ms,
        frames_simulated,
        frames_presented,
        frames_dropped,
        drop_rate,
        avg_latency_ms: avg_latency,
        p95_latency_ms: p95_latency,
        max_latency_ms: max_latency,
        jitter_ms: jitter,
        average_fps,
        passes_latency,
        notes,
        drop_events,
    }
}

fn record_drop(events: &mut Vec<DropEvent>, frame_index: u64, reason: DropReason, latency_ms: f64) {
    if events.len() < 12 {
        events.push(DropEvent {
            frame_index,
            reason,
            latency_ms,
        });
    }
}

fn sample_capture(def: &ScenarioDefinition, stress: &StressLevelDefinition, rng: &mut Rng64) -> f64 {
    let complexity = def.pixel_complexity();
    let mean = 3.5 * complexity.powf(0.45) * stress.processing_multiplier;
    let std_dev = mean * 0.12;
    rng.gaussian(mean, std_dev).clamp(0.5, 40.0)
}

fn sample_encode(def: &ScenarioDefinition, stress: &StressLevelDefinition, rng: &mut Rng64) -> f64 {
    let complexity = def.pixel_complexity();
    let fps_factor = def.fps as f64 / 60.0;
    let bitrate_factor = (def.bitrate_mbps / 4.0).max(0.6);
    let mean = 5.5 * complexity.powf(0.68) * fps_factor.powf(0.25) * stress.processing_multiplier * bitrate_factor.powf(0.08);
    let std_dev = mean * (0.18 + (stress.processing_multiplier - 1.0).max(0.0) * 0.12);
    rng.gaussian(mean, std_dev).clamp(1.0, 120.0)
}

fn sample_network(def: &ScenarioDefinition, stress: &StressLevelDefinition, rng: &mut Rng64) -> f64 {
    let bitrate_factor = (def.bitrate_mbps / 4.0).max(0.6);
    let base = 26.0 + 4.5 * bitrate_factor;
    let mean = base * stress.jitter_multiplier;
    let std_dev = (6.0 + 2.0 * bitrate_factor) * stress.jitter_multiplier;
    rng.gaussian(mean, std_dev).clamp(2.0, 220.0)
}

fn sample_decode(def: &ScenarioDefinition, stress: &StressLevelDefinition, rng: &mut Rng64) -> f64 {
    let complexity = def.pixel_complexity();
    let mean = 4.2 * complexity.powf(0.52) * stress.processing_multiplier;
    let std_dev = mean * 0.14;
    rng.gaussian(mean, std_dev).clamp(0.8, 80.0)
}

fn summarise_latencies(latencies: &[f64]) -> (f64, f64, f64, f64) {
    if latencies.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }

    let mut sorted = latencies.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let len = sorted.len();
    let max = *sorted.last().unwrap_or(&0.0);
    let p95_index = ((len as f64 * 0.95).ceil() as usize).saturating_sub(1);
    let p95 = sorted.get(p95_index).copied().unwrap_or(max);
    let sum: f64 = latencies.iter().copied().sum();
    let avg = sum / len as f64;
    let variance: f64 = latencies
        .iter()
        .map(|value| {
            let delta = value - avg;
            delta * delta
        })
        .sum::<f64>()
        / len as f64;
    let jitter = variance.sqrt();
    (avg, max, p95, jitter)
}
