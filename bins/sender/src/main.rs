#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(not(target_os = "windows"))]
fn main() {
    panic!("the sender binary is only supported on Windows targets");
}

#[cfg(target_os = "windows")]
#[tokio::main(flavor = "multi_thread")]
async fn main() -> shared::AppResult<()> {
    shared::init_tracing()?;

    let capture = capture::FrameCapture::new()?;
    let encoder = codec::Encoder::new(codec::CodecConfig::default())?;

    let target = "127.0.0.1:5000";
    let endpoint_json = format!("{{\"address\":\"{target}\"}}");
    let endpoint = network::config_from_json(&endpoint_json)?;
    let sender = network::NetworkSender::new(endpoint.clone());

    let frame = capture.poll_frame()?;
    let capture::CapturedFrame { bytes, .. } = frame;
    let encoded = encoder.encode(&bytes)?;
    sender.send(&encoded).await?;
    shared::yield_control().await;
    tracing::info!(addr = %endpoint.address, bytes = encoded.len(), "payload dispatched");

    let accent = shared::ui::accent_colour();
    tracing::info!(?accent, "UI accent colour loaded");
    tracing::info!("sender pipeline executed placeholder routine");

    Ok(())
}
