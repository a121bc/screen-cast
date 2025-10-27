# Windows Capture Workspace

This repository hosts a Rust workspace that targets Windows and provides a modular pipeline for high-fidelity screen capture and streaming. The project is split into focused crates that encapsulate specific responsibilities while sharing common utilities and build configuration.

## Workspace layout

```
.
├── Cargo.toml
├── README.md
├── crates
│   ├── capture       # Media Foundation capture abstractions
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
- **Media Foundation** functionality surfaced through Windows API bindings for efficient media capture and pipeline configuration.

Feature flags are defined in each crate to make platform capabilities explicit and allow binaries to opt into the functionality they require.

## Development

All crates are configured to compile exclusively for Windows targets. Attempting to build on other platforms will result in meaningful build errors. To validate the workspace on Windows, run:

```bash
cargo check --workspace
```

Static analysis tooling is provided via `rustfmt` and `clippy` configuration files at the repository root. Use these tools to maintain consistent formatting and lint quality across crates.

## Next steps

The current code establishes the workspace skeleton, shared error handling, logging, and placeholder APIs across crates. Implementation efforts can now focus on:

1. Integrating real Media Foundation capture pipelines in `capture`.
2. Wiring up encoder/decoder logic in `codec`.
3. Implementing robust network transport in `network`.
4. Building interactive user interfaces for the sender/receiver binaries.

With the workspace in place, contributors can iterate on each component in isolation while maintaining a cohesive sense of the end-to-end architecture.
