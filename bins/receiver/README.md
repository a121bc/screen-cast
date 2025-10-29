# Receiver pipeline

The receiver binary turns the UDP payloads emitted by the `sender` application back into a presentable video stream. It wires together three asynchronous stages:

1. **Transport** – wraps the `network` crate to perform handshake negotiation, maintain the jitter buffer, and surface `ReceivedFrame` structs that include sender timestamps.
2. **Decode** – uses the placeholder `codec::Decoder` to convert the payload into a BGRA image while validating the expected resolution.
3. **Render** – maintains a bounded queue that enforces a latency budget (default 500 ms) before uploading the most recent frame to a `wgpu`-backed `eframe` texture for display.

Keeping the render queue shallow ensures that stale frames are discarded instead of accumulating latency. Every drop is tracked and surfaced through metrics to make tuning visible.

## Running

```bash
cargo run -p receiver -- \
    --address 127.0.0.1:5000 \
    --render-queue 6 \
    --max-latency 450
```

All CLI switches have matching JSON configuration keys. Provide `--config receiver.json` to load a file, and then override selected values on the command line.

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

The UI overlays the latest latency, queue depth, and drop counters alongside the active frame. Closing the window or hitting an optional `--frame-limit` cleanly tears down the async tasks.
