#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

#[cfg(not(target_os = "windows"))]
compile_error!("The capture crate currently only supports Windows targets.");

#[cfg(not(feature = "runtime"))]
compile_error!("The capture crate requires the `runtime` feature to produce DXGI capture streams.");

use std::fmt;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use shared::{AppError, AppResult};
use tracing::{debug, instrument, trace, warn};

#[cfg(feature = "runtime")]
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
#[cfg(feature = "runtime")]
use tokio_stream::wrappers::UnboundedReceiverStream;
#[cfg(feature = "runtime")]
use tokio_stream::{Stream, StreamExt};

use windows::core::{Error as WinError, Interface};
use windows::Win32::Foundation::{RECT, S_FALSE};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_FEATURE_LEVEL_10_0,
    D3D11_FEATURE_LEVEL_10_1, D3D11_FEATURE_LEVEL_11_0, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_DENIED, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_DEVICE_REMOVED,
    DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_NOT_FOUND, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_DESC,
    DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput,
    IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, DXGI_OUTPUT_DESC,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Ole::RPC_E_CHANGED_MODE;

#[cfg(feature = "runtime")]
use std::task::{Context, Poll};

/// Lightweight identifier for a physical monitor, expressed as adapter/output indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MonitorId {
    pub adapter_index: u32,
    pub output_index: u32,
}

impl MonitorId {
    pub const PRIMARY: Self = Self {
        adapter_index: 0,
        output_index: 0,
    };
}

impl fmt::Display for MonitorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "adapter {}, output {}", self.adapter_index, self.output_index)
    }
}

/// Human-friendly description of a monitor and its DXGI configuration.
#[derive(Debug, Clone)]
pub struct MonitorDescriptor {
    pub id: MonitorId,
    pub name: String,
    pub resolution: (u32, u32),
    pub rotation: Rotation,
    pub format: DXGI_FORMAT,
}

/// Representation of the monitor's rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rotation {
    Identity,
    Rotate90,
    Rotate180,
    Rotate270,
    Unspecified,
}

impl From<DXGI_MODE_ROTATION> for Rotation {
    fn from(value: DXGI_MODE_ROTATION) -> Self {
        match value {
            DXGI_MODE_ROTATION::Unspecified => Self::Unspecified,
            DXGI_MODE_ROTATION::Identity => Self::Identity,
            DXGI_MODE_ROTATION::Rotate90 => Self::Rotate90,
            DXGI_MODE_ROTATION::Rotate180 => Self::Rotate180,
            DXGI_MODE_ROTATION::Rotate270 => Self::Rotate270,
            _ => Self::Unspecified,
        }
    }
}

/// Configuration options for DXGI desktop duplication.
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub monitor: MonitorId,
    pub frame_rate: NonZeroU32,
    pub timeout: Duration,
}

impl CaptureConfig {
    pub fn new(monitor: MonitorId) -> Self {
        Self {
            monitor,
            ..Default::default()
        }
    }

    pub fn with_frame_rate(mut self, frame_rate: NonZeroU32) -> Self {
        self.frame_rate = frame_rate;
        self.timeout = Duration::from_secs_f64(1.0 / frame_rate.get() as f64);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.max(Duration::from_millis(1));
        self
    }

    fn frame_interval(&self) -> Duration {
        Duration::from_secs_f64(1.0 / self.frame_rate.get() as f64)
    }

    fn timeout_ms(&self) -> u32 {
        let millis = self.timeout.as_millis().clamp(1, u128::from(u32::MAX));
        millis as u32
    }
}

impl Default for CaptureConfig {
    fn default() -> Self {
        let default_rate = NonZeroU32::new(60).unwrap();
        Self {
            monitor: MonitorId::PRIMARY,
            frame_rate: default_rate,
            timeout: Duration::from_secs_f64(1.0 / f64::from(default_rate.get())),
        }
    }
}

/// Metadata describing a captured frame beyond the raw pixel buffer.
#[derive(Debug, Clone)]
pub struct FrameMetadata {
    pub timestamp: SystemTime,
    pub monitor: MonitorId,
    pub frame_index: u64,
    pub accumulated_frames: u32,
    pub metadata_size: u32,
    pub pointer_shape_size: u32,
    pub pointer_position: Option<(i32, i32)>,
    pub pointer_visible: bool,
    pub rects_coalesced: bool,
    pub protected_content_masked: bool,
    pub last_present_qpc: i64,
    pub last_mouse_update_qpc: i64,
}

