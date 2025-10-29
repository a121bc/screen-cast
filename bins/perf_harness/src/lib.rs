pub mod rng;
pub mod scenario;
pub mod simulation;
pub mod report;

use report::BenchmarkSummary;
use scenario::BenchmarkConfig;
use simulation::run_all_scenarios;

pub fn run_benchmarks(config: &BenchmarkConfig) -> BenchmarkSummary {
    run_all_scenarios(config)
}
