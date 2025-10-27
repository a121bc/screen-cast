#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(not(target_os = "windows"))]
fn main() {
    panic!("the receiver binary is only supported on Windows targets");
}

#[cfg(target_os = "windows")]
#[tokio::main(flavor = "multi_thread")]
async fn main() -> shared::AppResult<()> {
    shared::init_tracing()?;

    let decoder = codec::Decoder::new()?;
    let endpoint_json = "{\"address\":\"127.0.0.1:5000\"}";
    let endpoint = network::config_from_json(endpoint_json)?;
    let receiver = network::NetworkReceiver::new(endpoint.clone());

    let packet = receiver.receive().await?;
    shared::yield_control().await;
    let frame = decoder.decode(&packet)?;

    tracing::info!(addr = %endpoint.address, bytes = packet.len(), "placeholder payload received");
    let accent = shared::ui::accent_colour();
    tracing::info!(?accent, size = frame.len(), "received frame in placeholder pipeline");

    Ok(())
}
