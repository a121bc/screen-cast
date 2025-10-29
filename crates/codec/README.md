# Codec Crate

The `codec` crate provides encoder and decoder implementations for the Windows Capture Workspace, with a focus on hardware-accelerated video encoding using Windows Media Foundation.

## Features

### H.264 Encoder (Windows Media Foundation)

The `h264` module provides a production-ready H.264 encoder using Windows Media Foundation Transform (MFT). This encoder leverages hardware acceleration when available and provides fine-grained control over encoding parameters.

### H.264 Decoder (Windows Media Foundation)

The `h264` module also provides a Windows Media Foundation H.264 decoder capable of accepting Annex-B NAL unit streams and producing decoded frames suitable for rendering.

Key features:

- NAL unit input (Annex-B with start codes)
- Timestamp passthrough for latency calculation
- Reconfiguration handling on resolution changes (SPS updates)
- Buffering strategy with drop policies (oldest/newest)
- Output to CPU buffers (NV12 or BGRA). DXGI output is planned and API-stubbed.

Basic usage:

```rust
use codec::h264::{H264Decoder, H264DecoderConfig, DecoderOutput, DecodeOutput};

let mut decoder = H264Decoder::new(H264DecoderConfig { output: DecoderOutput::CpuNv12, ..Default::default() })?;

// Feed NAL units with timestamps in 100-ns units (Media Foundation timebase)
let timestamp_100ns: i64 = 0;
decoder.push_nal(&annexb_bytes, timestamp_100ns)?;

while let Some(out) = decoder.try_get_output()? {
    match out {
        DecodeOutput::Frame(frame) => {
            println!(
                "decoded {}x{} {:?} frame ({} bytes) @{}", 
                frame.width, frame.height, frame.pixel_format, frame.data.len(), frame.timestamp
            );
        }
        DecodeOutput::Event(evt) => {
            println!("decoder event: {:?}", evt);
        }
    }
}
```

Notes:

- The decoder negotiates output format automatically. Resolution is learned from SPS and may change at runtime; a `Reconfigured` event is emitted and subsequent frames reflect the new format.
- Timestamp passthrough: Sample timestamps set on input are propagated to output frames by the decoder to support precise end-to-end latency tracking.
- Buffering: Configure `buffering.max_frames` and `drop_policy` to bound latency during spikes.
- DXGI texture output requires a D3D11 device manager and is not wired in this initial implementation; CPU frame output is recommended for integration and testing.

#### Key Features

- **Hardware Acceleration**: Automatically uses hardware-accelerated encoding when available
- **Configurable Parameters**:
  - Resolution (width, height)
  - Bitrate (bits per second)
  - Framerate (frames per second)
  - GOP (Group of Pictures) length
  - H.264 Profile (Baseline, Main, High)
  - Input pixel format (BGRA, RGBA)
- **Annex-B Output**: Emits NAL units in Annex-B format with start codes (0x00 0x00 0x00 0x01)
- **Timestamp Management**: Tracks and assigns timestamps to encoded frames
- **Thread Safety**: Safe to send across threads
- **Proper Resource Management**: Automatic cleanup of Media Foundation resources

#### Usage Example

```rust
use codec::h264::{H264Encoder, H264EncoderConfig, H264Profile, PixelFormat};

let config = H264EncoderConfig {
    width: 1920,
    height: 1080,
    bitrate: 8_000_000,
    framerate: 60,
    gop_length: 60,
    profile: H264Profile::High,
    pixel_format: PixelFormat::Bgra,
};

let mut encoder = H264Encoder::new(config)?;

// Encode a frame (BGRA format)
let frame_data: Vec<u8> = get_frame_from_capture();
let encoded_frames = encoder.encode(&frame_data)?;

for frame in encoded_frames {
    println!("Encoded {} bytes at timestamp {}", frame.data.len(), frame.timestamp);
    println!("Is keyframe: {}", frame.is_keyframe);
    
    // Parse NAL units
    let nal_units = codec::h264::parse_nal_units(&frame.data);
    for (start, end, nal_type) in nal_units {
        println!("NAL type: {} ({}..{})", nal_type, start, end);
    }
}

// Flush remaining frames
let remaining = encoder.flush()?;
```

#### Configuration

**H264EncoderConfig**

- `width`: Frame width in pixels (must be > 0)
- `height`: Frame height in pixels (must be > 0)
- `bitrate`: Target bitrate in bits per second (must be > 0)
- `framerate`: Target framerate in frames per second (must be > 0)
- `gop_length`: GOP size (keyframe interval)
- `profile`: H.264 profile (Baseline, Main, or High)
- `pixel_format`: Input pixel format (Bgra or Rgba)

