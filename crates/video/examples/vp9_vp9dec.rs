//! Decode an IVF file: `cargo run --release -p craft-video --features vp9-decode,threads --example vp9_vp9dec -- in.ivf [out.yuv]`.
//! Writes raw planar output (8-bit or 16-bit little endian) and prints decoder statistics.

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&args[1]).expect("read input");
    let mut out = args.get(2).map(|p| std::io::BufWriter::new(std::fs::File::create(p).expect("create output")));
    let threads = std::env::var("VP9_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut dec = if threads > 0 { craft_video::vp9::decode::Decoder::with_threads(threads) } else { craft_video::vp9::decode::Decoder::new() };
    // VP9_DRAFT=1: draft mode (non-reference frames skip the loop filter).
    dec.set_draft(std::env::var_os("VP9_DRAFT").is_some());
    // VP9_LOOPS=n decodes the file n times (for profiling; the stream starts with a key frame).
    let loops = std::env::var("VP9_LOOPS").ok().and_then(|s| s.parse().ok()).unwrap_or(1usize);
    let hl = u16::from_le_bytes([data[6], data[7]]) as usize;
    let mut i = 0i64;
    let t0 = std::time::Instant::now();
    let mut n = 0;
    for _ in 0..loops {
        let mut p = hl;
        while p + 12 <= data.len() {
            let sz = u32::from_le_bytes(data[p..p + 4].try_into().unwrap()) as usize;
            p += 12;
            let Some(frame) = data.get(p..p + sz) else { break };
            p += sz;
            match dec.decode(frame, i) {
                Ok(pics) => {
                    for pic in pics.into_iter().chain(if p >= data.len() { dec.flush() } else { Vec::new() }) {
                        n += 1;
                        if let Some(o) = out.as_mut() {
                            for pl in [&pic.y, &pic.u, &pic.v] {
                                o.write_all(&pl.to_le_bytes()).unwrap();
                            }
                        }
                    }
                }
                Err(e) => eprintln!("chunk {i}: {e}"),
            }
            i += 1;
        }
    }
    eprintln!("{n} pictures in {:.3}s", t0.elapsed().as_secs_f64());
    eprintln!("{:#?}", dec.stats());
}