/// CPU-accessible BGRA frame data paired with its metadata.
#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub metadata: FrameMetadata,
}

/// Public facade for configuring and running DXGI desktop duplication.
#[derive(Debug, Clone)]
pub struct FrameCapture {
    config: CaptureConfig,
    monitor: MonitorDescriptor,
}

impl FrameCapture {
    #[instrument]
    pub fn new(config: CaptureConfig) -> AppResult<Self> {
        let monitors = enumerate_monitors()?;
        let monitor = monitors
            .into_iter()
            .find(|descriptor| descriptor.id == config.monitor)
            .ok_or_else(|| AppError::Message(format!("no monitor found for {}", config.monitor)))?;

        Ok(Self { config, monitor })
    }

    #[instrument]
    pub fn new_primary() -> AppResult<Self> {
        Self::new(CaptureConfig::default())
    }

    pub fn config(&self) -> &CaptureConfig {
        &self.config
    }

    pub fn monitor(&self) -> &MonitorDescriptor {
        &self.monitor
    }

    #[cfg(feature = "runtime")]
    #[instrument]
    pub fn into_stream(self) -> AppResult<FrameStream> {
        spawn_capture_thread(self.config, self.monitor)
    }

    #[cfg(feature = "runtime")]
    #[instrument]
    pub fn frame_stream(&self) -> AppResult<FrameStream> {
        self.clone().into_stream()
    }
}

/// Enumerate all monitors exposed through DXGI.
#[instrument]
pub fn enumerate_monitors() -> AppResult<Vec<MonitorDescriptor>> {
    let _com = ComGuard::new()?;
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1().map_err(|err| map_error("CreateDXGIFactory1", err))? };
    let mut descriptors = Vec::new();
    let mut adapter_index = 0;

    loop {
        match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => {
                let mut output_index = 0;
                loop {
                    match unsafe { adapter.EnumOutputs(output_index) } {
                        Ok(output) => {
                            let descriptor = build_monitor_descriptor(adapter_index, output_index, &output)?;
                            descriptors.push(descriptor);
                            output_index += 1;
                        }
                        Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => break,
                        Err(err) => return Err(map_error("IDXGIAdapter1::EnumOutputs", err)),
                    }
                }
                adapter_index += 1;
            }
            Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(err) => return Err(map_error("IDXGIFactory1::EnumAdapters1", err)),
        }
    }

    Ok(descriptors)
}

#[cfg(feature = "runtime")]
#[derive(Debug)]
pub struct FrameStream {
    inner: UnboundedReceiverStream<AppResult<CapturedFrame>>,
    cancel: Arc<AtomicBool>,
}

#[cfg(feature = "runtime")]
impl FrameStream {
    fn new(receiver: UnboundedReceiver<AppResult<CapturedFrame>>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            inner: UnboundedReceiverStream::new(receiver),
            cancel,
        }
    }

    pub async fn next_frame(&mut self) -> Option<AppResult<CapturedFrame>> {
        self.inner.next().await
    }
}

#[cfg(feature = "runtime")]
impl Stream for FrameStream {
    type Item = AppResult<CapturedFrame>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Safety: FrameStream never moves inner after pin. We only delegate polling.
        let inner = unsafe { self.map_unchecked_mut(|me| &mut me.inner) };
        inner.poll_next(cx)
    }
}

