# Benchmarking and profiling guide

This repository now ships with a synthetic benchmarking harness that exercises the sender/receiver pipeline settings under multiple resolutions, bitrates, and jitter profiles. The goal is to validate the sub-500 ms end-to-end latency target while capturing frame drop behaviour and surfacing potential bottlenecks before running hardware-backed tests on Windows.

## Components

- `perf_harness` binary (see [`bins/perf_harness`](../../bins/perf_harness)) generates deterministic latency, jitter, and drop statistics using the same buffering heuristics that the receiver applies.
- Scenario definitions live in [`benchmarks/scenarios.json`](../../benchmarks/scenarios.json) and cover nominal, elevated, and extreme load profiles for 1080p, 1440p, and 2160p streams.
- Output artefacts are written to [`benchmarks/results`](../../benchmarks/results) as both structured JSON (`latest.json`) and a human-readable Markdown summary (`latest.md`).
- A convenience PowerShell script is provided at [`benchmarks/run_perf_tests.ps1`](../../benchmarks/run_perf_tests.ps1) for Windows hosts.

## Running the harness on Windows

1. Install the Rust toolchain (`rustup` + stable channel).
2. Open a Developer PowerShell prompt and clone this repository if you have not already.
3. Execute the harness via the helper script:

   ```powershell
   ./benchmarks/run_perf_tests.ps1 -Release
   ```

   The script runs the harness in release mode against `benchmarks/scenarios.json` and writes refreshed summaries to `benchmarks/results/latest.json` and `benchmarks/results/latest.md`.

4. To tweak scenarios, edit `benchmarks/scenarios.json` and rerun the script. Each scenario entry can override resolution, FPS, bitrate, buffer target, and the stress profiles (jitter multiplier, processing multiplier, packet loss).

You can also invoke the binary directly:

```powershell
cargo run -p perf_harness --release -- \
    --config benchmarks/scenarios.json \
    --output benchmarks/results/latest.json \
    --markdown benchmarks/results/latest.md
```

Command-line options include:

- `--seed <u64>` to override the deterministic RNG seed.
- `--print-markdown` to dump the Markdown table to stdout.
- Custom output locations with `--output`/`--markdown`.

## Latest benchmark highlights

The current synthetic benchmark sweep (seed `3032024109`) produced the following aggregated metrics:

- **Overall average latency:** 225.36 ms
- **Worst 95th percentile latency:** 457.53 ms
- **Overall drop rate:** 1.91 %
- **Pass status:** ✅ All scenarios remained within the 500 ms target and under the 5 % drop budget.

Detailed per-scenario results, including representative drop events and stress-level notes, are captured in [`benchmarks/results/latest.md`](../../benchmarks/results/latest.md).

## Applied tuning changes

Findings from the harness runs informed a set of default adjustments that keep buffers primed for bursty traffic while limiting head-of-line blocking:

- **Receiver buffering defaults** now provision more headroom:
  - Jitter buffer capacity increased from 32 → 48 frames.
  - Decode queue expanded from 8 → 10 frames.
  - Render queue expanded from 6 → 8 frames.
- **Sender pipeline channel capacity** increased from 4 → 6 to absorb encode bursts without stalling the capture loop.
- **Capture thread priority** is elevated to `THREAD_PRIORITY_ABOVE_NORMAL` on Windows so desktop duplication pre-empts best-effort background work.

These changes reduce queue oscillations under stress while preserving the sub-500 ms latency commitment. Use the harness to validate additional resolutions or bespoke network conditions before rolling the adjustments into production builds.
