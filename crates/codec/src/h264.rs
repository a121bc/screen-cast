use shared::{AppError, AppResult};
use std::collections::VecDeque;
use std::sync::Arc;
use tracing::{debug, instrument, trace, warn};
use windows::core::{Interface, GUID, HRESULT};
use windows::Win32::Foundation::{E_FAIL, S_OK};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFAttributes, IMFMediaBuffer, IMFMediaType, IMFSample, IMFTransform,
    MFCreateAttributes, MFCreateMemoryBuffer, MFCreateSample, MFEnumTransforms, MFShutdown,
    MFStartup, MFTEnumFlag, MFTGetInfo, MFT_CATEGORY_VIDEO_DECODER, MFT_CATEGORY_VIDEO_ENCODER,
    MFT_ENUM_FLAG_ALL, MFT_ENUM_FLAG_SORTANDFILTER, MFT_INPUT_STREAM_INFO, MFT_OUTPUT_DATA_BUFFER,
    MFT_OUTPUT_STREAM_INFO, MFT_PROCESS_OUTPUT_STATUS_NEW_STREAMS, MFSTARTUP_FULL, MF_LOW_LATENCY,
    MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_MPEG2_PROFILE, MF_MT_SUBTYPE, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MFMediaType_Video,
    MFVideoInterlace_Progressive, MFVideoInterlaceMode,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

const MF_MT_MPEG2_LEVEL: GUID =
    GUID::from_u128(0x96f66574_11c5_4015_8666_bff516436da7);
const MFT_ENUM_TRANSCODE_ONLY_ATTRIBUTE: GUID =
    GUID::from_u128(0xb0c6eb44_0bf9_4b4a_8c0c_ae7dc5e8f5f1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Profile {
    Baseline,
    Main,
    High,
}

impl H264Profile {
    fn to_mf_profile(self) -> u32 {
        match self {
            H264Profile::Baseline => 66,
            H264Profile::Main => 77,
            H264Profile::High => 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra,
    Rgba,
}

#[derive(Debug, Clone)]
pub struct H264EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub bitrate: u32,
    pub framerate: u32,
    pub gop_length: u32,
    pub profile: H264Profile,
    pub pixel_format: PixelFormat,
}

impl Default for H264EncoderConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            bitrate: 8_000_000,
            framerate: 60,
            gop_length: 60,
            profile: H264Profile::High,
            pixel_format: PixelFormat::Bgra,
        }
    }
}

#[derive(Debug)]
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub timestamp: i64,
    pub is_keyframe: bool,
}

struct MfContext {
    _com_initialized: bool,
}

impl MfContext {
    fn new() -> AppResult<Self> {
        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            if hr.is_err() && hr != HRESULT(0x00000001) {
                return Err(AppError::MediaFoundation("Failed to initialize COM"));
            }

            MFStartup(MFSTARTUP_FULL, 0)
                .map_err(|_| AppError::MediaFoundation("Failed to initialize Media Foundation"))?;
        }

        Ok(Self {
            _com_initialized: true,
        })
    }
}