#[cfg(feature = "runtime")]
impl Drop for FrameStream {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

#[cfg(feature = "runtime")]
#[instrument]
fn spawn_capture_thread(config: CaptureConfig, monitor: MonitorDescriptor) -> AppResult<FrameStream> {
    let (tx, rx) = mpsc::unbounded_channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_flag = Arc::clone(&cancel);

    thread::Builder::new()
        .name("dxgi-capture".into())
        .spawn(move || {
            if let Err(err) = run_capture_loop(config, monitor, cancel_flag, tx.clone()) {
                let _ = tx.send(Err(err));
            }
        })
        .map_err(|err| AppError::Message(format!("failed to spawn capture thread: {err}")))?;

    Ok(FrameStream::new(rx, cancel))
}

#[cfg(feature = "runtime")]
fn run_capture_loop(
    config: CaptureConfig,
    monitor: MonitorDescriptor,
    cancel: Arc<AtomicBool>,
    sender: UnboundedSender<AppResult<CapturedFrame>>,
) -> AppResult<()> {
    let _com = ComGuard::new()?;
    let mut duplicator = DxgiDuplicator::new(&monitor)?;
    let mut frame_index: u64 = 0;
    let frame_interval = config.frame_interval();
    let timeout_ms = config.timeout_ms();

    loop {
        if cancel.load(Ordering::Relaxed) {
            trace!("capture loop cancelled");
            break;
        }

        let start = Instant::now();

        match duplicator.capture_next(timeout_ms)? {
            FrameAcquisition::Frame(mut frame) => {
                frame.metadata.frame_index = frame_index;
                frame.metadata.monitor = monitor.id;
                frame_index = frame_index.saturating_add(1);

                if sender.send(Ok(frame)).is_err() {
                    trace!("receiver dropped; terminating capture loop");
                    break;
                }
            }
            FrameAcquisition::Timeout => {
                trace!("no new frame within timeout window");
            }
            FrameAcquisition::NeedsReset => {
                warn!("output duplication lost; recreating device");
                duplicator = DxgiDuplicator::new(&monitor)?;
                continue;
            }
        }

        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let elapsed = start.elapsed();
        if frame_interval > elapsed {
            thread::sleep(frame_interval - elapsed);
        }
    }

    Ok(())
}

#[derive(Debug)]
struct DxgiDuplicator {
    _factory: IDXGIFactory1,
    _adapter: IDXGIAdapter1,
    _output: IDXGIOutput1,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    staging: Option<ID3D11Texture2D>,
    staging_desc: Option<D3D11_TEXTURE2D_DESC>,
}

impl DxgiDuplicator {
    fn new(monitor: &MonitorDescriptor) -> AppResult<Self> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1().map_err(|err| map_error("CreateDXGIFactory1", err))? };
        let adapter = unsafe { factory.EnumAdapters1(monitor.id.adapter_index) }
            .map_err(|err| map_error("IDXGIFactory1::EnumAdapters1", err))?;
        let output_base = unsafe { adapter.EnumOutputs(monitor.id.output_index) }
            .map_err(|err| map_error("IDXGIAdapter1::EnumOutputs", err))?;
        let output: IDXGIOutput1 = output_base
            .cast()
            .map_err(|err| map_error("IDXGIOutput::cast<IDXGIOutput1>", err))?;

        let (device, context) = create_device(&adapter)?;
        let duplication = unsafe { output.DuplicateOutput(&device) }
            .map_err(|err| map_error("IDXGIOutput1::DuplicateOutput", err))?;

        let mut desc = DXGI_OUTDUPL_DESC::default();
        unsafe { duplication.GetDesc(&mut desc) };
        debug!(
            width = desc.ModeDesc.Width,
            height = desc.ModeDesc.Height,
            format = ?desc.ModeDesc.Format,
            rotation = ?desc.Rotation,
            "initialised DXGI output duplication",
        );

