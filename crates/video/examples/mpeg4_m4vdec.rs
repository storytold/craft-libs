//! Decode a raw MPEG-4 Part 2 elementary stream (`.m4v`) to raw yuv420p:
//! `mpeg4_m4vdec in.m4v [out.yuv]`. Prints timing information to stderr. `MPEG4_BENCH_ITERS=n`
//! turns it into a benchmark that reports the best of n runs.

use craft_video::mpeg4::{Decoder, Picture};
use std::io::Write;

/// Split the stream before each VOP start code, so every chunk holds one VOP and the headers
/// that precede it.
fn chunks(data: &[u8]) -> Vec<&[u8]> {
    let mut cuts = vec![0];
    let mut seen_vop = false;
    for i in 0..data.len().saturating_sub(3) {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let code = data[i + 3];
            if seen_vop && (code == 0xB6 || code == 0xB3 || code < 0x30 || code == 0xB0) {
                cuts.push(i);
                seen_vop = false;
            }
            seen_vop |= code == 0xB6;
        }
    }
    cuts.push(data.len());
    cuts.windows(2).map(|w| &data[w[0]..w[1]]).filter(|c| !c.is_empty()).collect()
}

/// Decode every chunk, pass each picture to `sink`, and report decoding errors on stderr: a
/// failed call, and damage concealed in a call that still returned pictures.
fn run(chunks: &[&[u8]], mut sink: impl FnMut(Picture) -> std::io::Result<()>) -> std::io::Result<usize> {
    let mut dec = Decoder::new();
    let mut n = 0;
    for (i, c) in chunks.iter().enumerate() {
        let errors = dec.errors();
        match dec.decode(c, i as i64) {
            Ok(pics) => {
                if dec.errors() > errors
                    && let Some(e) = dec.last_error()
                {
                    eprintln!("decode error (concealed): {e}");
                }
                for p in pics {
                    n += 1;
                    sink(p)?;
                }
            }
            Err(e) => eprintln!("decode error: {e}"),
        }
    }
    for p in dec.flush() {
        n += 1;
        sink(p)?;
    }
    Ok(n)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        return Err("usage: mpeg4_m4vdec in.m4v [out.yuv]".into());
    }
    let data = std::fs::read(&args[1])?;
    let chunks = chunks(&data);
    if let Some(n) = std::env::var("MPEG4_BENCH_ITERS").ok().and_then(|v| v.parse::<usize>().ok()) {
        let mut best = f64::MAX;
        let mut frames = 0;
        for _ in 0..n {
            let t0 = std::time::Instant::now();
            frames = run(&chunks, |_| Ok(()))?;
            best = best.min(t0.elapsed().as_secs_f64());
        }
        eprintln!("best of {n}: {frames} frames in {best:.3}s ({:.1} fps)", frames as f64 / best);
        return Ok(());
    }
    let mut out = match args.get(2) {
        Some(p) => Some(std::io::BufWriter::new(std::fs::File::create(p)?)),
        None => None,
    };
    let t0 = std::time::Instant::now();
    let n = run(&chunks, |p| {
        if let Some(o) = out.as_mut() {
            o.write_all(&p.y)?;
            o.write_all(&p.u)?;
            o.write_all(&p.v)?;
        }
        Ok(())
    })?;
    if let Some(mut o) = out {
        // Dropping a BufWriter discards a failed final write; flush so a short file is reported.
        o.flush()?;
    }
    let dt = t0.elapsed().as_secs_f64();
    eprintln!("{n} frames in {dt:.3}s ({:.1} fps)", n as f64 / dt);
    Ok(())
}