impl Drop for MfContext {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

pub struct H264Encoder {
    transform: IMFTransform,
    input_stream_info: MFT_INPUT_STREAM_INFO,
    output_stream_info: MFT_OUTPUT_STREAM_INFO,
    config: H264EncoderConfig,
    frame_count: i64,
    _mf_context: Arc<MfContext>,
}

impl H264Encoder {
    #[instrument(skip(config))]
    pub fn new(config: H264EncoderConfig) -> AppResult<Self> {
        if config.width == 0 || config.height == 0 {
            return Err(AppError::Message("Width and height must be positive".into()));
        }
        if config.bitrate == 0 {
            return Err(AppError::Message("Bitrate must be positive".into()));
        }
        if config.framerate == 0 {
            return Err(AppError::Message("Framerate must be positive".into()));
        }

        let mf_context = Arc::new(MfContext::new()?);

        let transform = Self::create_h264_encoder()?;

        Self::configure_encoder(&transform, &config)?;

        let input_stream_info = unsafe {
            let mut info = std::mem::zeroed();
            transform
                .GetInputStreamInfo(0, &mut info)
                .map_err(|_| AppError::MediaFoundation("Failed to get input stream info"))?;
            info
        };

        let output_stream_info = unsafe {
            let mut info = std::mem::zeroed();
            transform
                .GetOutputStreamInfo(0, &mut info)
                .map_err(|_| AppError::MediaFoundation("Failed to get output stream info"))?;
            info
        };

        debug!(
            width = config.width,
            height = config.height,
            bitrate = config.bitrate,
            framerate = config.framerate,
            gop_length = config.gop_length,
            profile = ?config.profile,
            "H264 encoder initialized"
        );

        Ok(Self {
            transform,
            input_stream_info,
            output_stream_info,
            config,
            frame_count: 0,
            _mf_context: mf_context,
        })
    }

    fn create_h264_encoder() -> AppResult<IMFTransform> {
        unsafe {
            let input_type = MFMediaType_Video;
            let output_type = MFMediaType_Video;

            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attributes, 1)
                .map_err(|_| AppError::MediaFoundation("Failed to create attributes"))?;

            let attrs = attributes.ok_or(AppError::MediaFoundation("Attributes is null"))?;

            attrs
                .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                .ok();

            let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count = 0u32;

            MFEnumTransforms(
                &MFT_CATEGORY_VIDEO_ENCODER,
                MFTEnumFlag(MFT_ENUM_FLAG_ALL | MFT_ENUM_FLAG_SORTANDFILTER),
                Some(&input_type),
                Some(&output_type),
                Some(&attrs),
                &mut activates,
                &mut count,
            )
            .map_err(|_| AppError::MediaFoundation("Failed to enumerate transforms"))?;

            if count == 0 || activates.is_null() {
                return Err(AppError::MediaFoundation("No H264 encoder found"));
            }

            let activates_slice = std::slice::from_raw_parts(activates, count as usize);

            let mut h264_transform: Option<IMFTransform> = None;
            for activate in activates_slice {
                if let Some(act) = activate {
                    if let Ok(transform) = act.ActivateObject::<IMFTransform>() {
                        h264_transform = Some(transform);
                        break;
                    }
                }
            }

            for i in 0..count {
                if let Some(activate) = activates_slice[i as usize].as_ref() {
                    let _ = activate.ShutdownObject();
                }
            }

            h264_transform.ok_or(AppError::MediaFoundation("Failed to activate H264 encoder"))
        }
    }

    fn configure_encoder(transform: &IMFTransform, config: &H264EncoderConfig) -> AppResult<()> {
        unsafe {
            let input_type = Self::create_input_media_type(config)?;
            transform
                .SetInputType(0, &input_type, 0)
                .map_err(|_| AppError::MediaFoundation("Failed to set input type"))?;

            let output_type = Self::create_output_media_type(config)?;
            transform
                .SetOutputType(0, &output_type, 0)
                .map_err(|_| AppError::MediaFoundation("Failed to set output type"))?;

            if let Ok(attrs) = transform.GetAttributes() {
                let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
                let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
            }

            transform
                .ProcessMessage(0, 0)
                .map_err(|_| AppError::MediaFoundation("Failed to send stream start message"))?;
        }

        Ok(())
    }

