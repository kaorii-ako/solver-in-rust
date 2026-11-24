//! Colour maps and a tiny BMP encoder, so reports need no image crate.

const STOPS: [(f32, [f32; 3]); 6] = [
    (0.0, [0.10, 0.12, 0.45]),
    (0.25, [0.13, 0.42, 0.78]),
    (0.5, [0.20, 0.72, 0.62]),
    (0.75, [0.95, 0.80, 0.25]),
    (0.9, [0.92, 0.42, 0.18]),
    (1.0, [0.70, 0.10, 0.12]),
];

pub fn colormap(t: f32) -> [u8; 3] {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    for w in STOPS.windows(2) {
        let (a, ca) = w[0];
        let (b, cb) = w[1];
        if t <= b {
            let s = (t - a) / (b - a);
            return [0, 1, 2].map(|i| ((ca[i] + s * (cb[i] - ca[i])) * 255.0) as u8);
        }
    }
    [178, 26, 31]
}

pub const SOLID_RGB: [u8; 3] = [55, 55, 60];

/// Colours a slice of |u|/U: negative cells are solid.
pub fn slice_rgb(values: &[f32], vmax: f32) -> Vec<[u8; 3]> {
    values
        .iter()
        .map(|&v| if v < 0.0 { SOLID_RGB } else { colormap(v / vmax) })
        .collect()
}

/// 24-bit BMP, rows given top to bottom.
pub fn bmp(width: usize, height: usize, rgb: &[[u8; 3]]) -> Vec<u8> {
    let row = (width * 3 + 3) & !3;
    let size = 54 + row * height;
    let mut b = Vec::with_capacity(size);
    b.extend(b"BM");
    b.extend((size as u32).to_le_bytes());
    b.extend([0u8; 4]);
    b.extend(54u32.to_le_bytes());
    b.extend(40u32.to_le_bytes());
    b.extend((width as i32).to_le_bytes());
    b.extend((height as i32).to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(24u16.to_le_bytes());
    b.extend([0u8; 24]);
    for y in (0..height).rev() {
        let start = b.len();
        for x in 0..width {
            let [r, g, bl] = rgb[y * width + x];
            b.extend([bl, g, r]);
        }
        b.resize(start + row, 0);
    }
    b
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_known_values() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn bmp_size_is_padded() {
        assert_eq!(bmp(3, 2, &[[0, 0, 0]; 6]).len(), 54 + 12 * 2);
    }

    #[test]
    fn colormap_ends() {
        assert_ne!(colormap(0.0), colormap(1.0));
        assert_eq!(colormap(f32::NAN), colormap(0.0));
    }
}
