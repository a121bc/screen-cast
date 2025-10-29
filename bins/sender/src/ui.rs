#![cfg(target_os = "windows")]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout, RichText, ScrollArea, TextEdit};
use eframe::egui::plot::{Line, Plot, PlotPoints};
use eframe::{App, CreationContext, Frame, NativeOptions};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::task::JoinHandle;
use tracing::error;

use crate::config::{BitratePreset, ScalingMethod, ScalingSettings, SenderConfig};
use crate::{PipelineCallbacks, PipelineEvent, PipelineReport, SenderPipeline};
use crate::metrics::MetricsSnapshot;
use shared::{AppError, AppResult};

const MAX_HISTORY_SAMPLES: usize = 240;
const STATUS_HISTORY: usize = 32;
const ACTIVE_REPAINT_MS: u64 = 16;
const IDLE_REPAINT_MS: u64 = 250;

#[derive(Clone, Copy)]
struct ResolutionPreset {
    label: &'static str,
    width: u32,
    height: u32,
}

const RESOLUTION_PRESETS: [ResolutionPreset; 3] = [
    ResolutionPreset {
        label: "1280 × 720",
        width: 1280,
        height: 720,
    },
    ResolutionPreset {
        label: "1600 × 900",
        width: 1600,
        height: 900,
    },
    ResolutionPreset {
        label: "1920 × 1080",
        width: 1920,
        height: 1080,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolutionSelection {
    Native,
    Preset(usize),
    Custom,
}

#[derive(Debug, Clone)]
struct MetricSample {
    elapsed: f64,
    fps: f64,
    avg_ms: f64,
    max_ms: f64,
}

#[derive(Debug, Clone)]
struct StatusEntry {
    level: StatusLevel,
    message: String,
    timestamp: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipelineState {
    Idle,
    Starting,
    Running,
    Error,
}

pub fn run_gui(config: SenderConfig) -> AppResult<()> {
    let mut runtime = Some(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|err| AppError::Message(format!("failed to initialise runtime: {err}")))?,
    );
    let mut initial_config = Some(config);

    let mut options = NativeOptions::default();
    options.viewport = egui::ViewportBuilder::default()
        .with_inner_size(egui::vec2(960.0, 640.0))
        .with_min_inner_size(egui::vec2(720.0, 480.0));

    eframe::run_native(
        "Sender Control",
        options,
        Box::new(move |cc| {
            let runtime = runtime.take().expect("runtime initialised once");
            let config = initial_config
                .take()
                .expect("configuration available at startup");
            Box::new(SenderApp::new(cc, runtime, config))
        }),
    )
    .map_err(|err| AppError::Message(format!("failed to launch sender GUI: {err}")))
}

struct SenderApp {
    runtime: Runtime,
    config: SenderConfig,
    pipeline_handle: Option<JoinHandle<()>>,
    metrics_rx: Option<UnboundedReceiver<MetricsSnapshot>>,
    status_rx: Option<UnboundedReceiver<PipelineEvent>>,
    metrics_history: VecDeque<MetricSample>,
    metrics_origin: Option<Instant>,
    latest_metrics: Option<MetricsSnapshot>,
    last_report: Option<PipelineReport>,
    status_log: VecDeque<StatusEntry>,
    start_time: Instant,
    pipeline_state: PipelineState,
    address_input: String,
    resolution_selection: ResolutionSelection,
    custom_resolution_input: String,
    bitrate_preset: BitratePreset,
    custom_bitrate_input: String,
}

impl SenderApp {
    fn new(cc: &CreationContext<'_>, runtime: Runtime, config: SenderConfig) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let mut app = Self {
            runtime,
            config: config.clone(),
            pipeline_handle: None,
            metrics_rx: None,
            status_rx: None,
            metrics_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            metrics_origin: None,
            latest_metrics: None,
            last_report: None,
            status_log: VecDeque::with_capacity(STATUS_HISTORY),
            start_time: Instant::now(),
            pipeline_state: PipelineState::Idle,
            address_input: config.network.address.clone(),
            resolution_selection: ResolutionSelection::Native,
            custom_resolution_input: String::new(),
            bitrate_preset: config.encoder.preset,
            custom_bitrate_input: String::new(),
        };

        if let Some(bitrate) = config.encoder.bitrate {
            app.bitrate_preset = BitratePreset::Custom;
            app.custom_bitrate_input = bitrate.to_string();
        }

        if let Some(scaling) = config.scaling.as_ref() {
            if let Some(index) = RESOLUTION_PRESETS
                .iter()
                .position(|preset| preset.width == scaling.width && preset.height == scaling.height)
            {
                app.resolution_selection = ResolutionSelection::Preset(index);
            } else {
                app.resolution_selection = ResolutionSelection::Custom;
                app.custom_resolution_input = format!("{}x{}", scaling.width, scaling.height);
            }
        }

        app.push_status(StatusLevel::Info, "Ready to start capture".to_string());
        app
    }

    fn is_pipeline_active(&self) -> bool {
        self.pipeline_handle.is_some()
    }

    fn update_channels(&mut self) {
        self.drain_status_events();
        self.drain_metric_events();
    }

    fn drain_status_events(&mut self) {
        if let Some(receiver) = self.status_rx.as_mut() {
            loop {
                match receiver.try_recv() {
                    Ok(event) => self.handle_pipeline_event(event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.status_rx = None;
                        break;
                    }
                }
            }
        }
    }

    fn drain_metric_events(&mut self) {
        if let Some(receiver) = self.metrics_rx.as_mut() {
            loop {
                match receiver.try_recv() {
                    Ok(snapshot) => self.record_metrics(snapshot),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.metrics_rx = None;
                        break;
                    }
                }
            }
        }
    }

    fn record_metrics(&mut self, snapshot: MetricsSnapshot) {
        let now = Instant::now();
        let origin = self.metrics_origin.get_or_insert(now);
        let elapsed = now
            .checked_duration_since(*origin)
            .unwrap_or_default()
            .as_secs_f64();

        if self.metrics_history.len() == MAX_HISTORY_SAMPLES {
            self.metrics_history.pop_front();
        }
        self.metrics_history.push_back(MetricSample {
            elapsed,
            fps: snapshot.fps,
            avg_ms: snapshot.avg_encode_ms,
            max_ms: snapshot.max_encode_ms,
        });
        self.latest_metrics = Some(snapshot);
    }

    fn handle_pipeline_event(&mut self, event: PipelineEvent) {
        match event {
            PipelineEvent::Started => {
                self.pipeline_state = PipelineState::Running;
                self.push_status(StatusLevel::Info, "Pipeline started".to_string());
            }
            PipelineEvent::CaptureWarning { message } => {
                self.push_status(StatusLevel::Warning, message);
            }
            PipelineEvent::Error { message } => {
                self.pipeline_state = PipelineState::Error;
                self.push_status(StatusLevel::Error, message);
                self.pipeline_handle = None;
            }
            PipelineEvent::Stopped(report) => {
                self.pipeline_state = PipelineState::Idle;
                self.pipeline_handle = None;
                self.last_report = Some(report);
                self.push_status(
                    StatusLevel::Info,
                    format!(
                        "Capture complete – {} frames, {} capture errors, {} dropped",
                        report.frames_transmitted, report.capture_errors, report.dropped_frames
                    ),
                );
            }
        }
    }

    fn push_status(&mut self, level: StatusLevel, message: String) {
        if self.status_log.len() == STATUS_HISTORY {
            self.status_log.pop_front();
        }
        self.status_log.push_back(StatusEntry {
            level,
            message,
            timestamp: Instant::now(),
        });
    }

    fn start_pipeline(&mut self) {
        if self.is_pipeline_active() {
            return;
        }

        let mut config = self.config.clone();
        config.network.address = self.address_input.trim().to_string();

        match self.resolution_selection {
            ResolutionSelection::Native => {
                config.scaling = None;
            }
            ResolutionSelection::Preset(index) => {
                if let Some(preset) = RESOLUTION_PRESETS.get(index) {
                    config.scaling = Some(ScalingSettings {
                        width: preset.width,
                        height: preset.height,
                        method: ScalingMethod::Software,
                    });
                }
            }
            ResolutionSelection::Custom => match parse_resolution(&self.custom_resolution_input) {
                Ok((width, height)) => {
                    config.scaling = Some(ScalingSettings {
                        width,
                        height,
                        method: ScalingMethod::Software,
                    });
                }
                Err(err) => {
                    self.push_status(StatusLevel::Error, format!("Invalid resolution: {err}"));
                    return;
                }
            },
        }

        config.encoder.preset = self.bitrate_preset;
        if self.bitrate_preset == BitratePreset::Custom {
            match self.custom_bitrate_input.trim().parse::<u32>() {
                Ok(value) if value > 0 => {
                    config.encoder.bitrate = Some(value);
                }
                _ => {
                    self.push_status(
                        StatusLevel::Error,
                        "Provide a positive bitrate when using the custom preset".into(),
                    );
                    return;
                }
            }
        } else {
            config.encoder.bitrate = None;
        }

        if let Err(err) = config.validate() {
            self.push_status(StatusLevel::Error, format!("Configuration error: {err}"));
            return;
        }

        let (metrics_tx, metrics_rx) = mpsc::unbounded_channel();
        let (status_tx, status_rx) = mpsc::unbounded_channel();

        self.metrics_rx = Some(metrics_rx);
        self.status_rx = Some(status_rx);
        self.metrics_history.clear();
        self.metrics_origin = None;
        self.latest_metrics = None;
        self.last_report = None;

        let callbacks = PipelineCallbacks {
            metrics_sender: Some(metrics_tx),
            status_sender: Some(status_tx.clone()),
        };
        let pipeline_config = config.clone();

        let handle = self.runtime.spawn(async move {
            let pipeline = SenderPipeline::new(pipeline_config);
            if let Err(err) = pipeline.run_with_callbacks(callbacks).await {
                error!(error = %err, "sender pipeline terminated with error");
            }
        });

        self.pipeline_handle = Some(handle);
        self.pipeline_state = PipelineState::Starting;
        self.config = config;
        self.push_status(StatusLevel::Info, "Starting capture pipeline…".to_string());
    }

    fn stop_pipeline(&mut self) {
        if let Some(handle) = self.pipeline_handle.take() {
            handle.abort();
            self.push_status(StatusLevel::Info, "Stop requested".to_string());
        }
        self.pipeline_state = PipelineState::Idle;
    }

    fn cleanup_finished_handle(&mut self) {
        if let Some(handle) = &self.pipeline_handle {
            if handle.is_finished() {
                self.pipeline_handle = None;
            }
        }
    }

    fn draw_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Capture control");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let start_enabled = !self.is_pipeline_active();
            let start_button = egui::Button::new(RichText::new("Start capture").color(Color32::WHITE))
                .fill(shared::ui::accent_colour());
            if ui.add_enabled(start_enabled, start_button).clicked() {
                self.start_pipeline();
            }

            let stop_button = egui::Button::new("Stop capture").fill(Color32::from_rgb(192, 57, 43));
            if ui
                .add_enabled(self.is_pipeline_active(), stop_button)
                .clicked()
            {
                self.stop_pipeline();
            }
        });

        ui.add_space(10.0);
        ui.label("Network target");
        ui.add(TextEdit::singleline(&mut self.address_input).hint_text("host:port"));

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Resolution");
            egui::ComboBox::from_id_source("sender_resolution_combo")
                .selected_text(self.current_resolution_label())
                .show_ui(ui, |combo| {
                    combo.selectable_value(
                        &mut self.resolution_selection,
                        ResolutionSelection::Native,
                        "Native (no scaling)",
                    );
                    for (index, preset) in RESOLUTION_PRESETS.iter().enumerate() {
                        let label = format!("{}", preset.label);
                        combo.selectable_value(
                            &mut self.resolution_selection,
                            ResolutionSelection::Preset(index),
                            label,
                        );
                    }
                    combo.selectable_value(&mut self.resolution_selection, ResolutionSelection::Custom, "Custom");
                });
        });

        if matches!(self.resolution_selection, ResolutionSelection::Custom) {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Custom");
                ui.add(
                    TextEdit::singleline(&mut self.custom_resolution_input)
                        .hint_text("WIDTHxHEIGHT"),
                );
            });
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Bitrate preset");
            egui::ComboBox::from_id_source("sender_bitrate_combo")
                .selected_text(self.current_bitrate_label())
                .show_ui(ui, |combo| {
                    for preset in [
                        BitratePreset::Low,
                        BitratePreset::Medium,
                        BitratePreset::High,
                        BitratePreset::Custom,
                    ] {
                        combo.selectable_value(&mut self.bitrate_preset, preset, preset_label(preset));
                    }
                });
        });

        if self.bitrate_preset == BitratePreset::Custom {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Bitrate (bps)");
                ui.add(
                    TextEdit::singleline(&mut self.custom_bitrate_input)
                        .hint_text("e.g. 12000000"),
                );
            });
        }

        ui.add_space(8.0);
        ui.label(format!("State: {}", self.pipeline_state.as_str()));
    }

    fn draw_metrics(&mut self, ui: &mut egui::Ui) {
        ui.heading("Performance");
        if let Some(snapshot) = self.latest_metrics.as_ref() {
            ui.label(format!(
                "FPS: {:.1} | Avg encode: {:.2} ms | Max encode: {:.2} ms",
                snapshot.fps, snapshot.avg_encode_ms, snapshot.max_encode_ms
            ));
        } else {
            ui.label("Metrics will appear after the first capture window.");
        }

        if self.metrics_history.is_empty() {
            ui.label("Waiting for samples…");
            return;
        }

        let fps_points: PlotPoints = self
            .metrics_history
            .iter()
            .map(|sample| [sample.elapsed, sample.fps])
            .collect();
        let avg_points: PlotPoints = self
            .metrics_history
            .iter()
            .map(|sample| [sample.elapsed, sample.avg_ms])
            .collect();
        let max_points: PlotPoints = self
            .metrics_history
            .iter()
            .map(|sample| [sample.elapsed, sample.max_ms])
            .collect();

        Plot::new("fps_plot")
            .view_aspect(2.0)
            .legend(egui::plot::Legend::default())
            .y_axis_label("Frames per second")
            .show(ui, |plot_ui| {
                plot_ui.line(Line::new(fps_points).name("FPS"));
            });

        Plot::new("latency_plot")
            .view_aspect(2.0)
            .legend(egui::plot::Legend::default())
            .y_axis_label("Encode latency (ms)")
            .show(ui, |plot_ui| {
                plot_ui.line(Line::new(avg_points).name("Average"));
                plot_ui.line(Line::new(max_points).name("Max"));
            });
    }

    fn draw_status(&mut self, ui: &mut egui::Ui) {
        ui.heading("Status");
        if let Some(report) = self.last_report {
            ui.label(format!(
                "Last session: {} frames | {} capture errors | {} dropped",
                report.frames_transmitted, report.capture_errors, report.dropped_frames
            ));
        }
        ScrollArea::vertical()
            .max_height(200.0)
            .id_source("sender_status_log")
            .show(ui, |scroll| {
                for entry in self.status_log.iter().rev() {
                    let elapsed = entry
                        .timestamp
                        .checked_duration_since(self.start_time)
                        .unwrap_or_default()
                        .as_secs_f32();
                    let text = format!("[{:>5.1}s] {}", elapsed, entry.message);
                    scroll.colored_label(entry.level.colour(), text);
                }
            });
    }

    fn current_resolution_label(&self) -> String {
        match self.resolution_selection {
            ResolutionSelection::Native => "Native (no scaling)".to_string(),
            ResolutionSelection::Preset(index) => RESOLUTION_PRESETS
                .get(index)
                .map(|preset| preset.label.to_string())
                .unwrap_or_else(|| "Preset".to_string()),
            ResolutionSelection::Custom => {
                let value = self.custom_resolution_input.trim();
                if value.is_empty() {
                    "Custom".to_string()
                } else {
                    format!("Custom ({value})")
                }
            }
        }
    }

    fn current_bitrate_label(&self) -> String {
        if self.bitrate_preset == BitratePreset::Custom {
            let value = self.custom_bitrate_input.trim();
            if value.is_empty() {
                "Custom".to_string()
            } else {
                format!("Custom ({value})")
            }
        } else {
            preset_label(self.bitrate_preset).to_string()
        }
    }
}

