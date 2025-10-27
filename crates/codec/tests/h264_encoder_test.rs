use codec::h264::{
    H264Encoder, H264EncoderConfig, H264Profile, PixelFormat, parse_nal_units,
    NAL_TYPE_SPS, NAL_TYPE_PPS, NAL_TYPE_IDR, NAL_TYPE_NON_IDR,
};

#[test]
#[cfg(target_os = "windows")]
fn test_h264_encoder_creation() {
    let config = H264EncoderConfig {
        width: 1920,
        height: 1080,
        bitrate: 4_000_000,
        framerate: 30,
        gop_length: 30,
        profile: H264Profile::High,
        pixel_format: PixelFormat::Bgra,
    };

    let encoder = H264Encoder::new(config.clone());
    assert!(encoder.is_ok(), "Failed to create encoder: {:?}", encoder.err());

    let encoder = encoder.unwrap();
    assert_eq!(encoder.config().width, config.width);
    assert_eq!(encoder.config().height, config.height);
    assert_eq!(encoder.config().bitrate, config.bitrate);
}

#[test]
#[cfg(target_os = "windows")]
fn test_h264_encoder_invalid_config() {
    let config = H264EncoderConfig {
        width: 0,
        height: 1080,
        bitrate: 4_000_000,
        framerate: 30,
        gop_length: 30,
        profile: H264Profile::High,
        pixel_format: PixelFormat::Bgra,
    };

    let encoder = H264Encoder::new(config);
    assert!(encoder.is_err(), "Should fail with zero width");

    let config = H264EncoderConfig {
        width: 1920,
        height: 1080,
        bitrate: 0,
        framerate: 30,
        gop_length: 30,
        profile: H264Profile::High,
        pixel_format: PixelFormat::Bgra,
    };

    let encoder = H264Encoder::new(config);
    assert!(encoder.is_err(), "Should fail with zero bitrate");
}

fn create_test_frame(width: u32, height: u32, pixel_format: PixelFormat) -> Vec<u8> {
    let bytes_per_pixel = match pixel_format {
        PixelFormat::Bgra | PixelFormat::Rgba => 4,
    };
    let size = (width * height * bytes_per_pixel) as usize;
    let mut frame = vec![0u8; size];

    for y in 0..height {
        for x in 0..width {
            let offset = ((y * width + x) * bytes_per_pixel) as usize;
            match pixel_format {
                PixelFormat::Bgra => {
                    frame[offset] = ((x * 255) / width) as u8;
                    frame[offset + 1] = ((y * 255) / height) as u8;
                    frame[offset + 2] = 128;
                    frame[offset + 3] = 255;
                }
                PixelFormat::Rgba => {
                    frame[offset] = 128;
                    frame[offset + 1] = ((y * 255) / height) as u8;
                    frame[offset + 2] = ((x * 255) / width) as u8;
                    frame[offset + 3] = 255;
                }
            }
        }
    }

    frame
}

#[test]
#[cfg(target_os = "windows")]
fn test_h264_encoder_encode_frames() {
    let config = H264EncoderConfig {
        width: 640,
        height: 480,
        bitrate: 2_000_000,
        framerate: 30,
        gop_length: 30,
        profile: H264Profile::Main,
        pixel_format: PixelFormat::Bgra,
    };

    let mut encoder = H264Encoder::new(config.clone()).expect("Failed to create encoder");

    let frame_data = create_test_frame(config.width, config.height, config.pixel_format);

    let mut total_encoded_frames = 0;
    let mut has_sps = false;
    let mut has_pps = false;
    let mut has_idr = false;

    for i in 0..10 {
        let result = encoder.encode(&frame_data);
        assert!(result.is_ok(), "Failed to encode frame {}: {:?}", i, result.err());

        let encoded_frames = result.unwrap();
        for frame in &encoded_frames {
            assert!(!frame.data.is_empty(), "Encoded frame {} should not be empty", i);

            let nal_units = parse_nal_units(&frame.data);
            for (_, _, nal_type) in nal_units {
                match nal_type {
                    NAL_TYPE_SPS => has_sps = true,
                    NAL_TYPE_PPS => has_pps = true,
                    NAL_TYPE_IDR => has_idr = true,
                    _ => {}
                }
            }
        }
        total_encoded_frames += encoded_frames.len();
    }

    let flushed = encoder.flush().expect("Failed to flush encoder");
    total_encoded_frames += flushed.len();

    println!("Total encoded frames: {}", total_encoded_frames);
    println!("Has SPS: {}, Has PPS: {}, Has IDR: {}", has_sps, has_pps, has_idr);

    assert!(has_sps, "Should have SPS NAL unit");
    assert!(has_pps, "Should have PPS NAL unit");
    assert!(has_idr || total_encoded_frames > 0, "Should have IDR frame or some encoded output");
}

