use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, CentralPanel, Color32, Layout, SidePanel, TopBottomPanel};
use eframe::egui::plot::{Line, Legend, Plot, PlotPoints};
use eframe::egui::widgets::ProgressBar;
use shared::{AppError, AppResult};
use tokio::sync::broadcast;
use tokio::sync::mpsc::{self, error::TryRecvError, UnboundedReceiver, UnboundedSender};
use tokio::sync::Notify;

use crate::config::RenderSettings;
use crate::metrics::MetricEvent;
use crate::pipeline::{ControlEvent, PipelineCounters, PipelineStatus};

#[derive(Debug, Clone)]
pub struct BufferingPresetConfig {
    pub label: String,
    pub max_frames: usize,
    pub max_latency_ms: u64,
}

impl BufferingPresetConfig {
    pub fn new(label: impl Into<String>, max_frames: usize, max_latency_ms: u64) -> Self {
        Self {
            label: label.into(),
            max_frames: max_frames.max(1),
            max_latency_ms: max_latency_ms.max(1),
        }
    }
}

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
    control_tx: broadcast::Sender<ControlEvent>,
    status_rx: mpsc::UnboundedReceiver<PipelineStatus>,
    sender_presets: Vec<String>,
    buffering_presets: Vec<BufferingPresetConfig>,
) -> AppResult<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(egui::vec2(settings.width as f32, settings.height as f32))
            .with_title(settings.window_title.clone())
            .with_vsync(settings.vsync),
        ..Default::default()
    };

    let mut app = ReceiverApp::new(
        frame_rx,
        metrics_tx,
        counters,
        running.clone(),
        shutdown.clone(),
        settings,
        frame_limit,
        control_tx,
        status_rx,
        sender_presets,
        buffering_presets,
    );

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
    control_tx: broadcast::Sender<ControlEvent>,
    status_rx: mpsc::UnboundedReceiver<PipelineStatus>,
    sender_presets: Vec<String>,
    buffering_presets: Vec<BufferingPresetConfig>,
    texture: Option<egui::TextureHandle>,
    latest_latency: Option<Duration>,
    latest_queue_latency: Option<Duration>,
    latest_decode_latency: Option<Duration>,
    queue_depth: usize,
    last_frame_id: Option<u64>,
    last_session_id: Option<u64>,
    dropped_total: u64,
    presented: u64,
    frame_metrics: VecDeque<FrameMetrics>,
    metrics_capacity: usize,
    session_start: Instant,
    last_present_instant: Option<Instant>,
    target_refresh: f64,
    connection_state: ConnectionState,
    selected_sender_index: usize,
    pending_sender_index: usize,
    selected_buffering_index: usize,
    last_buffering_sent: Option<usize>,
    last_sender_sent: Option<String>,
}

#[derive(Debug, Clone)]
struct FrameMetrics {
    timestamp: Instant,
    fps: f64,
    latency_ms: f64,
    queue_ms: f64,
    decode_ms: f64,
}

