#![cfg(target_os = "windows")]

use std::num::NonZeroU32;

use capture::{enumerate_monitors, CaptureConfig, FrameCapture};
use shared::{AppError, AppResult};

#[tokio::test]
async fn capture_stream_initialises() -> AppResult<()> {
    let _ = shared::init_tracing();

    let monitors = enumerate_monitors()?;
    let monitor = monitors
        .first()
        .ok_or_else(|| AppError::Message("no DXGI outputs available".into()))?;

    let config = CaptureConfig::new(monitor.id)
        .with_frame_rate(NonZeroU32::new(15).expect("frame rate must be non-zero"));
    let capture = FrameCapture::new(config)?;
    let mut stream = capture.into_stream()?;
    let _ = stream.next_frame().await.transpose()?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires active desktop session for DXGI duplication"]
async fn capture_stream_multiple_frames() -> AppResult<()> {
    let _ = shared::init_tracing();

    let monitors = enumerate_monitors()?;
    let monitor = monitors
        .first()
        .ok_or_else(|| AppError::Message("no DXGI outputs available".into()))?;

    let capture = FrameCapture::new(CaptureConfig::new(monitor.id))?;
    let mut stream = capture.into_stream()?;

    for _ in 0..3 {
        if let Some(frame) = stream.next_frame().await.transpose()? {
            assert_eq!(frame.metadata.monitor, monitor.id);
            assert_eq!(frame.bytes.len(), (frame.width * frame.height * 4) as usize);
        }
    }

    Ok(())
}