    fn create_input_media_type(config: &H264EncoderConfig) -> AppResult<IMFMediaType> {
        unsafe {
            let media_type: IMFMediaType =
                windows::Win32::Media::MediaFoundation::MFCreateMediaType()
                    .map_err(|_| {
                        AppError::MediaFoundation("Failed to create input media type")
                    })?;

            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|_| AppError::MediaFoundation("Failed to set major type"))?;

            let subtype = match config.pixel_format {
                PixelFormat::Bgra => {
                    GUID::from_u128(0x42475241_0000_0010_8000_00aa00389b71)
                }
                PixelFormat::Rgba => {
                    GUID::from_u128(0x41424752_0000_0010_8000_00aa00389b71)
                }
            };

            media_type
                .SetGUID(&MF_MT_SUBTYPE, &subtype)
                .map_err(|_| AppError::MediaFoundation("Failed to set subtype"))?;

            let frame_size = ((config.width as u64) << 32) | (config.height as u64);
            media_type
                .SetUINT64(&MF_MT_FRAME_SIZE, frame_size)
                .map_err(|_| AppError::MediaFoundation("Failed to set frame size"))?;

            let framerate_ratio = ((config.framerate as u64) << 32) | 1u64;
            media_type
                .SetUINT64(&MF_MT_FRAME_RATE, framerate_ratio)
                .map_err(|_| AppError::MediaFoundation("Failed to set framerate"))?;

            media_type
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(|_| AppError::MediaFoundation("Failed to set interlace mode"))?;

            Ok(media_type)
        }
    }

    fn create_output_media_type(config: &H264EncoderConfig) -> AppResult<IMFMediaType> {
        unsafe {
            let media_type: IMFMediaType =
                windows::Win32::Media::MediaFoundation::MFCreateMediaType()
                    .map_err(|_| {
                        AppError::MediaFoundation("Failed to create output media type")
                    })?;

            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|_| AppError::MediaFoundation("Failed to set major type"))?;

            let h264_subtype = GUID::from_u128(0x34363248_0000_0010_8000_00aa00389b71);
            media_type
                .SetGUID(&MF_MT_SUBTYPE, &h264_subtype)
                .map_err(|_| AppError::MediaFoundation("Failed to set H264 subtype"))?;

            let frame_size = ((config.width as u64) << 32) | (config.height as u64);
            media_type
                .SetUINT64(&MF_MT_FRAME_SIZE, frame_size)
                .map_err(|_| AppError::MediaFoundation("Failed to set frame size"))?;

            let framerate_ratio = ((config.framerate as u64) << 32) | 1u64;
            media_type
                .SetUINT64(&MF_MT_FRAME_RATE, framerate_ratio)
                .map_err(|_| AppError::MediaFoundation("Failed to set framerate"))?;

            media_type
                .SetUINT32(&MF_MT_AVG_BITRATE, config.bitrate)
                .map_err(|_| AppError::MediaFoundation("Failed to set bitrate"))?;

            media_type
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(|_| AppError::MediaFoundation("Failed to set interlace mode"))?;

            let profile = config.profile.to_mf_profile();
            media_type
                .SetUINT32(&MF_MT_MPEG2_PROFILE, profile)
                .map_err(|_| AppError::MediaFoundation("Failed to set profile"))?;

            Ok(media_type)
        }
    }

    #[instrument(skip(self, frame_data))]
    pub fn encode(&mut self, frame_data: &[u8]) -> AppResult<Vec<EncodedFrame>> {
        let bytes_per_pixel = match self.config.pixel_format {
            PixelFormat::Bgra | PixelFormat::Rgba => 4,
        };
        let expected_size = (self.config.width * self.config.height * bytes_per_pixel) as usize;

        if frame_data.len() != expected_size {
            return Err(AppError::Message(format!(
                "Invalid frame size: expected {}, got {}",
                expected_size,
                frame_data.len()
            )));
        }

        let sample = self.create_input_sample(frame_data)?;

        let timestamp = self.calculate_timestamp();
        unsafe {
            sample
                .SetSampleTime(timestamp)
                .map_err(|_| AppError::MediaFoundation("Failed to set sample time"))?;

            let duration = (10_000_000 / self.config.framerate as i64);
            sample
                .SetSampleDuration(duration)
                .map_err(|_| AppError::MediaFoundation("Failed to set sample duration"))?;
        }

        self.process_input(sample)?;

        let mut encoded_frames = Vec::new();
        loop {
            match self.process_output() {
                Ok(Some(frame)) => encoded_frames.push(frame),
                Ok(None) => break,
                Err(e) => {
                    if encoded_frames.is_empty() {
                        return Err(e);
                    }
                    break;
                }
            }
        }

        self.frame_count += 1;

        if encoded_frames.is_empty() {
            trace!("No output available yet (encoder buffering)");
        } else {
            debug!(count = encoded_frames.len(), "Encoded frames produced");
        }

        Ok(encoded_frames)
    }

    fn create_input_sample(&self, frame_data: &[u8]) -> AppResult<IMFSample> {
        unsafe {
            let buffer: IMFMediaBuffer = MFCreateMemoryBuffer(frame_data.len() as u32)
                .map_err(|_| AppError::MediaFoundation("Failed to create memory buffer"))?;

            let mut ptr: *mut u8 = std::ptr::null_mut();
            buffer
                .Lock(&mut ptr, None, None)
                .map_err(|_| AppError::MediaFoundation("Failed to lock buffer"))?;

            std::ptr::copy_nonoverlapping(frame_data.as_ptr(), ptr, frame_data.len());

            buffer
                .Unlock()
                .map_err(|_| AppError::MediaFoundation("Failed to unlock buffer"))?;

            buffer
                .SetCurrentLength(frame_data.len() as u32)
                .map_err(|_| AppError::MediaFoundation("Failed to set buffer length"))?;

            let sample: IMFSample = MFCreateSample()
                .map_err(|_| AppError::MediaFoundation("Failed to create sample"))?;

            sample
                .AddBuffer(&buffer)
                .map_err(|_| AppError::MediaFoundation("Failed to add buffer to sample"))?;

            Ok(sample)
        }
    }

    fn process_input(&self, sample: IMFSample) -> AppResult<()> {
        unsafe {
            self.transform
                .ProcessInput(0, &sample, 0)
                .map_err(|e| {
                    AppError::MediaFoundation("Failed to process input")
                })?;
        }
        Ok(())
    }

    fn process_output(&self) -> AppResult<Option<EncodedFrame>> {
        unsafe {
            let mut output_buffer = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: None,
                dwStatus: 0,
                pEvents: None,
            };

            let mut status = 0u32;
            let result = self.transform.ProcessOutput(0, 1, &mut output_buffer, &mut status);

            if result.is_err() {
                let hr = HRESULT(result.0);
                if hr.0 == 0xC00D6D72u32 as i32 {
                    return Ok(None);
                }
                return Err(AppError::MediaFoundation("ProcessOutput failed"));
            }

            if let Some(sample) = output_buffer.pSample {
                let data = self.extract_sample_data(&sample)?;
                let timestamp = sample.GetSampleTime().unwrap_or(0);

                let is_keyframe = self.is_keyframe(&sample);

                Ok(Some(EncodedFrame {
                    data,
                    timestamp,
                    is_keyframe,
                }))
            } else {
                Ok(None)
            }
        }
    }

    fn extract_sample_data(&self, sample: &IMFSample) -> AppResult<Vec<u8>> {
        unsafe {
            let buffer: IMFMediaBuffer = sample
                .GetBufferByIndex(0)
                .map_err(|_| AppError::MediaFoundation("Failed to get buffer"))?;

            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut length = 0u32;
            buffer
                .Lock(&mut ptr, None, Some(&mut length))
                .map_err(|_| AppError::MediaFoundation("Failed to lock buffer"))?;

            let data = std::slice::from_raw_parts(ptr, length as usize).to_vec();

            buffer
                .Unlock()
                .map_err(|_| AppError::MediaFoundation("Failed to unlock buffer"))?;

            Ok(data)
        }
    }

    fn is_keyframe(&self, sample: &IMFSample) -> bool {
        unsafe {
            let keyframe_guid = GUID::from_u128(0x9154b92_5f0e_4797_8407_d5b00ae8ff00);
            sample.GetUINT32(&keyframe_guid).unwrap_or(0) != 0
        }
    }

    fn calculate_timestamp(&self) -> i64 {
        (self.frame_count * 10_000_000) / self.config.framerate as i64
    }

    pub fn config(&self) -> &H264EncoderConfig {
        &self.config
    }

    pub fn flush(&mut self) -> AppResult<Vec<EncodedFrame>> {
        unsafe {
            let _ = self.transform.ProcessMessage(3, 0);
        }

        let mut frames = Vec::new();
        loop {
            match self.process_output() {
                Ok(Some(frame)) => frames.push(frame),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        Ok(frames)
    }
}

unsafe impl Send for H264Encoder {}

pub fn parse_nal_units(data: &[u8]) -> Vec<(usize, usize, u8)> {
    let mut nal_units = Vec::new();
    let mut i = 0;

    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            let start = if data[i + 2] == 1 {
                i + 3
            } else if i + 3 < data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                i + 4
            } else {
                i += 1;
                continue;
            };

            let nal_type = data[start] & 0x1F;

            let mut end = start;
            while end + 3 < data.len() {
                let is_start_code = data[end] == 0
                    && data[end + 1] == 0
                    && (data[end + 2] == 1
                        || (end + 3 < data.len()
                            && data[end + 2] == 0
                            && data[end + 3] == 1));
                if is_start_code {
                    break;
                }
                end += 1;
            }

            if end == start {
                end = data.len();
            }

            nal_units.push((start, end, nal_type));
            i = end;
        } else {
            i += 1;
        }
    }

    nal_units
}