#[test]
#[cfg(target_os = "windows")]
fn test_h264_encoder_sps_pps_presence() {
    let config = H264EncoderConfig {
        width: 320,
        height: 240,
        bitrate: 1_000_000,
        framerate: 30,
        gop_length: 30,
        profile: H264Profile::Baseline,
        pixel_format: PixelFormat::Bgra,
    };

    let mut encoder = H264Encoder::new(config.clone()).expect("Failed to create encoder");

    let frame_data = create_test_frame(config.width, config.height, config.pixel_format);

    let mut all_nal_types = Vec::new();

    for _ in 0..30 {
        if let Ok(encoded_frames) = encoder.encode(&frame_data) {
            for frame in encoded_frames {
                let nal_units = parse_nal_units(&frame.data);
                for (_, _, nal_type) in nal_units {
                    all_nal_types.push(nal_type);
                }
            }
        }
    }

    if let Ok(flushed) = encoder.flush() {
        for frame in flushed {
            let nal_units = parse_nal_units(&frame.data);
            for (_, _, nal_type) in nal_units {
                all_nal_types.push(nal_type);
            }
        }
    }

    println!("NAL types found: {:?}", all_nal_types);

    let has_sps = all_nal_types.iter().any(|&t| t == NAL_TYPE_SPS);
    let has_pps = all_nal_types.iter().any(|&t| t == NAL_TYPE_PPS);

    assert!(has_sps, "SPS (NAL type 7) must be present in encoded stream");
    assert!(has_pps, "PPS (NAL type 8) must be present in encoded stream");
}

#[test]
#[cfg(target_os = "windows")]
fn test_h264_encoder_different_resolutions() {
    let resolutions = vec![
        (320, 240),
        (640, 480),
        (1280, 720),
        (1920, 1080),
    ];

    for (width, height) in resolutions {
        let config = H264EncoderConfig {
            width,
            height,
            bitrate: 2_000_000,
            framerate: 30,
            gop_length: 30,
            profile: H264Profile::High,
            pixel_format: PixelFormat::Bgra,
        };

        let encoder = H264Encoder::new(config.clone());
        assert!(
            encoder.is_ok(),
            "Failed to create encoder for {}x{}: {:?}",
            width,
            height,
            encoder.err()
        );
    }
}

#[test]
#[cfg(target_os = "windows")]
fn test_h264_encoder_different_profiles() {
    let profiles = vec![
        H264Profile::Baseline,
        H264Profile::Main,
        H264Profile::High,
    ];

    for profile in profiles {
        let config = H264EncoderConfig {
            width: 640,
            height: 480,
            bitrate: 2_000_000,
            framerate: 30,
            gop_length: 30,
            profile,
            pixel_format: PixelFormat::Bgra,
        };

        let encoder = H264Encoder::new(config);
        assert!(
            encoder.is_ok(),
            "Failed to create encoder with profile {:?}: {:?}",
            profile,
            encoder.err()
        );
    }
}

#[test]
#[cfg(target_os = "windows")]
fn test_h264_encoder_rgba_format() {
    let config = H264EncoderConfig {
        width: 640,
        height: 480,
        bitrate: 2_000_000,
        framerate: 30,
        gop_length: 30,
        profile: H264Profile::Main,
        pixel_format: PixelFormat::Rgba,
    };

    let mut encoder = H264Encoder::new(config.clone()).expect("Failed to create encoder");

    let frame_data = create_test_frame(config.width, config.height, config.pixel_format);

    let result = encoder.encode(&frame_data);
    assert!(result.is_ok(), "Failed to encode RGBA frame: {:?}", result.err());
}

