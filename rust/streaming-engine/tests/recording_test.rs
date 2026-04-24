//! End-to-end integration tests for session recording.
//! Covers Recorder (Annex B) + AudioRecorder (WAV) behavior when written to
//! real tempfiles — complements the in-crate unit tests by validating the
//! full write-to-close flow, including file layout assertions against bytes
//! on disk.

use std::fs;
use std::path::PathBuf;
use streaming_engine::recording::{AudioRecorder, Recorder};

fn unique_tmp(name: &str) -> PathBuf {
    let pid = std::process::id();
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("fvp_it_rec_{name}_{pid}_{ns}"))
}

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
}

/// Split a raw Annex B byte stream at every 4-byte start code (00 00 00 01).
/// Matches how ffmpeg / VLC scan the stream.
fn split_annex_b(data: &[u8]) -> Vec<&[u8]> {
    let start = [0x00, 0x00, 0x00, 0x01];
    let mut out = Vec::new();
    let mut i = 0;
    let mut last = None;
    while i + 4 <= data.len() {
        if data[i..i + 4] == start {
            if let Some(prev) = last {
                out.push(&data[prev..i]);
            }
            last = Some(i + 4);
            i += 4;
        } else {
            i += 1;
        }
    }
    if let Some(prev) = last {
        out.push(&data[prev..]);
    }
    out
}

#[test]
fn test_annex_b_roundtrip_multiple_nals() {
    let path = unique_tmp("annexb").with_extension("h265");
    let _ = fs::remove_file(&path);

    let nals: Vec<Vec<u8>> = vec![
        vec![0x40, 0x01, 0x0C, 0x01], // VPS
        vec![0x42, 0x01, 0x01, 0x01], // SPS
        vec![0x44, 0x01, 0xC0],       // PPS
        vec![0x26, 0x01, 0xAF, 0x00, 0x22, 0x44], // IDR
        (0..200u8).map(|i| i.wrapping_mul(7)).collect(), // random-ish payload
    ];

    {
        let mut rec = Recorder::open(&path).expect("open recorder");
        for nal in &nals {
            rec.write_nal(nal);
        }
        assert_eq!(rec.nal_count(), nals.len() as u64);
        // drop → close() → flush
    }

    let data = fs::read(&path).expect("read back recording");
    let parsed = split_annex_b(&data);
    assert_eq!(parsed.len(), nals.len(), "parsed NAL count mismatch");
    for (i, (got, want)) in parsed.iter().zip(nals.iter()).enumerate() {
        assert_eq!(*got, want.as_slice(), "NAL {i} body mismatch");
    }

    let _ = fs::remove_file(&path);
}

#[test]
fn test_annex_b_passthrough_with_existing_start_code() {
    let path = unique_tmp("passthru").with_extension("h265");
    let _ = fs::remove_file(&path);

    // NAL already prefixed with 3-byte start code — should pass through unchanged.
    let three_byte = [0x00, 0x00, 0x01, 0x42, 0x01];
    let four_byte = [0x00, 0x00, 0x00, 0x01, 0x26, 0x01, 0xFF];

    {
        let mut rec = Recorder::open(&path).unwrap();
        rec.write_nal(&three_byte);
        rec.write_nal(&four_byte);
    }

    let data = fs::read(&path).unwrap();
    // Expect the bytes exactly as written, no additional prefix.
    let mut expected = Vec::new();
    expected.extend_from_slice(&three_byte);
    expected.extend_from_slice(&four_byte);
    assert_eq!(data, expected);

    let _ = fs::remove_file(&path);
}

#[test]
fn test_wav_header_finalized_after_close() {
    let path = unique_tmp("wav").with_extension("wav");
    let _ = fs::remove_file(&path);

    {
        let mut rec = AudioRecorder::open(&path, 48000, 2).expect("open audio");
        // Write 480 stereo samples → 960 f32 → 1920 bytes of i16 PCM
        let frame: Vec<f32> = (0..960).map(|i| (i as f32 * 0.001).sin()).collect();
        rec.write_pcm_f32(&frame);
        assert_eq!(rec.data_bytes(), 1920);
    }

    let data = fs::read(&path).unwrap();
    assert!(data.len() >= 44, "header must be at least 44 bytes");
    assert_eq!(&data[0..4], b"RIFF");
    assert_eq!(&data[8..12], b"WAVE");
    assert_eq!(&data[36..40], b"data");
    // data size = total - 44 header bytes
    let data_size = read_u32_le(&data, 40);
    assert_eq!(data_size as usize, data.len() - 44);
    // RIFF size = 36 + data size (post-finalize)
    let riff_size = read_u32_le(&data, 4);
    assert_eq!(riff_size, 36 + data_size);

    // Sample rate / channel / bits_per_sample round-trip
    assert_eq!(read_u32_le(&data, 24), 48000); // sample rate
    assert_eq!(u16::from_le_bytes([data[22], data[23]]), 2); // channels
    assert_eq!(u16::from_le_bytes([data[34], data[35]]), 16); // bits

    let _ = fs::remove_file(&path);
}

#[test]
fn test_wav_empty_has_zero_data_size() {
    let path = unique_tmp("wav_empty").with_extension("wav");
    let _ = fs::remove_file(&path);
    {
        let _rec = AudioRecorder::open(&path, 48000, 2).unwrap();
    }
    let data = fs::read(&path).unwrap();
    assert_eq!(data.len(), 44, "empty WAV = header only");
    assert_eq!(read_u32_le(&data, 40), 0, "data size = 0");
    assert_eq!(read_u32_le(&data, 4), 36, "RIFF size = 36 + 0");
    let _ = fs::remove_file(&path);
}

#[test]
fn test_recorder_survives_open_in_nested_subdir() {
    // Creates parent dirs automatically.
    let tmp = std::env::temp_dir().join(format!("fvp_it_rec_nested_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let path = tmp.join("a").join("b").join("session.h265");

    {
        let mut rec = Recorder::open(&path).expect("nested dir creation");
        rec.write_nal(&[0x42, 0x01]);
    }
    assert!(path.exists());
    let _ = fs::remove_dir_all(&tmp);
}
