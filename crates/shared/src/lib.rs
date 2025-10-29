#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

mod error;

use std::sync::OnceLock;

use tracing_subscriber::{fmt, EnvFilter};

pub use crate::error::{AppError, AppResult};

static TRACING_GUARD: OnceLock<()> = OnceLock::new();

/// Initialise global tracing subscriber with sensible defaults.
pub fn init_tracing() -> AppResult<()> {
    TRACING_GUARD
        .get_or_try_init(|| {
            let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
            fmt()
                .with_env_filter(filter)
                .try_init()
                .map_err(|err| AppError::Message(format!("failed to initialise tracing: {err}")))?;
            Ok(())
        })
        .map(|_| ())
}

/// Yield the async runtime, ensuring a Tokio dependency when the runtime feature is enabled.
#[cfg(feature = "runtime")]
pub async fn yield_control() {
    tokio::task::yield_now().await;
}

/// Provide a canonical descriptor for the Media Foundation pipeline being used.
#[cfg(feature = "media-foundation")]
pub fn media_foundation_descriptor() -> &'static str {
    "Windows Media Foundation (placeholder)"
}

/// Represent the UI theme used across binaries.
#[cfg(feature = "ui")]
pub mod ui {
    use eframe::egui::{self, CentralPanel};

    pub fn draw_placeholder(ctx: &egui::Context) {
        CentralPanel::default().show(ctx, |ui| {
            ui.heading("Receiver UI");
            ui.label("UI components to be implemented.");
        });
    }

    pub fn accent_colour() -> egui::Color32 {
        egui::Color32::from_rgb(52, 152, 219)
    }
}
