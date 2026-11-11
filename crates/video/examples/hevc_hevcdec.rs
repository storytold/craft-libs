//! Decode an Annex-B H.265 file to raw planar YUV (yuv420p or yuv420p10le):
//! `cargo run --release -p craft-video --features hevc-decode,threads --example hevc_hevcdec -- in.hevc out.yuv`

use craft_video::hevc::decode::Decoder;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: hevcdec <in.hevc> [out.yuv]");
        std::process::exit(2);
    }
    let data = std::fs::read(&args[1]).expect("read input");
    let t0 = std::time::Instant::now();
    // HEVC_THREADS=n sets the thread count, HEVC_LOOPS=k decodes the file k times (benchmarking).
    let threads = std::env::var("HEVC_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(0usize);
    let loops = std::env::var("HEVC_LOOPS").ok().and_then(|s| s.parse().ok()).unwrap_or(1usize);
    let mut pics = Vec::new();
    for _ in 0..loops {
        let mut dec = if threads > 0 { Decoder::with_threads(threads) } else { Decoder::new() };
        // HEVC_DRAFT=1: draft mode (non-reference pictures skip deblocking and SAO).
        dec.set_draft(std::env::var_os("HEVC_DRAFT").is_some());
        pics = dec.decode(&data, 0).expect("decode");
        pics.extend(dec.flush());
    }
    let secs = t0.elapsed().as_secs_f64();
    eprintln!("{} pictures in {:.3}s ({:.1} fps)", pics.len(), secs, pics.len() as f64 / secs);
    if let Some(out) = args.get(2) {
        let mut buf = Vec::new();
        for p in &pics {
            buf.extend(p.y.to_le_bytes());
            buf.extend(p.u.to_le_bytes());
            buf.extend(p.v.to_le_bytes());
        }
        std::fs::write(out, buf).expect("write output");
    }
}
