#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(not(target_os = "windows"))]
fn main() {
    panic!("the sender binary is only supported on Windows targets");
}

#[cfg(target_os = "windows")]
#[tokio::main(flavor = "multi_thread")]
async fn main() -> shared::AppResult<()> {
    use std::num::NonZeroU32;

    shared::init_tracing()?;

    let monitors = capture::enumerate_monitors()?;
    let monitor = monitors
        .first()
        .ok_or_else(|| shared::AppError::Message("no DXGI outputs available for capture".into()))?;
    tracing::info!(
        monitor = %monitor.name,
        adapter = monitor.id.adapter_index,
        output = monitor.id.output_index,
        resolution = ?monitor.resolution,
        "selected monitor for capture"
    );

    let config = capture::CaptureConfig::new(monitor.id)
        .with_frame_rate(NonZeroU32::new(30).expect("non-zero frame rate"));
    let capture = capture::FrameCapture::new(config)?;
    let mut stream = capture.into_stream()?;

    let frame = stream
        .next_frame()
        .await
        .transpose()?
        .ok_or_else(|| shared::AppError::Message("capture stream closed before producing a frame".into()))?;
    tracing::info!(
        frame_index = frame.metadata.frame_index,
        timestamp = ?frame.metadata.timestamp,
        bytes = frame.bytes.len(),
        "captured BGRA frame from desktop duplication"
    );

    let encoder = codec::Encoder::new(codec::CodecConfig::default())?;

    let target = "127.0.0.1:5000";
    let endpoint_json = format!("{{\"address\":\"{target}\"}}");
    let endpoint = network::config_from_json(&endpoint_json)?;
    let sender = network::NetworkSender::new(endpoint.clone());

    let encoded = encoder.encode(&frame.bytes)?;
    sender.send(&encoded).await?;
    shared::yield_control().await;
    tracing::info!(addr = %endpoint.address, bytes = encoded.len(), "payload dispatched");

    let accent = shared::ui::accent_colour();
    tracing::info!(?accent, "UI accent colour loaded");
    tracing::info!("sender pipeline executed placeholder routine");

    Ok(())
}
