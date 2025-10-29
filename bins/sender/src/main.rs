#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(not(target_os = "windows"))]
fn main() {
    panic!("the sender binary is only supported on Windows targets");
}

#[cfg(target_os = "windows")]
#[tokio::main(flavor = "multi_thread")]
async fn main() -> shared::AppResult<()> {
    use clap::Parser;

    shared::init_tracing()?;

    let cli = sender::CliArgs::parse();
    let config = sender::SenderConfig::from_cli(cli)?;

    let pipeline = sender::SenderPipeline::new(config);
    let report = pipeline.run().await?;

    tracing::info!(
        frames_transmitted = report.frames_transmitted,
        capture_errors = report.capture_errors,
        dropped_frames = report.dropped_frames,
        "sender pipeline completed"
    );

    Ok(())
}
