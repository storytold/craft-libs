//! Decode an IVF AV1 file: `cargo run --release -p craft-video --features av1-decode,threads --example av1_av1dec -- in.ivf [out.yuv]`.
//! Writes raw planar 16-bit little-endian output. `AV1_THREADS=n` sets the thread count,
//! `AV1_LOOPS=k` decodes the file k times (benchmarking), `AV1_STATS=1` prints stage statistics.

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: av1dec in.ivf [out.yuv]");
        std::process::exit(2);
    }
    let data = std::fs::read(&args[1]).expect("read input");
    let mut out = args.get(2).map(|p| std::io::BufWriter::new(std::fs::File::create(p).expect("create output")));
    let threads = std::env::var("AV1_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(0usize);
    let loops = std::env::var("AV1_LOOPS").ok().and_then(|s| s.parse().ok()).unwrap_or(1usize);
    let hl = u16::from_le_bytes([data[6], data[7]]) as usize;
    let t0 = std::time::Instant::now();
    let mut n = 0;
    let mut dec = if threads > 0 { craft_video::av1::decode::Decoder::with_threads(threads) } else { craft_video::av1::decode::Decoder::new() };
    // AV1_DRAFT=1: draft mode (non-reference frames skip the in-loop filters).
    dec.set_draft(std::env::var_os("AV1_DRAFT").is_some());
    let mut write = |pics: Vec<craft_video::av1::decode::Picture>| {
        for pic in pics {
            n += 1;
            if let Some(o) = out.as_mut() {
                for pl in &pic.planes {
                    let bytes: Vec<u8> = pl.iter().flat_map(|s| s.to_le_bytes()).collect();
                    o.write_all(&bytes).unwrap();
                }
            }
        }
    };
    for _ in 0..loops {
        let mut p = hl;
        let mut i = 0i64;
        while p + 12 <= data.len() {
            let sz = u32::from_le_bytes(data[p..p + 4].try_into().unwrap()) as usize;
            p += 12;
            let Some(tu) = data.get(p..p + sz) else { break };
            p += sz;
            match dec.decode_pts(tu, i) {
                Ok(pics) => write(pics),
                Err(e) => eprintln!("temporal unit {i}: {e}"),
            }
            i += 1;
        }
        write(dec.flush());
    }
    eprintln!("{n} pictures in {:.3}s", t0.elapsed().as_secs_f64());
    if std::env::var_os("AV1_STATS").is_some() {
        eprintln!("{:#?}", dec.stats());
    }
}
