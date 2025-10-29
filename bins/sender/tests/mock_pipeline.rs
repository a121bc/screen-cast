#![cfg(target_os = "windows")]

use sender::{BitratePreset, ScalingMethod, ScalingSettings, SenderConfig, SenderPipeline};

#[tokio::test]
async fn mock_pipeline_transmits_expected_frames() {
    let mut config = SenderConfig::default();
    config.pipeline.use_mock_components = true;
    config.pipeline.mock_frame_count = Some(5);
    config.encoder.preset = BitratePreset::Low;
    config.network.address = "127.0.0.1:6001".into();
    config.capture.frame_rate = Some(30);
    config.scaling = Some(ScalingSettings {
        width: 640,
        height: 360,
        method: ScalingMethod::Software,
    });

    let pipeline = SenderPipeline::new(config);
    let report = pipeline.run().await.expect("pipeline should complete");

    assert_eq!(report.frames_transmitted, 5);
    assert_eq!(report.capture_errors, 0);
    assert_eq!(report.dropped_frames, 0);
}