pub const NAL_TYPE_SPS: u8 = 7;
pub const NAL_TYPE_PPS: u8 = 8;
pub const NAL_TYPE_IDR: u8 = 5;
pub const NAL_TYPE_NON_IDR: u8 = 1;

// ===== H.264 Decoder (Media Foundation) =====

/// Decoder output mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderOutput {
    /// CPU-accessible NV12 buffer (YUV420). Lowest overhead, widely supported.
    CpuNv12,
    /// CPU-accessible BGRA buffer. Supported if decoder exposes a color converter.
    CpuBgra,
    /// DXGI texture output (Windows-only). Requires D3D device manager wiring.
    #[allow(dead_code)]
    DxgiTexture,
}

/// Pixel format for decoded CPU frames
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderPixelFormat {
    Nv12,
    Bgra,
}

/// Drop policy when the internal frame buffer is full
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropPolicy {
    DropOldest,
    DropNewest,
}

/// Buffering configuration for frame queueing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferingConfig {
    pub max_frames: usize,
    pub drop_policy: DropPolicy,
}

impl Default for BufferingConfig {
    fn default() -> Self {
        Self {
            max_frames: 4,
            drop_policy: DropPolicy::DropOldest,
        }
    }
}

/// Decoder configuration
#[derive(Debug, Clone)]
pub struct H264DecoderConfig {
    pub output: DecoderOutput,
    pub buffering: BufferingConfig,
}

