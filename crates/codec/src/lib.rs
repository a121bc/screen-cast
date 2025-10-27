#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(not(target_os = "windows"))]
compile_error!("The codec crate currently only supports Windows targets.");

use serde::{Deserialize, Serialize};
use shared::{AppError, AppResult};
use tracing::instrument;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodecConfig {
    pub bitrate: u32,
    pub framerate: u32,
    pub codec: CodecKind,
}

impl Default for CodecConfig {
    fn default() -> Self {
        Self {
            bitrate: 8_000_000,
            framerate: 60,
            codec: CodecKind::H264,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CodecKind {
    H264,
    Hevc,
}

#[derive(Debug)]
pub struct Encoder {
    config: CodecConfig,
}

impl Encoder {
    #[instrument]
    pub fn new(config: CodecConfig) -> AppResult<Self> {
        if config.framerate == 0 {
            return Err(AppError::Message("framerate must be positive".into()));
        }
        Ok(Self { config })
    }

    #[instrument(skip(self, raw_frame))]
    pub fn encode(&self, raw_frame: &[u8]) -> AppResult<Vec<u8>> {
        tracing::debug!(len = raw_frame.len(), "encoding frame using placeholder implementation");
        let mut payload = Vec::from(raw_frame);
        payload.extend_from_slice(self.config.codec.marker());
        Ok(payload)
    }

    pub fn config(&self) -> &CodecConfig {
        &self.config
    }
}

#[derive(Debug)]
pub struct Decoder;

impl Decoder {
    #[instrument]
    pub fn new() -> AppResult<Self> {
        Ok(Self)
    }

    #[instrument(skip(self, packet))]
    pub fn decode(&self, packet: &[u8]) -> AppResult<Vec<u8>> {
        tracing::debug!(len = packet.len(), "decoding packet using placeholder implementation");
        Ok(packet.to_vec())
    }
}

impl CodecKind {
    fn marker(&self) -> &'static [u8] {
        match self {
            CodecKind::H264 => b"H264",
            CodecKind::Hevc => b"HEVC",
        }
    }
}
