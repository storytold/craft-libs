pub fn triangle_csh() -> Vec<u8> {
    polygon_csh(&[&[(0.25, 0.25), (0.75, 0.25), (0.5, 0.75)]])
}

pub fn polygon_csh(polygons: &[&[(f64, f64)]]) -> Vec<u8> {
    named_polygon_csh("Triangle", polygons)
}

pub fn named_polygon_csh(label: &str, polygons: &[&[(f64, f64)]]) -> Vec<u8> {
    let mut out = b"cush".to_vec();
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(&1u32.to_be_bytes());
    let name: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
    out.extend_from_slice(&(name.len() as u32).to_be_bytes());
    for unit in &name {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    if !name.len().is_multiple_of(2) {
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(&1u32.to_be_bytes());
    let mut body = vec![36];
    body.extend_from_slice(b"00000000-0000-4000-8000-000000000001");
    for bound in [0i32, 0, 100, 200] {
        body.extend_from_slice(&bound.to_be_bytes());
    }
    for selector in [6u16, 8] {
        body.extend_from_slice(&selector.to_be_bytes());
        body.extend_from_slice(&[0; 24]);
    }
    for polygon in polygons {
        body.extend_from_slice(&0u16.to_be_bytes());
        body.extend_from_slice(&(polygon.len() as u16).to_be_bytes());
        body.extend_from_slice(&[0; 22]);
        for &(x, y) in *polygon {
            body.extend_from_slice(&2u16.to_be_bytes());
            for _ in 0..3 {
                body.extend_from_slice(&((y * 16_777_216.0) as i32).to_be_bytes());
                body.extend_from_slice(&((x * 16_777_216.0) as i32).to_be_bytes());
            }
        }
    }
    while !body.len().is_multiple_of(4) {
        body.push(0);
    }
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}
