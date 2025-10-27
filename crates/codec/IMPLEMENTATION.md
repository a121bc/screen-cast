# H.264 Encoder Implementation Details

This document describes the technical implementation of the Windows Media Foundation H.264 encoder.

## Architecture

### Core Components

1. **MfContext**: Manages COM and Media Foundation initialization/cleanup
   - Initializes COM with `COINIT_MULTITHREADED`
   - Calls `MFStartup` to initialize Media Foundation
   - Properly cleans up with `MFShutdown` and `CoUninitialize` in Drop
   - Wrapped in `Arc` for shared ownership across threads

2. **H264Encoder**: Main encoder structure
   - Holds `IMFTransform` (the actual encoder MFT)
   - Caches stream info for performance
   - Tracks frame count for timestamp calculation
   - Maintains encoder configuration

3. **H264EncoderConfig**: Configuration structure
   - Dimensions (width, height)
   - Bitrate (bits per second)
   - Framerate (frames per second)
   - GOP length (keyframe interval)
   - H.264 profile (Baseline, Main, High)
   - Input pixel format (BGRA, RGBA)

4. **EncodedFrame**: Output structure
   - Raw NAL units in Annex-B format
   - Timestamp in 100-nanosecond units
   - Keyframe flag

### Initialization Flow

```
H264Encoder::new()
  ├─> MfContext::new()
  │     ├─> CoInitializeEx(COINIT_MULTITHREADED)
  │     └─> MFStartup(MFSTARTUP_FULL)
  │
  ├─> create_h264_encoder()
  │     ├─> MFEnumTransforms() to find H.264 encoders
  │     └─> ActivateObject() to create encoder instance
  │
  ├─> configure_encoder()
  │     ├─> create_input_media_type() - BGRA/RGBA format
  │     ├─> SetInputType()
  │     ├─> create_output_media_type() - H.264 format
  │     ├─> SetOutputType()
  │     └─> ProcessMessage() to start encoder
  │
  └─> GetInputStreamInfo() and GetOutputStreamInfo()
```

### Encoding Flow

```
encode(frame_data)
  ├─> Validate frame size
  │
  ├─> create_input_sample()
  │     ├─> MFCreateMemoryBuffer()
  │     ├─> Lock buffer and copy frame data
  │     ├─> Unlock buffer
  │     ├─> MFCreateSample()
  │     ├─> AddBuffer()
  │     └─> SetSampleTime() and SetSampleDuration()
  │
  ├─> ProcessInput() - submit frame to encoder
  │
  └─> Loop: ProcessOutput() - retrieve encoded frames
        ├─> Extract sample data
        ├─> Get timestamp
        ├─> Check if keyframe
        └─> Return EncodedFrame
```

## Media Foundation Concepts

### Media Types

Media types in MF describe the format of media data. Key attributes:

- `MF_MT_MAJOR_TYPE`: Major type (e.g., Video)
- `MF_MT_SUBTYPE`: Specific format (e.g., BGRA, H264)
- `MF_MT_FRAME_SIZE`: Width and height packed into u64
- `MF_MT_FRAME_RATE`: Framerate as ratio (numerator << 32 | denominator)
- `MF_MT_AVG_BITRATE`: Target bitrate
- `MF_MT_INTERLACE_MODE`: Progressive or interlaced
- `MF_MT_MPEG2_PROFILE`: H.264 profile (66, 77, 100)

### Transform (MFT)

A Media Foundation Transform processes media data:

- **Input**: Raw video frames (BGRA/RGBA)
- **Output**: Compressed H.264 NAL units
- **Modes**: Synchronous or asynchronous
- **Buffering**: May buffer multiple frames before output

### Samples and Buffers

- `IMFSample`: Container for media data with metadata (timestamp, duration)
- `IMFMediaBuffer`: Actual data storage
- Multiple buffers can be attached to a single sample

## Color Space Handling

The encoder accepts BGRA or RGBA input and converts internally to the format needed by the hardware encoder (typically NV12 or YUV420). This conversion is handled transparently by Media Foundation.

### Pixel Format GUIDs

- **BGRA**: `MFVideoFormat_ARGB32` (0x42475241)
- **RGBA**: `MFVideoFormat_ABGR32` (0x41424752)
- **H.264**: `MFVideoFormat_H264` (0x34363248)