impl Default for H264DecoderConfig {
    fn default() -> Self {
        Self {
            output: DecoderOutput::CpuNv12,
            buffering: BufferingConfig::default(),
        }
    }
}

/// Video format descriptor used for reconfiguration events
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub pixel_format: DecoderPixelFormat,
    pub stride: u32,
}

/// Decoded frame in CPU memory
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub data: Vec<u8>,
    pub timestamp: i64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: DecoderPixelFormat,
    /// Indicates if the decoded output corresponds to an IDR/keyframe
    pub is_keyframe: bool,
}

/// Decoder event stream items
#[derive(Debug, Clone)]
pub enum DecoderEvent {
    Reconfigured(VideoFormat),
}

/// Output item from decoder polling
#[derive(Debug, Clone)]
pub enum DecodeOutput {
    Frame(DecodedFrame),
    Event(DecoderEvent),
}

/// Windows Media Foundation H.264 decoder (NAL in -> frames out)
pub struct H264Decoder {
    transform: IMFTransform,
    current_format: VideoFormat,
    cfg: H264DecoderConfig,
    queue: VecDeque<DecodeOutput>,
    _mf_context: Arc<MfContext>,
}

impl H264Decoder {
    #[instrument]
    pub fn new(cfg: H264DecoderConfig) -> AppResult<Self> {
        let mf = Arc::new(MfContext::new()?);
        let transform = Self::create_h264_decoder(&cfg)?;
        let fmt = Self::negotiate_output_format(&transform, cfg.output)?;

        unsafe {
            if let Ok(attrs) = transform.GetAttributes() {
                let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
                let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
            }
            // MFT_MESSAGE_COMMAND_FLUSH (0x00000001) and START OF STREAM (0x00000002) aren't
            // strictly necessary here; many decoders accept streaming immediately.
            let _ = transform.ProcessMessage(0, 0);
        }

        Ok(Self {
            transform,
            current_format: fmt,
            cfg,
            queue: VecDeque::new(),
            _mf_context: mf,
        })
    }

