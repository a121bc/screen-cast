use codec::h264::{H264Decoder, H264DecoderConfig, DecoderOutput, DecodeOutput};

#[test]
#[cfg(target_os = "windows")]
fn test_h264_decoder_creation() {
    let cfg = H264DecoderConfig { output: DecoderOutput::CpuNv12, ..Default::default() };
    let decoder = H264Decoder::new(cfg);
    assert!(decoder.is_ok(), "Failed to create H264 decoder: {:?}", decoder.err());
}

// Integration-oriented test stub: requires a valid Annex-B H.264 bitstream with SPS/PPS/IDR.
// This test is ignored by default and serves as a scaffold for future integration testing.
#[test]
#[cfg(target_os = "windows")]
#[ignore]
fn test_h264_decoder_decodes_nal_stream() {
    let cfg = H264DecoderConfig { output: DecoderOutput::CpuNv12, ..Default::default() };
    let mut decoder = H264Decoder::new(cfg).expect("decoder creation");

    // In a real test, load an Annex-B bytestream containing SPS/PPS and at least one IDR frame.
    // For example, read from a file or generate via the encoder and feed here.
    let nal_stream: Vec<u8> = include_bytes!("testdata/sample_annexb.h264").to_vec();

    // Push the entire stream as a single packet for simplicity; decoders accept arbitrary NAL chunking.
    decoder.push_nal(&nal_stream, 0).expect("push NAL");

    let mut got_frame = false;
    while let Some(out) = decoder.try_get_output().expect("poll output") {
        match out {
            DecodeOutput::Frame(frame) => {
                println!("decoded {}x{} {:?} bytes={}", frame.width, frame.height, frame.pixel_format, frame.data.len());
                got_frame = true;
                break;
            }
            DecodeOutput::Event(evt) => {
                println!("event: {:?}", evt);
            }
        }
    }

    assert!(got_frame, "expected at least one decoded frame");
}