        Ok(Self {
            _factory: factory,
            _adapter: adapter,
            _output: output,
            device,
            context,
            duplication,
            staging: None,
            staging_desc: None,
        })
    }

    fn capture_next(&mut self, timeout_ms: u32) -> AppResult<FrameAcquisition> {
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;

        match unsafe { self.duplication.AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource) } {
            Ok(()) => {}
            Err(err) => {
                let code = err.code();
                if code == DXGI_ERROR_WAIT_TIMEOUT {
                    return Ok(FrameAcquisition::Timeout);
                }
                if matches!(code, DXGI_ERROR_ACCESS_LOST | DXGI_ERROR_DEVICE_REMOVED | DXGI_ERROR_DEVICE_RESET) {
                    return Ok(FrameAcquisition::NeedsReset);
                }
                if code == DXGI_ERROR_ACCESS_DENIED {
                    return Err(AppError::Message(
                        "DXGI denied access to the duplication session; protected content is present".into(),
                    ));
                }
                return Err(map_error("IDXGIOutputDuplication::AcquireNextFrame", err));
            }
        }

        let mut frame_guard = FrameGuard::new(&self.duplication);

        let resource = match resource {
            Some(res) => res,
            None => {
                frame_guard.release()?;
                return Ok(FrameAcquisition::Timeout);
            }
        };

        let texture: ID3D11Texture2D = resource
            .cast()
            .map_err(|err| map_error("IDXGIResource::cast<ID3D11Texture2D>", err))?;

        let texture_desc = unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            desc
        };

        if texture_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            warn!(format = ?texture_desc.Format, "unexpected desktop duplication format; expected BGRA8");
        }

        self.ensure_staging(&texture_desc)?;
        let staging = self
            .staging
            .as_ref()
            .ok_or_else(|| AppError::Message("staging texture unavailable".into()))?;

        let staging_resource: ID3D11Resource = staging
            .cast()
            .map_err(|err| map_error("ID3D11Texture2D::cast<ID3D11Resource>", err))?;
        let source_resource: ID3D11Resource = texture
            .cast()
            .map_err(|err| map_error("ID3D11Texture2D::cast<ID3D11Resource>", err))?;

        unsafe {
            self.context.CopyResource(&staging_resource, &source_resource);
        }

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(&staging_resource, 0, D3D11_MAP_READ, 0, &mut mapped)
        }
        .map_err(|err| map_error("ID3D11DeviceContext::Map", err))?;

        let width = texture_desc.Width;
        let height = texture_desc.Height;
        let row_pitch = mapped.RowPitch as usize;
        let bytes_per_row = width as usize * 4;
        let total_bytes = bytes_per_row.checked_mul(height as usize).ok_or_else(|| {
            AppError::Message("frame dimensions overflowed expected buffer size".into())
        })?;

        let source_ptr = mapped.pData as *const u8;
        if source_ptr.is_null() {
            unsafe { self.context.Unmap(&staging_resource, 0) };
            frame_guard.release()?;
            return Err(AppError::Message("mapped staging texture returned a null pointer".into()));
        }

        let mut buffer = vec![0_u8; total_bytes];
        for row in 0..(height as usize) {
            let src_offset = row * row_pitch;
            let dst_offset = row * bytes_per_row;
            let src_slice = unsafe { std::slice::from_raw_parts(source_ptr.add(src_offset), bytes_per_row) };
            buffer[dst_offset..dst_offset + bytes_per_row].copy_from_slice(src_slice);
        }

        unsafe { self.context.Unmap(&staging_resource, 0) };
        frame_guard.release()?;

        if frame_info.AccumulatedFrames > 1 {
            warn!(
                accumulated = frame_info.AccumulatedFrames,
                "DXGI reported skipped frames since the previous acquisition"
            );
        }

        let metadata = FrameMetadata {
            timestamp: SystemTime::now(),
            monitor: MonitorId::PRIMARY,
            frame_index: 0,
            accumulated_frames: frame_info.AccumulatedFrames,
            metadata_size: frame_info.TotalMetadataBufferSize,
            pointer_shape_size: frame_info.PointerShapeBufferSize,
            pointer_position: pointer_position(&frame_info.PointerPosition),
            pointer_visible: frame_info.PointerPosition.Visible.as_bool(),
            rects_coalesced: frame_info.RectsCoalesced.as_bool(),
            protected_content_masked: frame_info.ProtectedContentMaskedOut.as_bool(),
            last_present_qpc: frame_info.LastPresentTime,
            last_mouse_update_qpc: frame_info.LastMouseUpdateTime,
        };

        Ok(FrameAcquisition::Frame(CapturedFrame {
            bytes: buffer,
            width,
            height,
            stride: (bytes_per_row) as u32,
            metadata,
        }))
    }

    fn ensure_staging(&mut self, desc: &D3D11_TEXTURE2D_DESC) -> AppResult<()> {
        let needs_recreate = match &self.staging_desc {
            Some(existing) => existing.Width != desc.Width || existing.Height != desc.Height || existing.Format != desc.Format,
            None => true,
        };

        if !needs_recreate {
            return Ok(());
        }

        let mut staging_desc = D3D11_TEXTURE2D_DESC::default();
        staging_desc.Width = desc.Width;
        staging_desc.Height = desc.Height;
        staging_desc.MipLevels = 1;
        staging_desc.ArraySize = 1;
        staging_desc.Format = desc.Format;
        staging_desc.SampleDesc = desc.SampleDesc;
        staging_desc.SampleDesc.Count = 1;
        staging_desc.SampleDesc.Quality = 0;
        staging_desc.Usage = D3D11_USAGE_STAGING;
        staging_desc.BindFlags = 0;
        staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0;
        staging_desc.MiscFlags = 0;

        let staging = unsafe { self.device.CreateTexture2D(&staging_desc, None) }
            .map_err(|err| map_error("ID3D11Device::CreateTexture2D", err))?;

        self.staging = Some(staging);
        self.staging_desc = Some(staging_desc);

        Ok(())
    }
}

