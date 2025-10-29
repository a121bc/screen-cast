#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(not(target_os = "windows"))]
fn main() {
    panic!("the receiver binary is only supported on Windows targets");
}

#[cfg(all(target_os = "windows", feature = "gui"))]
#[tokio::main(flavor = "multi_thread")]
async fn main() -> shared::AppResult<()> {
    use clap::Parser;

    shared::init_tracing()?;

    let cli = receiver::CliArgs::parse();
    let config = receiver::ReceiverConfig::from_cli(cli)?;

    tracing::info!(
        address = %config.network.address,
        bind = ?config.network.bind,
        width = config.render.width,
        height = config.render.height,
        render_queue = config.pipeline.render_queue,
        max_latency_ms = config.pipeline.max_latency_ms,
        "starting receiver pipeline"
    );

    let pipeline = receiver::ReceiverPipeline::new(config)?;
    let report = pipeline.run().await?;

    tracing::info!(
        frames_received = report.frames_received,
        frames_decoded = report.frames_decoded,
        frames_dispatched = report.frames_dispatched,
        frames_rendered = report.frames_rendered,
        dropped_frames = report.frames_dropped,
        avg_latency_ms = format!("{:.2}", report.avg_latency_ms),
        max_latency_ms = format!("{:.2}", report.max_latency_ms),
        "receiver pipeline completed"
    );

    Ok(())
}

#[cfg(all(target_os = "windows", not(feature = "gui")))]
fn main() {
    eprintln!(
        "receiver was built without GUI support. Re-run with `--features gui` to enable the interface."
    );
}
