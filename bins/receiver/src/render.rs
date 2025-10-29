use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use eframe::egui::{self, CentralPanel};
use shared::{AppError, AppResult};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::Notify;

use crate::config::RenderSettings;
use crate::metrics::MetricEvent;
use crate::pipeline::PipelineCounters;

#[derive(Debug)]
pub struct RenderFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: SystemTime,
    pub arrival: SystemTime,
    pub frame_id: u64,
    pub session_id: u64,
    pub decode_latency: Duration,
    pub queue_latency: Duration,
    pub queue_depth: usize,
    pub dropped_total: u64,
}

pub fn run_renderer(
    frame_rx: UnboundedReceiver<RenderFrame>,
    metrics_tx: UnboundedSender<MetricEvent>,
    counters: Arc<PipelineCounters>,
    running: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
    settings: RenderSettings,
    frame_limit: Option<u64>,
) -> AppResult<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(egui::vec2(settings.width as f32, settings.height as f32))
            .with_title(settings.window_title.clone())
            .with_vsync(settings.vsync),
        ..Default::default()
    };

    let mut app = ReceiverApp::new(frame_rx, metrics_tx, counters, running.clone(), shutdown.clone(), settings, frame_limit);

    eframe::run_native(
        "receiver",
        options,
        Box::new(|_cc| Box::new(app)),
    )
    .map_err(|err| AppError::Message(format!("receiver ui failed: {err}")))?;

    running.store(false, Ordering::SeqCst);
    shutdown.notify_waiters();
    Ok(())
}

struct ReceiverApp {
    frame_rx: UnboundedReceiver<RenderFrame>,
    metrics_tx: UnboundedSender<MetricEvent>,
    counters: Arc<PipelineCounters>,
    running: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
    settings: RenderSettings,
    frame_limit: Option<u64>,
    texture: Option<egui::TextureHandle>,
    latest_latency: Option<Duration>,
    latest_queue_latency: Option<Duration>,
    latest_decode_latency: Option<Duration>,
    queue_depth: usize,
    last_frame_id: Option<u64>,
    last_session_id: Option<u64>,
    dropped_total: u64,
    presented: u64,
}

impl ReceiverApp {
    fn new(
        frame_rx: UnboundedReceiver<RenderFrame>,
        metrics_tx: UnboundedSender<MetricEvent>,
        counters: Arc<PipelineCounters>,
        running: Arc<AtomicBool>,
        shutdown: Arc<Notify>,
        settings: RenderSettings,
        frame_limit: Option<u64>,
    ) -> Self {
        Self {
            frame_rx,
            metrics_tx,
            counters,
            running,
            shutdown,
            settings,
            frame_limit,
            texture: None,
            latest_latency: None,
            latest_queue_latency: None,
            latest_decode_latency: None,
            queue_depth: 0,
            last_frame_id: None,
            last_session_id: None,
            dropped_total: 0,
            presented: 0,
        }
    }

    fn poll_frames(&mut self, ctx: &egui::Context) {
        while let Ok(frame) = self.frame_rx.try_recv() {
            self.show_frame(ctx, frame);
        }
    }

    fn show_frame(&mut self, ctx: &egui::Context, frame: RenderFrame) {
        let rgba = bgra_to_rgba(&frame.pixels);
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width as usize, frame.height as usize],
            &rgba,
        );

        if let Some(texture) = &self.texture {
            texture.set(image, egui::TextureOptions::LINEAR);
        } else {
            let texture = ctx.load_texture("receiver-frame", image, egui::TextureOptions::LINEAR);
            self.texture = Some(texture);
        }

        let now = SystemTime::now();
        let latency = now
            .duration_since(frame.timestamp)
            .unwrap_or_else(|_| Duration::from_millis(0));

        self.latest_latency = Some(latency);
        self.latest_queue_latency = Some(frame.queue_latency);
        self.latest_decode_latency = Some(frame.decode_latency);
        self.queue_depth = frame.queue_depth;
        self.last_frame_id = Some(frame.frame_id);
        self.last_session_id = Some(frame.session_id);
        self.dropped_total = frame.dropped_total;
        self.presented = self.presented.saturating_add(1);

        let _ = self.metrics_tx.send(MetricEvent::FramePresented {
            latency,
            queue_latency: frame.queue_latency,
            decode_latency: frame.decode_latency,
            queue_depth: frame.queue_depth,
            dropped_total: frame.dropped_total,
        });

        self.counters.record_rendered(latency);
        ctx.request_repaint();

        if let Some(limit) = self.frame_limit {
            if self.presented >= limit {
                self.running.store(false, Ordering::SeqCst);
                self.shutdown.notify_waiters();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

impl eframe::App for ReceiverApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.running.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        self.poll_frames(ctx);

        CentralPanel::default().show(ctx, |ui| {
            if let Some(texture) = &self.texture {
                let available = ui.available_size_before_wrap_finite();
                let original = texture.size_vec2();
                let scale_x = if original.x > 0.0 { available.x / original.x } else { 1.0 };
                let scale_y = if original.y > 0.0 { available.y / original.y } else { 1.0 };
                let scale = scale_x.min(scale_y).min(1.0).max(0.1);
                let display = egui::vec2(original.x * scale, original.y * scale);
                ui.image(texture, display);
            } else {
                ui.vertical_centered(|ui| {
                    ui.heading("Waiting for frames...");
                    ui.add_space(8.0);
                    ui.label("Ensure the sender is streaming to the configured UDP endpoint.");
                });
            }

            ui.separator();

            if let Some(latency) = self.latest_latency {
                ui.label(format!(
                    "End-to-end latency: {:.2} ms",
                    latency.as_secs_f64() * 1_000.0
                ));
            } else {
                ui.label("End-to-end latency: n/a");
            }

            if let Some(latency) = self.latest_queue_latency {
                ui.label(format!("Queue latency: {:.2} ms", latency.as_secs_f64() * 1_000.0));
            } else {
                ui.label("Queue latency: n/a");
            }

            if let Some(latency) = self.latest_decode_latency {
                ui.label(format!("Decode latency: {:.2} ms", latency.as_secs_f64() * 1_000.0));
            } else {
                ui.label("Decode latency: n/a");
            }

            ui.label(format!("Queue depth: {}", self.queue_depth));
            ui.label(format!("Frames presented: {}", self.presented));
            ui.label(format!("Frames dropped: {}", self.dropped_total));

            if let Some(frame_id) = self.last_frame_id {
                ui.label(format!("Last frame ID: {}", frame_id));
            }
            if let Some(session_id) = self.last_session_id {
                ui.label(format!("Session ID: {}", session_id));
            }
        });

        ctx.request_repaint_after(self.settings.present_interval());
    }

    fn on_close_event(&mut self) -> bool {
        self.running.store(false, Ordering::SeqCst);
        self.shutdown.notify_waiters();
        true
    }
}

impl Drop for ReceiverApp {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.shutdown.notify_waiters();
    }
}

fn bgra_to_rgba(bytes: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(bytes.len());
    for chunk in bytes.chunks_exact(4) {
        rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
    }
    rgba
}
