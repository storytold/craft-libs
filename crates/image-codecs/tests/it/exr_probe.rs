//! `probe_size` reports the size `decode` opens an EXR at, including a data window staged on
//! a larger display window.

use craft_image_codecs::{decode, probe_size};

fn attr(out: &mut Vec<u8>, name: &str, ty: &str, value: &[u8]) {
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out.extend_from_slice(ty.as_bytes());
    out.push(0);
    out.extend_from_slice(&(value.len() as i32).to_le_bytes());
    out.extend_from_slice(value);
}

fn box2i(b: [i32; 4]) -> Vec<u8> {
    b.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A single-part, uncompressed, scanline RGBA float32 EXR. Windows are inclusive
/// `[x_min, y_min, x_max, y_max]`; every pixel is 0.8 / 0.4 / 0.2 / 1.0.
fn exr(display: [i32; 4], data: [i32; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x0131_2f76u32.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    // Channels are stored in name order: A, B, G, R; pixel type 2 is float32.
    let names = ["A", "B", "G", "R"];
    let mut chlist = Vec::new();
    for name in names {
        chlist.extend_from_slice(name.as_bytes());
        chlist.push(0);
        chlist.extend_from_slice(&2i32.to_le_bytes());
        chlist.extend_from_slice(&[0; 4]); // pLinear and reserved
        chlist.extend_from_slice(&1i32.to_le_bytes());
        chlist.extend_from_slice(&1i32.to_le_bytes());
    }
    chlist.push(0);
    attr(&mut out, "channels", "chlist", &chlist);
    attr(&mut out, "compression", "compression", &[0]);
    attr(&mut out, "dataWindow", "box2i", &box2i(data));
    attr(&mut out, "displayWindow", "box2i", &box2i(display));
    attr(&mut out, "lineOrder", "lineOrder", &[0]);
    attr(&mut out, "pixelAspectRatio", "float", &1f32.to_le_bytes());
    attr(&mut out, "screenWindowCenter", "v2f", &[0f32.to_le_bytes(), 0f32.to_le_bytes()].concat());
    attr(&mut out, "screenWindowWidth", "float", &1f32.to_le_bytes());
    out.push(0);

    let (w, h) = ((data[2] - data[0] + 1) as usize, (data[3] - data[1] + 1) as usize);
    let values = [1.0f32, 0.2, 0.4, 0.8];
    let line: Vec<u8> = values.iter().flat_map(|v| std::iter::repeat_n(v.to_le_bytes(), w).flatten()).collect();
    // One scan line per chunk: y, byte count, then each channel's row.
    let table_end = out.len() + h * 8;
    let chunk = 8 + line.len();
    for row in 0..h {
        out.extend_from_slice(&((table_end + row * chunk) as u64).to_le_bytes());
    }
    for row in 0..h {
        out.extend_from_slice(&(data[1] + row as i32).to_le_bytes());
        out.extend_from_slice(&(line.len() as i32).to_le_bytes());
        out.extend_from_slice(&line);
    }
    out
}

#[test]
fn probe_matches_the_size_decode_opens() {
    let cases = [
        ("data inside display", [0, 0, 31, 15], [8, 4, 23, 11]),
        ("equal windows", [0, 0, 31, 15], [0, 0, 31, 15]),
        ("data larger than display", [0, 0, 31, 15], [-4, -4, 35, 19]),
        ("data outside display", [0, 0, 31, 15], [40, 20, 55, 27]),
        ("data overlapping display edge", [0, 0, 31, 15], [24, 8, 39, 15]),
        ("display not at origin", [10, 5, 41, 20], [18, 9, 33, 16]),
    ];
    for (name, display, data) in cases {
        let bytes = exr(display, data);
        let opened = decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(probe_size(&bytes).ok(), Some(opened.dimensions()), "{name}");
    }
}

#[test]
fn hostile_windows_do_not_panic() {
    let windows = [
        ([0, 0, 0, 0], [i32::MAX - 1, i32::MAX - 1, i32::MAX - 1, i32::MAX - 1]),
        ([i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2], [0, 0, 3, 3]),
        ([0, 0, 31, 15], [5, 5, 4, 4]),
    ];
    for (display, data) in windows {
        let bytes = exr(display, data);
        let _ = probe_size(&bytes);
        let mut truncated = bytes.clone();
        truncated.truncate(bytes.len() / 2);
        let _ = probe_size(&truncated);
    }
}
