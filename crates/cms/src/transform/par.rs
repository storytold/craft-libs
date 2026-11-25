//! Row-parallel helpers for pixel buffers.

// ------------------------------------------------------------ parallel helpers

#[cfg(not(target_arch = "wasm32"))]
const PAR_MIN_PIXELS: usize = 16 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const PAR_CHUNK_PIXELS: usize = 8 * 1024;

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn par_rows<S: Sync, D: Send>(src: &[S], ss: usize, dst: &mut [D], ds: usize, f: impl Fn(&[S], &mut [D]) + Sync) {
    use rayon::prelude::*;
    let n = (src.len() / ss).min(dst.len() / ds);
    let (src, dst) = (&src[..n * ss], &mut dst[..n * ds]);
    if n < PAR_MIN_PIXELS {
        return f(src, dst);
    }
    src.par_chunks(PAR_CHUNK_PIXELS * ss).zip(dst.par_chunks_mut(PAR_CHUNK_PIXELS * ds)).for_each(|(s, d)| f(s, d));
}

#[cfg(target_arch = "wasm32")]
pub(super) fn par_rows<S: Sync, D: Send>(src: &[S], ss: usize, dst: &mut [D], ds: usize, f: impl Fn(&[S], &mut [D]) + Sync) {
    let n = (src.len() / ss).min(dst.len() / ds);
    f(&src[..n * ss], &mut dst[..n * ds]);
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn par_inplace(buf: &mut [f32], stride: usize, f: impl Fn(&mut [f32]) + Sync) {
    use rayon::prelude::*;
    if buf.len() / stride < PAR_MIN_PIXELS {
        return f(buf);
    }
    buf.par_chunks_mut(PAR_CHUNK_PIXELS * stride).for_each(&f);
}

#[cfg(target_arch = "wasm32")]
pub(super) fn par_inplace(buf: &mut [f32], _stride: usize, f: impl Fn(&mut [f32]) + Sync) {
    f(buf)
}
