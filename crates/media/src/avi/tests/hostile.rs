//! Hostile-input hammer: truncation, bit flips and extreme field values must never panic or hang.

use super::*;

fn exercise(d: &[u8]) {
    for opts in [OpenOptions::default(), OpenOptions { ignore_odml: true, ..Default::default() }] {
        if let Ok(f) = open_with(d, opts) {
            for (s, st) in f.streams.iter().enumerate() {
                let n = st.chunks.len();
                for k in [0, n / 2, n.saturating_sub(1), n, usize::MAX] {
                    let _ = f.read_chunk(d, s, k);
                }
            }
        }
    }
}

#[test]
fn hostile_input_never_panics() {
    let (_, at) = avi_odml([0, 0]);
    let seeds = [avi(true, false, false), avi(true, true, true), avi(false, false, false), avi_odml(at).0];
    for (i, s) in seeds.iter().enumerate() {
        craft_media_testkit::hostile::hammer(&format!("avi seed {i}"), s, 30_000, exercise);
    }
}
