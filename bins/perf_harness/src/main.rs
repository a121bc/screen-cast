use std::path::PathBuf;

use clap::{Parser, ValueHint};
use tracing::info;

use perf_harness::report::BenchmarkSummary;
use perf_harness::scenario::BenchmarkConfig;

#[derive(Debug, Parser)]
#[command(name = "perf-harness", about = "Synthetic benchmarking harness for latency/FPS validation")]
struct Cli {
    /// Optional path to a benchmark scenario configuration file (JSON).
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    config: Option<PathBuf>,

    /// Path to write the JSON summary report.
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    output: Option<PathBuf>,

    /// Path to write a Markdown summary report.
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    markdown: Option<PathBuf>,

    /// Override the random seed used by the simulation.
    #[arg(long, value_name = "SEED")]
    seed: Option<u64>,

    /// Print the Markdown summary to stdout instead of the textual table.
    #[arg(long)]
    print_markdown: bool,
}

fn main() -> shared::AppResult<()> {
    shared::init_tracing()?;

    let cli = Cli::parse();
    let mut config = load_config(cli.config.as_ref())?;
    if let Some(seed) = cli.seed {
        config.seed = seed;
    }

    let summary = perf_harness::run_benchmarks(&config);

    if let Some(path) = cli.output.as_ref() {
        summary.write_json(path)?;
        info!(path = ?path, "wrote JSON summary");
    }

    if let Some(path) = cli.markdown.as_ref() {
        summary.write_markdown(path)?;
        info!(path = ?path, "wrote Markdown summary");
    }

    if cli.print_markdown {
        println!("{}", summary.render_markdown());
    } else {
        print_console_table(&summary);
    }

    Ok(())
}

fn load_config(path: Option<&PathBuf>) -> shared::AppResult<BenchmarkConfig> {
    if let Some(path) = path {
        BenchmarkConfig::from_path(path)
    } else {
        let default_path = PathBuf::from("benchmarks/scenarios.json");
        if default_path.exists() {
            BenchmarkConfig::from_path(&default_path)
        } else {
            Ok(BenchmarkConfig::builtin())
        }
    }
}

fn print_console_table(summary: &BenchmarkSummary) {
    println!(
        "Seed: {} | Overall avg latency: {:.2} ms | Worst P95: {:.2} ms | Drop rate: {:.2}% | Pass: {}",
        summary.seed,
        summary.overall_avg_latency_ms,
        summary.worst_p95_latency_ms,
        summary.overall_drop_rate * 100.0,
        if summary.passes { "yes" } else { "no" }
    );

    println!(
        "{:<24} {:<10} {:<12} {:>6} {:>10} {:>10} {:>10} {:>8} {:>6}",
        "Scenario", "Stress", "Resolution", "FPS", "Avg(ms)", "P95(ms)", "Max(ms)", "Drop%", "Pass"
    );
    println!("{}", "-".repeat(112));

    for report in &summary.reports {
        println!(
            "{:<24} {:<10} {:<12} {:>6} {:>10.2} {:>10.2} {:>10.2} {:>8.2} {:>6}",
            truncate(&report.scenario, 24),
            truncate(&report.stress_level, 10),
            truncate(&report.resolution, 12),
            report.fps,
            report.avg_latency_ms,
            report.p95_latency_ms,
            report.max_latency_ms,
            report.drop_rate * 100.0,
            if report.passes_latency { "✅" } else { "❌" }
        );
    }
}

fn truncate(value: &str, width: usize) -> String {
    if value.len() <= width {
        value.to_string()
    } else if width <= 3 {
        "…".repeat(width)
    } else {
        let mut truncated = value.chars().take(width - 1).collect::<String>();
        truncated.push('…');
        truncated
    }
}
