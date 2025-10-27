#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(not(target_os = "windows"))]
compile_error!("The network crate currently only supports Windows targets.");

use serde::{Deserialize, Serialize};
use shared::{AppError, AppResult};
use tracing::instrument;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    pub address: String,
}

impl EndpointConfig {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NetworkSender {
    config: EndpointConfig,
}

impl NetworkSender {
    pub fn new(config: EndpointConfig) -> Self {
        Self { config }
    }

    #[cfg(feature = "runtime")]
    #[instrument(skip(self, payload))]
    pub async fn send(&self, payload: &[u8]) -> AppResult<()> {
        tracing::debug!(len = payload.len(), addr = %self.config.address, "sending payload over placeholder transport");
        tokio::task::yield_now().await;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct NetworkReceiver {
    config: EndpointConfig,
}

impl NetworkReceiver {
    pub fn new(config: EndpointConfig) -> Self {
        Self { config }
    }

    #[cfg(feature = "runtime")]
    #[instrument(skip(self))]
    pub async fn receive(&self) -> AppResult<Vec<u8>> {
        tracing::debug!(addr = %self.config.address, "receiving payload over placeholder transport");
        tokio::task::yield_now().await;
        Ok(Vec::new())
    }
}

#[instrument]
pub fn validate_address(address: &str) -> AppResult<()> {
    if address.is_empty() {
        return Err(AppError::Message("network address cannot be empty".into()));
    }
    if !address.contains(':') {
        return Err(AppError::Message(format!(
            "network address '{address}' must include a port"
        )));
    }
    Ok(())
}

#[instrument]
pub fn config_from_json(json: &str) -> AppResult<EndpointConfig> {
    let config: EndpointConfig = serde_json::from_str(json)
        .map_err(|err| AppError::Message(format!("invalid endpoint config: {err}")))?;
    validate_address(&config.address)?;
    Ok(config)
}