    fn create_h264_decoder(cfg: &H264DecoderConfig) -> AppResult<IMFTransform> {
        unsafe {
            let input_type = MFMediaType_Video;
            let output_type = MFMediaType_Video;

            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attributes, 1)
                .map_err(|_| AppError::MediaFoundation("Failed to create attributes"))?;
            let attrs = attributes.ok_or(AppError::MediaFoundation("Attributes is null"))?;

            // Prefer hardware transforms when available
            let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);

            let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count = 0u32;

            MFEnumTransforms(
                &MFT_CATEGORY_VIDEO_DECODER,
                MFTEnumFlag(MFT_ENUM_FLAG_ALL | MFT_ENUM_FLAG_SORTANDFILTER),
                Some(&input_type),
                Some(&output_type),
                Some(&attrs),
                &mut activates,
                &mut count,
            )
            .map_err(|_| AppError::MediaFoundation("Failed to enumerate decoders"))?;

            if count == 0 || activates.is_null() {
                return Err(AppError::MediaFoundation("No H264 decoder found"));
            }

            let activates_slice = std::slice::from_raw_parts(activates, count as usize);
            for activate in activates_slice {
                if let Some(act) = activate {
                    if let Ok(transform) = act.ActivateObject::<IMFTransform>() {
                        // We may later inspect transform attributes to ensure H.264 support.
                        return Ok(transform);
                    }
                }
            }

