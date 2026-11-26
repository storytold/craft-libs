//! In-loop deblocking filter for Theora (Section 7.10).

/// Non-linear loop filter limiting response function `lflim(R, L)` (Figure 7.3).
#[inline]
pub fn lflim(r: i32, l: i32) -> i32 {
    let two_l = 2 * l;
    if r <= -two_l || r >= two_l {
        0
    } else if r <= -l {
        -r - two_l
    } else if r < l {
        r
    } else {
        -r + two_l
    }
}

/// 4-tap horizontal block filter across a vertical edge (Section 7.10.1).
pub fn filter_horizontal_edge(recp: &mut [u8], stride: usize, fx: usize, fy: usize, l: i32) {
    for by in 0..8 {
        let row_start = (fy + by) * stride;
        let p0 = recp[row_start + fx] as i32;
        let p1 = recp[row_start + fx + 1] as i32;
        let p2 = recp[row_start + fx + 2] as i32;
        let p3 = recp[row_start + fx + 3] as i32;

        let r = (p0 - 3 * p1 + 3 * p2 - p3 + 4) >> 3;
        let delta = lflim(r, l);

        recp[row_start + fx + 1] = (p1 + delta).clamp(0, 255) as u8;
        recp[row_start + fx + 2] = (p2 - delta).clamp(0, 255) as u8;
    }
}

/// 4-tap vertical block filter across a horizontal edge (Section 7.10.2).
pub fn filter_vertical_edge(recp: &mut [u8], stride: usize, fx: usize, fy: usize, l: i32) {
    for bx in 0..8 {
        let p0 = recp[fy * stride + fx + bx] as i32;
        let p1 = recp[(fy + 1) * stride + fx + bx] as i32;
        let p2 = recp[(fy + 2) * stride + fx + bx] as i32;
        let p3 = recp[(fy + 3) * stride + fx + bx] as i32;

        let r = (p0 - 3 * p1 + 3 * p2 - p3 + 4) >> 3;
        let delta = lflim(r, l);

        recp[(fy + 1) * stride + fx + bx] = (p1 + delta).clamp(0, 255) as u8;
        recp[(fy + 2) * stride + fx + bx] = (p2 - delta).clamp(0, 255) as u8;
    }
}
