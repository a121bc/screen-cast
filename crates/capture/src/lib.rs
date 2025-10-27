#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(not(target_os = "windows"))]
compile_error!("The capture crate currently only supports Windows targets.");

use shared::{AppError, AppResult};
use tracing::instrument;

#[cfg(feature = "media-foundation")]
use windows::Win32::Media::MediaFoundation::MF_VERSION;

/// High-level handle for configuring and polling Media Foundation capture.
pub struct FrameCapture {
    backend: CaptureBackend,
}

impl FrameCapture {
    #[instrument]
    pub fn new() -> AppResult<Self> {
        Ok(Self {
            backend: CaptureBackend::initialise()?,
        })
    }

    #[instrument(skip(self))]
    pub fn poll_frame(&self) -> AppResult<CapturedFrame> {
        self.backend.poll_frame()
    }
}

#[derive(Debug)]
enum CaptureBackend {
    #[cfg(feature = "media-foundation")]
    MediaFoundation(MediaFoundationBackend),
}

impl CaptureBackend {
    #[instrument]
    fn initialise() -> AppResult<Self> {
        #[cfg(feature = "media-foundation")]
        {
            let backend = MediaFoundationBackend::bootstrap()?;
            Ok(Self::MediaFoundation(backend))
        }

        #[cfg(not(feature = "media-foundation"))]
        {
            Err(AppError::MediaFoundation(
                "Media Foundation support must be enabled to construct FrameCapture",
            ))
        }
    }

    #[instrument]
    fn poll_frame(&self) -> AppResult<CapturedFrame> {
        match self {
            #[cfg(feature = "media-foundation")]
            CaptureBackend::MediaFoundation(backend) => backend.poll_frame(),
        }
    }
}

/// Minimal representation of a captured frame. Placeholder for pixel buffer data.
#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub bytes: Vec<u8>,
    pub descriptor: &'static str,
}

#[cfg(feature = "media-foundation")]
struct MediaFoundationBackend {
    descriptor: &'static str,
}

#[cfg(feature = "media-foundation")]
impl MediaFoundationBackend {
    fn bootstrap() -> AppResult<Self> {
        tracing::debug!("initialising Media Foundation capture backend");
        let _ = MF_VERSION;
        Ok(Self {
            descriptor: shared::media_foundation_descriptor(),
        })
    }

    fn poll_frame(&self) -> AppResult<CapturedFrame> {
        tracing::trace!("polling frame from Media Foundation pipeline");
        Ok(CapturedFrame {
            bytes: Vec::new(),
            descriptor: self.descriptor,
        })
    }
}
