/// Generate a simple NV12 test pattern frame.
/// Y plane: gradient based on frame number. UV plane: constant (gray).
pub fn generate_nv12_frame(width: u32, height: u32, frame_num: u64) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = w * (h / 2); // NV12: interleaved UV at half height
    let mut buf = vec![0u8; y_size + uv_size];

    // Y plane: horizontal gradient shifted by frame number
    let shift = (frame_num % 256) as u8;
    for y in 0..h {
        for x in 0..w {
            let luma = ((x as u16 * 255 / w.max(1) as u16) as u8).wrapping_add(shift);
            buf[y * w + x] = luma;
        }
    }

    // UV plane: neutral (128 = no color)
    for i in 0..uv_size {
        buf[y_size + i] = 128;
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nv12_frame_size() {
        let frame = generate_nv12_frame(640, 480, 0);
        // NV12: Y = 640*480, UV = 640*240
        assert_eq!(frame.len(), 640 * 480 + 640 * 240);
    }

    #[test]
    fn test_frames_differ() {
        let f0 = generate_nv12_frame(64, 64, 0);
        let f1 = generate_nv12_frame(64, 64, 1);
        assert_ne!(f0, f1);
    }
}