**H264Profile**

- `Baseline`: H.264 Baseline Profile (66) - Maximum compatibility
- `Main`: H.264 Main Profile (77) - Good balance
- `High`: H.264 High Profile (100) - Best compression

**PixelFormat**

- `Bgra`: Blue-Green-Red-Alpha (32-bit, common for Windows)
- `Rgba`: Red-Green-Blue-Alpha (32-bit)

#### Encoded Output

The encoder produces `EncodedFrame` structures containing:

- `data`: Raw NAL units in Annex-B format
- `timestamp`: Frame timestamp in 100-nanosecond units
- `is_keyframe`: Whether this frame is a keyframe (IDR)

#### NAL Unit Parsing

The module provides a `parse_nal_units` utility to extract individual NAL units from Annex-B streams:

```rust
use codec::h264::{parse_nal_units, NAL_TYPE_SPS, NAL_TYPE_PPS, NAL_TYPE_IDR};

let nal_units = parse_nal_units(&encoded_data);
for (start, end, nal_type) in nal_units {
    match nal_type {
        NAL_TYPE_SPS => println!("Found SPS at {}..{}", start, end),
        NAL_TYPE_PPS => println!("Found PPS at {}..{}", start, end),
        NAL_TYPE_IDR => println!("Found IDR at {}..{}", start, end),
        _ => println!("Found NAL type {} at {}..{}", nal_type, start, end),
    }
}
```

#### NAL Unit Types

Common NAL unit types:
- `NAL_TYPE_SPS` (7): Sequence Parameter Set
- `NAL_TYPE_PPS` (8): Picture Parameter Set
- `NAL_TYPE_IDR` (5): IDR (keyframe) slice
- `NAL_TYPE_NON_IDR` (1): Non-IDR slice

#### Thread Safety

The `H264Encoder` is `Send` and can be moved between threads. However, it is not `Sync` and should not be shared between threads without proper synchronization (e.g., `Mutex`).

#### Performance Considerations

1. **Hardware Acceleration**: The encoder will use hardware acceleration when available. Performance varies significantly between software and hardware encoding.

2. **Frame Buffering**: The encoder may buffer several frames before producing output. Always call `flush()` to retrieve remaining frames.

3. **Resolution Impact**: Higher resolutions require more processing power. Test on target hardware to ensure real-time performance.

4. **Bitrate vs Quality**: Higher bitrates improve quality but increase bandwidth requirements. Adjust based on network capacity and quality requirements.

5. **GOP Length**: Longer GOP lengths improve compression but increase latency and reduce seek performance. For streaming, use GOP lengths equal to or less than framerate.

#### Error Handling

All encoding operations return `AppResult<T>` which wraps `Result<T, AppError>`. Common errors:

- Invalid configuration (zero dimensions, bitrate, framerate)
- Media Foundation initialization failure
- No H.264 encoder available on system
- Invalid frame data size
- Encoder processing errors

#### Testing

Run the test suite:

```bash
cargo test --package codec
```

The test suite includes:
- Basic encoder creation and configuration validation
- Frame encoding with various resolutions and profiles
- SPS/PPS presence verification
- NAL unit parsing
- Performance benchmarks

#### Limitations

- **Windows Only**: Requires Windows 8 or newer with Media Foundation support
- **Input Formats**: Only BGRA and RGBA 32-bit formats are supported
- **Output Format**: Only Annex-B format is supported (not AVCC/MP4 format)
- **No B-frames Control**: GOP structure is determined by Media Foundation

#### Future Enhancements

Potential improvements:
- Support for additional input formats (NV12, YUV420)
- AVCC/MP4 format output option
- Rate control mode selection (CBR, VBR, CQ)
- Region of Interest (ROI) encoding
- Dynamic bitrate adjustment
- Encoder capabilities enumeration

## Placeholder Encoder/Decoder

The crate also provides placeholder `Encoder` and `Decoder` types for compatibility with existing code. These are simple pass-through implementations that will be deprecated once the Media Foundation encoder is fully integrated.

## Dependencies

- `windows`: Windows API bindings for Media Foundation
- `shared`: Shared error types and utilities
- `tracing`: Structured logging
- `serde`: Configuration serialization
- `thiserror`: Error handling

## License

MIT OR Apache-2.0