## NAL Unit Format

The encoder outputs NAL units in Annex-B format:

```
[Start Code] [NAL Header] [NAL Payload]
```

### Start Codes

- 4-byte: `0x00 0x00 0x00 0x01`
- 3-byte: `0x00 0x00 0x01`

### NAL Header

First byte after start code:
```
| Forbidden | Ref IDC | Type |
|    1 bit  | 2 bits  | 5 bits|
```

NAL Type (lower 5 bits):
- 1: Non-IDR slice (P or B frame)
- 5: IDR slice (keyframe)
- 7: SPS (Sequence Parameter Set)
- 8: PPS (Picture Parameter Set)

### Typical IDR Frame

```
[0x00 0x00 0x00 0x01] [SPS]
[0x00 0x00 0x00 0x01] [PPS]
[0x00 0x00 0x00 0x01] [IDR Slice]
```

## Thread Safety

- `H264Encoder` is `Send` - can be moved between threads
- Not `Sync` - requires `Mutex` for shared access
- `MfContext` uses `Arc` for shared ownership
- COM is initialized with `COINIT_MULTITHREADED`

## Error Handling

All operations return `AppResult<T>`:
- Configuration validation errors
- Media Foundation API failures
- Buffer operations errors

Critical operations that must succeed:
1. COM initialization
2. Media Foundation startup
3. Encoder enumeration and activation
4. Media type configuration

## Performance Considerations

### Hardware Acceleration

The encoder requests hardware acceleration through:
- `MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS` attribute
- `MFT_ENUM_FLAG_SORTANDFILTER` to prioritize hardware encoders

Detection is automatic; fallback to software encoder is transparent.

### Buffering

The encoder may buffer several frames before producing output:
- Initial buffering for B-frames (if enabled)
- GOP structure analysis
- Rate control

Always call `flush()` at the end to retrieve buffered frames.

### Memory Management

- Input frames are copied into Media Foundation buffers
- Output is copied from Media Foundation to Vec<u8>
- No zero-copy optimization currently implemented

Potential optimization: Use `IMFDXGIBuffer` for GPU memory sharing with DXGI capture pipeline.

## Limitations and Future Work

### Current Limitations

1. **Input formats**: Only 32-bit BGRA/RGBA
2. **Output format**: Annex-B only (no AVCC/MP4)
3. **GOP structure**: No explicit control over B-frames
4. **Rate control**: No mode selection (CBR/VBR/CQ)
5. **No look-ahead**: Encoder doesn't analyze future frames

### Potential Enhancements

1. **Additional input formats**:
   - NV12 (more efficient for hardware)
   - YUV420P (standard format)
   - YUY2 (4:2:2 chroma)

2. **AVCC output format**:
   - Convert Annex-B to AVCC for MP4 container
   - Extract and cache SPS/PPS

3. **Advanced encoding controls**:
   - Quantization parameter (QP) control
   - Scene change detection
   - Temporal layering for SVC
   - Intra refresh

4. **Performance optimizations**:
   - Zero-copy GPU integration
   - Async encoding with dedicated thread
   - Batch processing

5. **Quality and rate control**:
   - Constant quality mode
   - Variable bitrate with min/max
   - Look-ahead for better compression
   - Adaptive bitrate

6. **Diagnostic features**:
   - Encoder capability enumeration
   - Statistics (QP, frame types, bitrate)
   - Quality metrics (PSNR, SSIM)

## Testing Strategy

### Unit Tests

1. Configuration validation
2. Encoder creation with various parameters
3. Frame encoding and output verification

### Integration Tests

1. SPS/PPS presence verification
2. Multiple resolution support
3. Profile compatibility
4. Pixel format handling

### Performance Tests

1. Throughput measurement
2. Latency analysis
3. Resource utilization
4. Hardware vs software comparison

### Conformance

Encoded streams should be validated with:
- FFmpeg/FFprobe for stream analysis
- H.264 conformance checker
- Decoder compatibility tests

## References

- [Microsoft Media Foundation Documentation](https://docs.microsoft.com/en-us/windows/win32/medfound/microsoft-media-foundation-sdk)
- [H.264 Encoder MFT](https://docs.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder)
- [ITU-T H.264 Specification](https://www.itu.int/rec/T-REC-H.264)
- [Windows crate documentation](https://docs.rs/windows/)