#[derive(Debug, Clone)]
enum ConnectionState {
    Idle,
    Connecting { address: String },
    Connected { address: String },
    Failed { address: String, error: String },
    Disconnected,
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
        control_tx: broadcast::Sender<ControlEvent>,
        status_rx: mpsc::UnboundedReceiver<PipelineStatus>,
        sender_presets: Vec<String>,
        buffering_presets: Vec<BufferingPresetConfig>,
    ) -> Self {
        let metrics_capacity = 360;
        let session_start = Instant::now();
        let target_refresh = if settings.refresh_rate == 0 {
            60.0
        } else {
            settings.refresh_rate as f64
        };

        let selected_sender_index = 0;
        let pending_sender_index = 0;
        let connection_state = sender_presets
            .get(0)
            .cloned()
            .map(|address| ConnectionState::Connecting { address })
            .unwrap_or(ConnectionState::Idle);

        let selected_buffering_index = 0;

        let mut app = Self {
            frame_rx,
            metrics_tx,
            counters,
            running,
            shutdown,
            settings,
            frame_limit,
            control_tx,
            status_rx,
            sender_presets,
            buffering_presets,
            texture: None,
            latest_latency: None,
            latest_queue_latency: None,
            latest_decode_latency: None,
            queue_depth: 0,
            last_frame_id: None,
            last_session_id: None,
            dropped_total: 0,
            presented: 0,
            frame_metrics: VecDeque::with_capacity(metrics_capacity),
            metrics_capacity,
            session_start,
            last_present_instant: None,
            target_refresh,
            connection_state,
            selected_sender_index,
            pending_sender_index,
            selected_buffering_index,
            last_buffering_sent: None,
            last_sender_sent: None,
        };

        if !app.buffering_presets.is_empty() {
            app.apply_buffering_preset(app.selected_buffering_index);
        }

        app
    }

    fn poll_frames(&mut self, ctx: &egui::Context) {
        loop {
            match self.frame_rx.try_recv() {
                Ok(frame) => self.show_frame(ctx, frame),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn poll_status(&mut self) {
        loop {
            match self.status_rx.try_recv() {
                Ok(status) => self.handle_status(status),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.connection_state = ConnectionState::Disconnected;
                    break;
                }
            }
        }
    }

    fn handle_status(&mut self, status: PipelineStatus) {
        match status {
            PipelineStatus::Connecting { address } => {
                self.connection_state = ConnectionState::Connecting {
                    address: address.clone(),
                };
                if let Some(idx) = self.sender_presets.iter().position(|s| s == &address) {
                    self.pending_sender_index = idx;
                }
            }
            PipelineStatus::Connected { address } => {
                if let Some(idx) = self.sender_presets.iter().position(|s| s == &address) {
                    self.selected_sender_index = idx;
                    self.pending_sender_index = idx;
                }
                self.connection_state = ConnectionState::Connected { address: address.clone() };
                self.last_sender_sent = Some(address);
            }
            PipelineStatus::ConnectionFailed { address, error } => {
                self.connection_state = ConnectionState::Failed { address: address.clone(), error };
                self.last_sender_sent = None;
            }
            PipelineStatus::Disconnected => {
                self.connection_state = ConnectionState::Disconnected;
                self.last_sender_sent = None;
            }
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

        let now = Instant::now();
        let presentation_latency = SystemTime::now()
            .duration_since(frame.timestamp)
            .unwrap_or_else(|_| Duration::from_millis(0));

        self.update_metrics(now, presentation_latency, frame.queue_latency, frame.decode_latency);

        self.queue_depth = frame.queue_depth;
        self.last_frame_id = Some(frame.frame_id);
        self.last_session_id = Some(frame.session_id);
        self.dropped_total = frame.dropped_total;
        self.presented = self.presented.saturating_add(1);

        let _ = self.metrics_tx.send(MetricEvent::FramePresented {
            latency: presentation_latency,
            queue_latency: frame.queue_latency,
            decode_latency: frame.decode_latency,
            queue_depth: frame.queue_depth,
            dropped_total: frame.dropped_total,
        });

        self.counters.record_rendered(presentation_latency);
        ctx.request_repaint();

        if let Some(limit) = self.frame_limit {
            if self.presented >= limit {
                self.running.store(false, Ordering::SeqCst);
                self.shutdown.notify_waiters();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn update_metrics(
        &mut self,
        now: Instant,
        latency: Duration,
        queue_latency: Duration,
        decode_latency: Duration,
    ) {
        let fps = if let Some(last) = self.last_present_instant {
            let delta = now.saturating_duration_since(last).as_secs_f64();
            if delta > f64::EPSILON {
                1.0 / delta
            } else {
                self.target_refresh
            }
        } else {
            self.target_refresh
        };
        self.last_present_instant = Some(now);

        let metrics = FrameMetrics {
            timestamp: now,
            fps,
            latency_ms: latency.as_secs_f64() * 1_000.0,
            queue_ms: queue_latency.as_secs_f64() * 1_000.0,
            decode_ms: decode_latency.as_secs_f64() * 1_000.0,
        };

        self.latest_latency = Some(latency);
        self.latest_queue_latency = Some(queue_latency);
        self.latest_decode_latency = Some(decode_latency);

        self.frame_metrics.push_back(metrics);
        if self.frame_metrics.len() > self.metrics_capacity {
            self.frame_metrics.pop_front();
        }
    }

    fn render_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Receiver");
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, color) = self.connection_status_text();
                ui.colored_label(color, text);
            });
        });

        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label("Sender");
            let current = self
                .sender_presets
                .get(self.pending_sender_index)
                .cloned()
                .unwrap_or_else(|| "(none)".to_string());
            egui::ComboBox::from_id_source("sender_select")
                .width(220.0)
                .selected_text(current.clone())
                .show_ui(ui, |ui| {
                    for (idx, preset) in self.sender_presets.iter().enumerate() {
                        ui.selectable_value(&mut self.pending_sender_index, idx, preset);
                    }
                });
            let connect_label = if matches!(self.connection_state, ConnectionState::Connecting { .. }) {
                "Connecting"
            } else {
                "Connect"
            };
            if ui.button(connect_label).clicked() {
                self.request_sender_switch(self.pending_sender_index);
            }
        });

        if let ConnectionState::Failed { address: _, error } = &self.connection_state {
            ui.add_space(4.0);
            ui.colored_label(Color32::from_rgb(231, 76, 60), format!("Connection failed: {error}"));
        }

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Buffering");
            if self.buffering_presets.is_empty() {
                ui.weak("No presets available");
            } else {
                for (idx, preset) in self.buffering_presets.iter().enumerate() {
                    let selected = self.selected_buffering_index == idx;
                    if ui
                        .selectable_label(selected, &preset.label)
                        .clicked()
                        && !selected
                    {
                        self.selected_buffering_index = idx;
                        self.apply_buffering_preset(idx);
                    }
                }
            }
        });
    }

    fn render_metrics_panel(&self, ui: &mut egui::Ui) {
        ui.heading("Metrics");
        ui.add_space(6.0);

        if let Some(current) = self.frame_metrics.back() {
            ui.label(format!("Frames presented: {}", self.presented));
            ui.label(format!("Frames dropped: {}", self.dropped_total));

            if self.target_refresh > 0.0 {
                let ratio = (current.fps / self.target_refresh).clamp(0.0, 2.0);
                ui.label(format!("Instant FPS: {:.1}", current.fps));
                ui.add(
                    ProgressBar::new((ratio / 2.0) as f32)
                        .text(format!("{:.0}%", (current.fps / self.target_refresh * 100.0).clamp(0.0, 200.0))),
                );
            } else {
                ui.label(format!("Instant FPS: {:.1}", current.fps));
            }

            if let Some(preset) = self.buffering_presets.get(self.selected_buffering_index) {
                let budget = preset.max_latency_ms as f64;
                let ratio = (current.latency_ms / budget).clamp(0.0, 2.0);
                ui.label(format!("Latency: {:.1} ms", current.latency_ms));
                ui.add(
                    ProgressBar::new((ratio / 2.0) as f32)
                        .text(format!("{:.0}%", (current.latency_ms / budget * 100.0).clamp(0.0, 200.0))),
                );
            } else {
                ui.label(format!("Latency: {:.1} ms", current.latency_ms));
            }

            ui.label(format!("Queue depth: {}", self.queue_depth));
            ui.label(format!("Queue latency: {:.1} ms", current.queue_ms));
            ui.label(format!("Decode latency: {:.1} ms", current.decode_ms));
        } else {
            ui.label("Waiting for metrics...");
        }

        ui.add_space(8.0);

        if self.frame_metrics.len() > 1 {
            let latency_points = self.metrics_points(|m| m.latency_ms);
            let queue_points = self.metrics_points(|m| m.queue_ms);
            let decode_points = self.metrics_points(|m| m.decode_ms);

            Plot::new("latency_plot")
                .allow_scroll(false)
                .allow_zoom(false)
                .legend(Legend::default())
                .show(ui, |plot_ui| {
                    plot_ui.line(
                        Line::new(latency_points).color(Color32::from_rgb(231, 76, 60)).name("Latency ms"),
                    );
                    plot_ui.line(
                        Line::new(queue_points).color(Color32::from_rgb(52, 152, 219)).name("Queue ms"),
                    );
                    plot_ui.line(
                        Line::new(decode_points).color(Color32::from_rgb(46, 204, 113)).name("Decode ms"),
                    );
                });
        }
    }

    fn render_video(&mut self, ui: &mut egui::Ui) {
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
                match &self.connection_state {
                    ConnectionState::Connecting { address } => {
                        ui.heading("Connecting to sender...");
                        ui.label(address);
                    }
                    ConnectionState::Failed { address, error } => {
                        ui.heading("Connection failed");
                        ui.label(address);
                        ui.colored_label(Color32::from_rgb(231, 76, 60), error);
                    }
                    ConnectionState::Disconnected => {
                        ui.heading("Disconnected");
                        ui.label("Reconnect to resume playback.");
                    }
                    _ => {
                        ui.heading("Waiting for frames...");
                        ui.label("Ensure the sender is streaming to the configured endpoint.");
                    }
                }
            });
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        let mut lines = Vec::new();
        if let Some(latency) = self.latest_latency {
            lines.push(format!("End-to-end latency: {:.2} ms", latency.as_secs_f64() * 1_000.0));
        }
        if let Some(latency) = self.latest_queue_latency {
            lines.push(format!("Queue latency: {:.2} ms", latency.as_secs_f64() * 1_000.0));
        }
        if let Some(latency) = self.latest_decode_latency {
            lines.push(format!("Decode latency: {:.2} ms", latency.as_secs_f64() * 1_000.0));
        }
        lines.push(format!("Queue depth: {}", self.queue_depth));
        lines.push(format!("Frames presented: {}", self.presented));
        lines.push(format!("Frames dropped: {}", self.dropped_total));
        if let Some(frame_id) = self.last_frame_id {
            lines.push(format!("Last frame ID: {}", frame_id));
        }
        if let Some(session_id) = self.last_session_id {
            lines.push(format!("Session ID: {}", session_id));
        }

        for line in lines {
            ui.label(line);
        }
    }

    fn request_sender_switch(&mut self, index: usize) {
        if let Some(address) = self.sender_presets.get(index).cloned() {
            if self.last_sender_sent.as_deref() == Some(address.as_str()) {
                return;
            }
            let _ = self.control_tx.send(ControlEvent::SelectSender { address: address.clone() });
            self.last_sender_sent = Some(address.clone());
            self.connection_state = ConnectionState::Connecting { address };
            self.selected_sender_index = index;
            self.pending_sender_index = index;
        }
    }

    fn apply_buffering_preset(&mut self, index: usize) {
        if self.last_buffering_sent == Some(index) {
            return;
        }
        if let Some(preset) = self.buffering_presets.get(index) {
            let _ = self.control_tx.send(ControlEvent::SetBuffering {
                max_frames: preset.max_frames,
                max_latency_ms: preset.max_latency_ms,
            });
            self.last_buffering_sent = Some(index);
        }
    }

    fn connection_status_text(&self) -> (String, Color32) {
        match &self.connection_state {
            ConnectionState::Idle => ("Idle".to_string(), Color32::from_gray(160)),
            ConnectionState::Connecting { address } => (
                format!("Connecting to {address}"),
                Color32::from_rgb(243, 156, 18),
            ),
            ConnectionState::Connected { address } => (
                format!("Connected to {address}"),
                Color32::from_rgb(46, 204, 113),
            ),
            ConnectionState::Failed { address, error } => (
                format!("Failed to {address}: {error}"),
                Color32::from_rgb(231, 76, 60),
            ),
            ConnectionState::Disconnected => (
                "Disconnected".to_string(),
                Color32::from_rgb(231, 76, 60),
            ),
        }
    }

    fn metrics_points<F>(&self, map: F) -> PlotPoints
    where
        F: Fn(&FrameMetrics) -> f64,
    {
        let start = self.session_start;
        let points: Vec<[f64; 2]> = self
            .frame_metrics
            .iter()
            .map(|m| {
                let x = m.timestamp.saturating_duration_since(start).as_secs_f64();
                [x, map(m)]
            })
            .collect();
        PlotPoints::from(points)
    }
}

impl eframe::App for ReceiverApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.running.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        self.poll_status();
        self.poll_frames(ctx);

        TopBottomPanel::top("receiver_controls").show(ctx, |ui| {
            self.render_controls(ui);
        });

        SidePanel::right("receiver_metrics")
            .min_width(260.0)
            .default_width(280.0)
            .show(ctx, |ui| {
                self.render_metrics_panel(ui);
            });

        CentralPanel::default().show(ctx, |ui| {
            self.render_video(ui);
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
