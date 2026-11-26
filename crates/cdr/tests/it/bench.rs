//! Read time of a large synthetic drawing. Run with
//! `cargo test --release -p craft-cdr --test it bench -- --ignored --nocapture`.

use craft_cdr::synth::{self, V, chunk, list};
use craft_cdr::{Limits, read};

/// A version 13 drawing of `n` curves of 22 points each, filled and outlined, compressed when
/// `compress`.
fn many_curves(n: usize, compress: bool) -> Vec<u8> {
    let v = V(1300);
    let red = synth::color(5, [0, 0, 255, 0]);
    let black = synth::color(2, [0, 0, 0, 100]);
    let styles = list(
        b"doc ",
        vec![
            synth::mcfg(v, 612.0, 792.0),
            list(b"filt", vec![chunk(b"fild", v.fill(1, &red))]),
            list(b"otlt", vec![chunk(b"outl", v.outline(2, synth::Pen { line_type: 0, width: 1.0, cap: 0, join: 0 }, &black, &[]))]),
        ],
    );
    let objects: Vec<_> = (0..n)
        .map(|i| {
            let o = (i % 500) as f64;
            // A move, then curves of two control points and an end point each.
            let kind = |k: i32| {
                if k == 0 {
                    0x08
                } else if k % 3 == 0 {
                    0x80
                } else {
                    0xC0
                }
            };
            let pts: Vec<(f64, f64, u8)> = (0..22).map(|k| (o + f64::from(k), o + f64::from(k % 5), kind(k))).collect();
            let args = vec![(20, 1u32.to_le_bytes().to_vec()), (10, 2u32.to_le_bytes().to_vec()), (30, v.points(&pts))];
            synth::object(v, 3, &args, Some([1.0, 0.0, 0.0, 1.0, o, -o]))
        })
        .collect();
    let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "Art", objects)]);
    let body = vec![styles, page];
    let mut kids = vec![chunk(b"vrsn", 1300u16.to_le_bytes().to_vec())];
    if compress {
        kids.push(synth::cmpr(&body));
    } else {
        kids.extend(body);
    }
    synth::riff(1300, &kids)
}

#[test]
#[ignore = "manual timing probe; wall-clock performance depends on machine load"]
fn read_a_large_drawing() {
    for compress in [false, true] {
        let file = many_curves(50_000, compress);
        let mut times = Vec::new();
        for _ in 0..5 {
            let t = std::time::Instant::now();
            let d = read(&file, Limits::default()).unwrap();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(d.pages.len(), 1);
        }
        times.sort_by(f64::total_cmp);
        println!("compress={compress} bytes={} median_ms={:.1} min_ms={:.1}", file.len(), times[2], times[0]);
    }
}