fn create_device(adapter: &IDXGIAdapter1) -> AppResult<(ID3D11Device, ID3D11DeviceContext)> {
    let adapter_base: IDXGIAdapter = adapter
        .cast()
        .map_err(|err| map_error("IDXGIAdapter1::cast<IDXGIAdapter>", err))?;

    let feature_levels = [
        D3D11_FEATURE_LEVEL_11_0,
        D3D11_FEATURE_LEVEL_10_1,
        D3D11_FEATURE_LEVEL_10_0,
    ];

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let mut chosen_level = D3D11_FEATURE_LEVEL_11_0;

    unsafe {
        D3D11CreateDevice(
            Some(&adapter_base),
            D3D_DRIVER_TYPE_UNKNOWN,
            None,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            feature_levels.len() as u32,
            D3D11_SDK_VERSION,
            &mut device,
            Some(&mut chosen_level),
            &mut context,
        )
    }
    .map_err(|err| map_error("D3D11CreateDevice", err))?;

    let device = device.ok_or_else(|| AppError::Message("D3D11CreateDevice returned no device".into()))?;
    let context = context.ok_or_else(|| AppError::Message("D3D11CreateDevice returned no immediate context".into()))?;

    debug!(?chosen_level, "created D3D11 device for desktop duplication");

    Ok((device, context))
}

#[derive(Debug)]
enum FrameAcquisition {
    Frame(CapturedFrame),
    Timeout,
    NeedsReset,
}

#[derive(Debug)]
struct FrameGuard<'a> {
    duplication: &'a IDXGIOutputDuplication,
    active: bool,
}

impl<'a> FrameGuard<'a> {
    fn new(duplication: &'a IDXGIOutputDuplication) -> Self {
        Self {
            duplication,
            active: true,
        }
    }

    fn release(&mut self) -> AppResult<()> {
        if self.active {
            unsafe { self.duplication.ReleaseFrame() }
                .map_err(|err| map_error("IDXGIOutputDuplication::ReleaseFrame", err))?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for FrameGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = unsafe { self.duplication.ReleaseFrame() };
        }
    }
}

fn pointer_position(position: &windows::Win32::Graphics::Dxgi::DXGI_OUTDUPL_POINTER_POSITION) -> Option<(i32, i32)> {
    if position.Visible.as_bool() {
        Some((position.Position.x, position.Position.y))
    } else {
        None
    }
}

fn build_monitor_descriptor(
    adapter_index: u32,
    output_index: u32,
    output: &IDXGIOutput,
) -> AppResult<MonitorDescriptor> {
    let mut desc = DXGI_OUTPUT_DESC::default();
    unsafe { output.GetDesc(&mut desc) };

    let name = widestring_to_string(&desc.DeviceName);
    let rect: RECT = desc.DesktopCoordinates;
    let width = (rect.right - rect.left).max(0) as u32;
    let height = (rect.bottom - rect.top).max(0) as u32;

    Ok(MonitorDescriptor {
        id: MonitorId {
            adapter_index,
            output_index,
        },
        name,
        resolution: (width, height),
        rotation: Rotation::from(desc.Rotation),
        format: desc.DesktopMode.Format,
    })
}

fn widestring_to_string(buffer: &[u16; 32]) -> String {
    let terminator = buffer.iter().position(|value| *value == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..terminator])
}

fn map_error(context: &'static str, err: WinError) -> AppError {
    AppError::Message(format!("{context} failed: {err}"))
}

struct ComGuard {
    initialised: bool,
}

impl ComGuard {
    fn new() -> AppResult<Self> {
        unsafe {
            match CoInitializeEx(null_mut(), COINIT_MULTITHREADED) {
                Ok(()) => Ok(Self { initialised: true }),
                Err(err) if err.code() == S_FALSE => Ok(Self { initialised: true }),
                Err(err) if err.code() == RPC_E_CHANGED_MODE => {
                    warn!("COM already initialised with a different threading model; continuing without reinitialising");
                    Ok(Self { initialised: false })
                }
                Err(err) => Err(map_error("CoInitializeEx", err)),
            }
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialised {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn capture_config_defaults() {
        let config = CaptureConfig::default();
        assert_eq!(config.monitor, MonitorId::PRIMARY);
        assert_eq!(config.frame_rate.get(), 60);
        assert!(config.timeout_ms() > 0);
    }
}