impl App for SenderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.update_channels();
        self.cleanup_finished_handle();

        let repaint_ms = if self.is_pipeline_active() {
            ACTIVE_REPAINT_MS
        } else {
            IDLE_REPAINT_MS
        };
        ctx.request_repaint_after(Duration::from_millis(repaint_ms));

        egui::TopBottomPanel::top("sender_controls_panel").show(ctx, |ui| {
            ui.with_layout(Layout::top_down(Align::LEFT), |ui| self.draw_controls(ui));
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                self.draw_metrics(ui);
                ui.add_space(12.0);
                self.draw_status(ui);
            });
        });
    }
}

impl Drop for SenderApp {
    fn drop(&mut self) {
        if let Some(handle) = self.pipeline_handle.take() {
            handle.abort();
        }
    }
}

impl PipelineState {
    fn as_str(self) -> &'static str {
        match self {
            PipelineState::Idle => "Idle",
            PipelineState::Starting => "Starting",
            PipelineState::Running => "Running",
            PipelineState::Error => "Error",
        }
    }
}

impl StatusLevel {
    fn colour(self) -> Color32 {
        match self {
            StatusLevel::Info => Color32::from_rgb(189, 195, 199),
            StatusLevel::Warning => Color32::from_rgb(243, 156, 18),
            StatusLevel::Error => Color32::from_rgb(231, 76, 60),
        }
    }
}

fn preset_label(preset: BitratePreset) -> &'static str {
    match preset {
        BitratePreset::Low => "Low",
        BitratePreset::Medium => "Medium",
        BitratePreset::High => "High",
        BitratePreset::Custom => "Custom",
    }
}

fn parse_resolution(raw: &str) -> Result<(u32, u32), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("provide WIDTHxHEIGHT values".into());
    }
    let parts: Vec<_> = trimmed.split(['x', 'X']).collect();
    if parts.len() != 2 {
        return Err("expected WIDTHxHEIGHT format".into());
    }
    let width = parts[0]
        .parse::<u32>()
        .map_err(|_| "invalid width component".to_string())?;
    let height = parts[1]
        .parse::<u32>()
        .map_err(|_| "invalid height component".to_string())?;
    if width == 0 || height == 0 {
        return Err("dimensions must be positive".into());
    }
    Ok((width, height))
}
