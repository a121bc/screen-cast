# Receiver pipeline

The receiver binary turns the UDP payloads emitted by the `sender` application back into a presentable video stream. It wires together three asynchronous stages:

1. **Transport** – wraps the `network` crate to perform handshake negotiation, maintain the jitter buffer, and surface `ReceivedFrame` structs that include sender timestamps.
2. **Decode** – uses the placeholder `codec::Decoder` to convert the payload into a BGRA image while validating the expected resolution.
3. **Render** – maintains a bounded queue that enforces a latency budget (default 500 ms) before uploading the most recent frame to a `wgpu`-backed `eframe` texture for display.

Keeping the render queue shallow ensures that stale frames are discarded instead of accumulating latency. Every drop is tracked and surfaced through metrics to make tuning visible.

## Running

```bash
cargo run -p receiver --features gui -- \
    --address 127.0.0.1:5000 \
    --render-queue 6 \
    --max-latency 450
```

If you disable the `gui` feature the binary prints a reminder to rebuild with GUI support before exiting. All CLI switches have matching JSON configuration keys. Provide `--config receiver.json` to load a file, and then override selected values on the command line.

## Metrics

The pipeline publishes aggregated latency/throughput snapshots via a broadcast channel:

```rust
let pipeline = receiver::ReceiverPipeline::new(config)?;
let metrics = pipeline.metrics_handle();
let mut snapshots = metrics.subscribe();
```

Each `MetricsSnapshot` captures:

- Frames observed and dropped during the interval.
- Frames-per-second and decode throughput.
- Average / max end-to-end latency (sender timestamp ➜ presentation) and queue residency.

## Graphical interface

Launching the receiver with the default configuration opens an `egui` window that embeds the decoded video surface alongside interactive controls:

- A connection indicator summarises the current state (connecting / connected / failed) and the active sender address.
- A sender selector lists every configured endpoint; choosing a target and pressing **Connect** immediately rebinds the network stage. Connection errors surface in-line without tearing down the UI.
- Buffering presets toggle between low-latency and smoother playback strategies. Switching presets updates the render queue limits on the fly and drops any frames that violate the new policy.
- A metrics side panel exposes real-time gauges for FPS and latency plus historical charts driven by the pipeline metrics stream. These plots make it easy to spot jitter or decode spikes while tuning buffering.

Populate the sender selector via the `network.sender_presets` list in the configuration file; the address supplied on the CLI (or in `network.address`) is added automatically when no presets are provided.

Closing the window or hitting an optional `--frame-limit` cleanly tears down the async tasks.