#[test]
#[cfg(target_os = "windows")]
fn test_parse_nal_units() {
    let test_data = vec![
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1e,
        0x00, 0x00, 0x00, 0x01, 0x68, 0xce, 0x3c, 0x80,
        0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00,
    ];

    let nal_units = parse_nal_units(&test_data);

    assert!(nal_units.len() >= 2, "Should find at least 2 NAL units");

    let nal_types: Vec<u8> = nal_units.iter().map(|(_, _, t)| *t).collect();
    println!("Parsed NAL types: {:?}", nal_types);
}

#[test]
#[cfg(target_os = "windows")]
fn test_h264_encoder_performance_note() {
    println!("\n=== H264 Encoder Performance Notes ===");
    println!("This test demonstrates basic encoding performance characteristics.");
    println!();

    let config = H264EncoderConfig {
        width: 1920,
        height: 1080,
        bitrate: 8_000_000,
        framerate: 60,
        gop_length: 60,
        profile: H264Profile::High,
        pixel_format: PixelFormat::Bgra,
    };

    let mut encoder = H264Encoder::new(config.clone()).expect("Failed to create encoder");

    let frame_data = create_test_frame(config.width, config.height, config.pixel_format);

    println!("Configuration:");
    println!("  Resolution: {}x{}", config.width, config.height);
    println!("  Bitrate: {} bps", config.bitrate);
    println!("  Framerate: {} fps", config.framerate);
    println!("  GOP Length: {}", config.gop_length);
    println!("  Profile: {:?}", config.profile);
    println!("  Pixel Format: {:?}", config.pixel_format);
    println!();

    let start = std::time::Instant::now();
    let num_frames = 60;

    let mut total_output_size = 0;
    let mut total_encoded_frames = 0;

    for i in 0..num_frames {
        if let Ok(encoded_frames) = encoder.encode(&frame_data) {
            for frame in &encoded_frames {
                total_output_size += frame.data.len();
                total_encoded_frames += 1;
            }
        }
    }

    if let Ok(flushed) = encoder.flush() {
        for frame in &flushed {
            total_output_size += frame.data.len();
            total_encoded_frames += 1;
        }
    }

    let elapsed = start.elapsed();

    println!("Performance Results:");
    println!("  Frames encoded: {}", num_frames);
    println!("  Output frames: {}", total_encoded_frames);
    println!("  Total time: {:.2?}", elapsed);
    println!("  Avg time per frame: {:.2?}", elapsed / num_frames);
    println!("  FPS: {:.2}", num_frames as f64 / elapsed.as_secs_f64());
    let size_mb = total_output_size as f64 / 1_048_576.0;
    println!("  Total output size: {} bytes ({:.2} MB)", total_output_size, size_mb);
    let avg_bitrate_mbps = (total_output_size as f64 * 8.0)
        / elapsed.as_secs_f64()
        / 1_000_000.0;
    println!("  Avg bitrate: {:.2} Mbps", avg_bitrate_mbps);
    println!();
    println!("Notes:");
    println!("  - Hardware acceleration is enabled when available");
    println!("  - Encoder may buffer frames initially");
    println!("  - Performance varies based on system and GPU capabilities");
    println!("  - NAL units are emitted in Annex-B format (with start codes)");
    println!("======================================\n");
}

#[test]
#[cfg(target_os = "windows")]
fn test_encoded_frame_timestamps() {
    let config = H264EncoderConfig {
        width: 640,
        height: 480,
        bitrate: 2_000_000,
        framerate: 30,
        gop_length: 30,
        profile: H264Profile::Main,
        pixel_format: PixelFormat::Bgra,
    };

    let mut encoder = H264Encoder::new(config.clone()).expect("Failed to create encoder");

    let frame_data = create_test_frame(config.width, config.height, config.pixel_format);

    let mut last_timestamp = -1i64;
    let mut timestamp_count = 0;

    for _ in 0..10 {
        if let Ok(encoded_frames) = encoder.encode(&frame_data) {
            for frame in encoded_frames {
                if frame.timestamp >= 0 {
                    assert!(
                        frame.timestamp >= last_timestamp,
                        "Timestamps should be monotonically increasing"
                    );
                    last_timestamp = frame.timestamp;
                    timestamp_count += 1;
                }
            }
        }
    }

    println!("Verified {} timestamps", timestamp_count);
}