            Err(AppError::MediaFoundation("Failed to activate H264 decoder"))
        }
    }

    fn negotiate_output_format(transform: &IMFTransform, output: DecoderOutput) -> AppResult<VideoFormat> {
        // Configure input type as H.264 bitstream
        unsafe {
            let in_type: IMFMediaType = windows::Win32::Media::MediaFoundation::MFCreateMediaType()
                .map_err(|_| AppError::MediaFoundation("Failed to create input media type"))?;
            in_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|_| AppError::MediaFoundation("Failed to set input major type"))?;
            let h264_subtype = GUID::from_u128(0x34363248_0000_0010_8000_00aa00389b71); // 'H264'
            in_type
                .SetGUID(&MF_MT_SUBTYPE, &h264_subtype)
                .map_err(|_| AppError::MediaFoundation("Failed to set H264 subtype for input"))?;
            transform
                .SetInputType(0, &in_type, 0)
                .map_err(|_| AppError::MediaFoundation("Failed to set decoder input type"))?;
        }

        // Configure output media type according to desired output
        let (subtype, pixel_format) = match output {
            DecoderOutput::CpuNv12 => (
                GUID::from_u128(0x3231564E_0000_0010_8000_00aa00389b71), // NV12
                DecoderPixelFormat::Nv12,
            ),
            DecoderOutput::CpuBgra => (
                GUID::from_u128(0x42475241_0000_0010_8000_00aa00389b71), // BGRA/ARGB32
                DecoderPixelFormat::Bgra,
            ),
            DecoderOutput::DxgiTexture => (
                GUID::from_u128(0x3231564E_0000_0010_8000_00aa00389b71), // still request NV12; DXGI path not wired
                DecoderPixelFormat::Nv12,
            ),
        };

        unsafe {
            let out_type: IMFMediaType = windows::Win32::Media::MediaFoundation::MFCreateMediaType()
                .map_err(|_| AppError::MediaFoundation("Failed to create output media type"))?;
            out_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|_| AppError::MediaFoundation("Failed to set output major type"))?;
            out_type
                .SetGUID(&MF_MT_SUBTYPE, &subtype)
                .map_err(|_| AppError::MediaFoundation("Failed to set output subtype"))?;

            // Don't specify frame size here; the decoder will establish it from bitstream SPS.
            transform
                .SetOutputType(0, &out_type, 0)
                .map_err(|_| AppError::MediaFoundation("Failed to set decoder output type"))?;
        }

        // Unknown until first output; use 0 to signal "uninitialized".
        Ok(VideoFormat { width: 0, height: 0, pixel_format, stride: 0 })
    }

    #[instrument(skip(self, nal_data))]
    pub fn push_nal(&mut self, nal_data: &[u8], timestamp_100ns: i64) -> AppResult<()> {
        // Backpressure via buffering policy is handled after outputs are produced.
        let sample = Self::create_nal_sample(nal_data, timestamp_100ns)?;
        self.process_input(sample)?;
        self.drain_outputs()?;
        Ok(())
    }

    #[instrument]
    pub fn try_get_output(&mut self) -> AppResult<Option<DecodeOutput>> {
        if let Some(item) = self.queue.pop_front() {
            return Ok(Some(item));
        }
        // Attempt to produce one output from decoder
        match self.process_output()? {
            Some(item) => Ok(Some(item)),
            None => Ok(None),
        }
    }

    #[instrument]
    pub fn drain(&mut self) -> AppResult<()> {
        loop {
            match self.process_output()? {
                Some(item) => self.enqueue(item),
                None => break,
            }
        }
        Ok(())
    }

    fn enqueue(&mut self, item: DecodeOutput) {
        if self.queue.len() < self.cfg.buffering.max_frames || self.cfg.buffering.max_frames == 0 {
            self.queue.push_back(item);
            return;
        }
        match self.cfg.buffering.drop_policy {
            DropPolicy::DropOldest => {
                let _ = self.queue.pop_front();
                self.queue.push_back(item);
            }
            DropPolicy::DropNewest => {
                // Drop the new item, keep existing queue intact
            }
        }
    }

    fn create_nal_sample(nal_data: &[u8], timestamp_100ns: i64) -> AppResult<IMFSample> {
        unsafe {
            let buffer: IMFMediaBuffer = MFCreateMemoryBuffer(nal_data.len() as u32)
                .map_err(|_| AppError::MediaFoundation("Failed to create NAL buffer"))?;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            buffer
                .Lock(&mut ptr, None, None)
                .map_err(|_| AppError::MediaFoundation("Failed to lock NAL buffer"))?;
            std::ptr::copy_nonoverlapping(nal_data.as_ptr(), ptr, nal_data.len());
            buffer
                .Unlock()
                .map_err(|_| AppError::MediaFoundation("Failed to unlock NAL buffer"))?;
            buffer
                .SetCurrentLength(nal_data.len() as u32)
                .map_err(|_| AppError::MediaFoundation("Failed to set NAL buffer length"))?;

            let sample: IMFSample = MFCreateSample()
                .map_err(|_| AppError::MediaFoundation("Failed to create input sample"))?;
            sample
                .AddBuffer(&buffer)
                .map_err(|_| AppError::MediaFoundation("Failed to add buffer to sample"))?;
            let _ = sample.SetSampleTime(timestamp_100ns);
            Ok(sample)
        }
    }

    fn process_input(&self, sample: IMFSample) -> AppResult<()> {
        unsafe {
            self.transform
                .ProcessInput(0, &sample, 0)
                .map_err(|_| AppError::MediaFoundation("Decoder ProcessInput failed"))?;
        }
        Ok(())
    }

    fn process_output(&mut self) -> AppResult<Option<DecodeOutput>> {
        unsafe {
            let mut out = MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: None, dwStatus: 0, pEvents: None };
            let mut status = 0u32;
            let hr = self.transform.ProcessOutput(0, 1, &mut out, &mut status);
            if hr.is_err() {
                let code = hr.0 as u32;
                if code == 0xC00D6D72 { // MF_E_TRANSFORM_NEED_MORE_INPUT
                    return Ok(None);
                }
                return Err(AppError::MediaFoundation("Decoder ProcessOutput failed"));
            }

            if status & MFT_PROCESS_OUTPUT_STATUS_NEW_STREAMS.0 != 0 {
                // A format change has occurred. Re-read current output type.
                if let Ok(fmt) = self.read_current_format() {
                    self.current_format = fmt;
                    return Ok(Some(DecodeOutput::Event(DecoderEvent::Reconfigured(fmt))));
                }
            }

            let sample = match out.pSample {
                Some(s) => s,
                None => return Ok(None),
            };

            // Determine keyframe by scanning NAL units in input is non-trivial here; fall back to false.
            let timestamp = sample.GetSampleTime().unwrap_or(0);
            let frame = self.extract_cpu_frame(&sample, timestamp)?;
            Ok(Some(DecodeOutput::Frame(frame)))
        }
    }

    fn read_current_format(&self) -> AppResult<VideoFormat> {
        unsafe {
            let out_type = self
                .transform
                .GetOutputCurrentType(0)
                .map_err(|_| AppError::MediaFoundation("Failed to get current output type"))?;

            let mut frame_size: u64 = 0;
            out_type
                .GetUINT64(&MF_MT_FRAME_SIZE, &mut frame_size)
                .ok();
            let width = (frame_size >> 32) as u32;
            let height = (frame_size & 0xffffffff) as u32;

            // Stride is not guaranteed; estimate as width for tightly packed formats.
            let stride = width;

            Ok(VideoFormat { width, height, pixel_format: self.current_format.pixel_format, stride })
        }
    }

    fn extract_cpu_frame(&mut self, sample: &IMFSample, timestamp: i64) -> AppResult<DecodedFrame> {
        unsafe {
            let buffer: IMFMediaBuffer = sample
                .GetBufferByIndex(0)
                .map_err(|_| AppError::MediaFoundation("Failed to get output buffer"))?;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut length = 0u32;
            buffer
                .Lock(&mut ptr, None, Some(&mut length))
                .map_err(|_| AppError::MediaFoundation("Failed to lock output buffer"))?;

            let bytes = std::slice::from_raw_parts(ptr, length as usize).to_vec();
            buffer
                .Unlock()
                .map_err(|_| AppError::MediaFoundation("Failed to unlock output buffer"))?;

            // Update current format lazily if unknown
            if self.current_format.width == 0 || self.current_format.height == 0 {
                if let Ok(fmt) = self.read_current_format() {
                    self.current_format = fmt;
                }
            }

            Ok(DecodedFrame {
                data: bytes,
                timestamp,
                width: self.current_format.width,
                height: self.current_format.height,
                stride: if self.current_format.stride != 0 { self.current_format.stride } else { self.current_format.width },
                pixel_format: self.current_format.pixel_format,
                is_keyframe: false,
            })
        }
    }
}

unsafe impl Send for H264Decoder {}
