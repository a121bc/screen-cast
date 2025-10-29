#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(not(target_os = "windows"))]
fn main() {
    panic!("the sender binary is only supported on Windows targets");
}

#[cfg(target_os = "windows")]
fn main() -> shared::AppResult<()> {
    use clap::Parser;

    shared::init_tracing()?;

    let cli = sender::CliArgs::parse();
    let use_gui = cli.gui;
    let config = sender::SenderConfig::from_cli(cli)?;

    if use_gui {
        sender::ui::run_gui(config)
    } else {
        run_headless(config)
    }
}

#[cfg(target_os = "windows")]
fn run_headless(config: sender::SenderConfig) -> shared::AppResult<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| shared::AppError::Message(format!("failed to build runtime: {err}")))?;

    let pipeline = sender::SenderPipeline::new(config);
    let report = runtime.block_on(async { pipeline.run().await })?;

    tracing::info!(
        frames_transmitted = report.frames_transmitted,
        capture_errors = report.capture_errors,
        dropped_frames = report.dropped_frames,
        "sender pipeline completed"
    );

    Ok(())
}
