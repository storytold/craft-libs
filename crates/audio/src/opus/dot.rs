//! Shared fixed-order FIR dot product.

/// Dot product with eight independent partial sums, which the compiler can vectorise (a single
/// running sum is bound by the latency of the float addition). The sums are combined pairwise.
#[inline]
pub(super) fn dot8(a: &[f32], b: &[f32]) -> f32 {
    let mut s = [0f32; 8];
    let (ca, ra) = a.as_chunks::<8>();
    let (cb, rb) = b.as_chunks::<8>();
    for (x, y) in ca.iter().zip(cb) {
        for l in 0..8 {
            s[l] += x[l] * y[l];
        }
    }
    let mut acc = ((s[0] + s[4]) + (s[1] + s[5])) + ((s[2] + s[6]) + (s[3] + s[7]));
    for (x, y) in ra.iter().zip(rb) {
        acc += x * y;
    }
    acc
}
