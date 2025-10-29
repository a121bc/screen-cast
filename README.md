# Windows Capture Workspace

This repository hosts a Rust workspace that targets Windows and provides a modular pipeline for high-fidelity screen capture and streaming. The project is split into focused crates that encapsulate specific responsibilities while sharing common utilities and build configuration.

## Workspace layout

```
.
├── Cargo.toml
├── README.md
├── crates
│   ├── capture       # DXGI desktop duplication capture pipeline
│   ├── codec         # Encoder/decoder primitives and format negotiation
│   ├── network       # Tokio-powered networking utilities
│   └── shared        # Cross-cutting types, logging, and configuration
└── bins
    ├── sender        # Screen capture + encode + transmit application
    └── receiver      # Decode + render application with egui front-end
```

Each crate is configured for Windows targets and uses conditional compilation to expose platform-specific functionality while maintaining clean build failures on unsupported hosts. The sender and receiver binaries orchestrate the library crates to perform end-to-end capture and playback.

## Features and dependencies

The workspace configures shared optional dependencies and features that are enabled by individual crates as needed:

- **Win32 bindings** via the [`windows`](https://crates.io/crates/windows) crate for calling native APIs.
- **Asynchronous runtime** provided by [`tokio`](https://crates.io/crates/tokio).
- **UI rendering** through [`egui`](https://crates.io/crates/egui)`/`[`eframe`](https://crates.io/crates/eframe) for building control surfaces.
- **Desktop Duplication (DXGI)** bindings used to obtain per-frame BGRA surfaces from the Windows compositor.

Feature flags are defined in each crate to make platform capabilities explicit and allow binaries to opt into the functionality they require.

## Desktop duplication requirements

The DXGI-based capture pipeline requires a Windows environment that exposes the Desktop Duplication API:

- Windows 8.1 or newer with GPU drivers that implement WDDM 1.2 or later (Windows 10 is recommended).
- An interactive desktop session; running as a service, over Remote Desktop without console access, or on a locked workstation will cause DXGI to report `ACCESS_DENIED` or `ACCESS_LOST` errors.
- No protected content in the capture region. The compositor masks protected surfaces before duplication and DXGI will surface `ProtectedContentMaskedOut` in the frame metadata.
- Sufficient GPU resources to create a D3D11 device with BGRA support. The crate falls back to recreating the duplication session when the device is removed or reset.

## Development

All crates are configured to compile exclusively for Windows targets. Attempting to build on other platforms will result in meaningful build errors. To validate the workspace on Windows, run:

```bash
cargo check --workspace
```

Static analysis tooling is provided via `rustfmt` and `clippy` configuration files at the repository root. Use these tools to maintain consistent formatting and lint quality across crates.

## Next steps

The current code establishes the workspace skeleton, shared error handling, logging, and placeholder APIs across crates. Implementation efforts can now focus on:

1. Hardening the DXGI desktop duplication pipeline in `capture` with batching, cursor composition, and colour-space controls.
2. Wiring up encoder/decoder logic in `codec`.
3. Implementing robust network transport in `network`.
4. Building interactive user interfaces for the sender/receiver binaries.

With the workspace in place, contributors can iterate on each component in isolation while maintaining a cohesive sense of the end-to-end architecture.

## Sender pipeline

The `sender` binary now assembles an end-to-end pipeline that captures frames, optionally scales them, encodes the output, and pushes the payload over UDP transport. The pipeline is composed of dedicated tasks for capture, processing, and metrics aggregation, and exposes both CLI and configuration file based controls.

### Running the sender

On a Windows host with the required DXGI support, run:

```bash
cargo run -p sender -- \
    --address 192.168.0.42:5000 \
    --scale 1280x720 \
    --bitrate-preset high
```

Key CLI options:

- `--config <PATH>` – load a JSON configuration file.
- `--monitor ADAPTER:OUTPUT` – choose the DXGI output to duplicate.
- `--scale WIDTHxHEIGHT` / `--disable-scaling` – override or disable frame scaling.
- `--bitrate-preset {low|medium|high|custom}` and `--bitrate <BPS>` – control encoder bitrate.
- `--address HOST:PORT` – override the UDP target.
- `--use-mocks` / `--mock-frames <COUNT>` – drive the pipeline with mock capture frames (useful for testing).

### Configuration file

Configuration can also be provided via JSON. An example `sender.json`:

```json
{
  "capture": { "adapter_index": 0, "output_index": 0, "frame_rate": 60 },
  "scaling": { "width": 1280, "height": 720, "method": "software" },
  "encoder": { "preset": "medium" },
  "network": { "address": "127.0.0.1:5000" },
  "pipeline": { "channel_capacity": 6, "max_retries": 5, "retry_backoff_ms": 1000 }
}
```

Launch with `cargo run -p sender -- --config sender.json` and any CLI flags will override the file.

### Metrics and observability

The pipeline logs frames-per-second along with average and worst-case encode time at the interval configured through `metrics.log_interval_secs` (defaults to 5 seconds). Aggregate counters covering transmitted frames, capture errors, and dropped frames are emitted when the pipeline shuts down.

### Testing with mocks

A mock capture harness is available via `cargo test -p sender`. The integration test toggles the `--use-mocks` pathway to validate that frames flow through capture, scaling, encoding, and transport stages without touching real hardware.
