use codec::h264::{
    H264Encoder, H264EncoderConfig, H264Profile, PixelFormat,
    parse_nal_units, NAL_TYPE_SPS, NAL_TYPE_PPS, NAL_TYPE_IDR,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Windows Media Foundation H.264 Encoder Example\n");

    let config = H264EncoderConfig {
        width: 1280,
        height: 720,
        bitrate: 4_000_000,
        framerate: 30,
        gop_length: 30,
        profile: H264Profile::High,
        pixel_format: PixelFormat::Bgra,
    };

    println!("Creating encoder with configuration:");
    println!("  Resolution: {}x{}", config.width, config.height);
    println!("  Bitrate: {} bps", config.bitrate);
    println!("  Framerate: {} fps", config.framerate);
    println!("  GOP length: {}", config.gop_length);
    println!("  Profile: {:?}", config.profile);
    println!("  Pixel format: {:?}\n", config.pixel_format);

    let mut encoder = H264Encoder::new(config.clone())?;
    println!("Encoder created successfully!\n");

    let frame_data = create_test_frame(config.width, config.height);
    println!("Generated test frame: {} bytes\n", frame_data.len());

    println!("Encoding frames...");
    let mut total_output = 0;
    let mut frame_count = 0;

    for i in 0..10 {
        let encoded_frames = encoder.encode(&frame_data)?;
        
        if !encoded_frames.is_empty() {
            println!("\nFrame {}: {} encoded outputs", i, encoded_frames.len());
            
            for (idx, frame) in encoded_frames.iter().enumerate() {
                println!("  Output {}: {} bytes, timestamp: {}, keyframe: {}",
                    idx, frame.data.len(), frame.timestamp, frame.is_keyframe);
                
                let nal_units = parse_nal_units(&frame.data);
                print!("    NAL units: ");
                for (_, _, nal_type) in &nal_units {
                    let name = match *nal_type {
                        NAL_TYPE_SPS => "SPS",
                        NAL_TYPE_PPS => "PPS",
                        NAL_TYPE_IDR => "IDR",
                        1 => "P/B",
                        _ => "Other",
                    };
                    print!("{} ", name);
                }
                println!();
                
                total_output += frame.data.len();
                frame_count += 1;
            }
        } else {
            println!("Frame {}: No output (encoder buffering)", i);
        }
    }

    println!("\nFlushing encoder...");
    let flushed = encoder.flush()?;
    println!("Flushed {} frames", flushed.len());
    
    for frame in flushed {
        total_output += frame.data.len();
        frame_count += 1;
    }

    println!("\n=== Summary ===");
    println!("Total frames encoded: {}", frame_count);
    println!("Total output size: {} bytes ({:.2} KB)", total_output, total_output as f64 / 1024.0);
    println!("Average frame size: {:.2} KB", (total_output as f64 / frame_count as f64) / 1024.0);

    Ok(())
}

fn create_test_frame(width: u32, height: u32) -> Vec<u8> {
    let size = (width * height * 4) as usize;
    let mut frame = vec![0u8; size];

    for y in 0..height {
        for x in 0..width {
            let offset = ((y * width + x) * 4) as usize;
            frame[offset] = ((x * 255) / width) as u8;
            frame[offset + 1] = ((y * 255) / height) as u8;
            frame[offset + 2] = 128;
            frame[offset + 3] = 255;
        }
    }

    frame
}
