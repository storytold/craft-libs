//! Check `.atn` files for byte-exact round trips: parse, write, compare, and show the first
//! difference with its context: `cargo run -p craft-psd --example atn_roundtrip -- set.atn…`
//!
//! Exit status 1 when any file differs or fails to parse.

use craft_psd::atn;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

fn main() {
    let mut failed = false;
    for path in std::env::args().skip(1) {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{path}: {e}");
                failed = true;
                continue;
            }
        };
        let file = match atn::parse(&bytes) {
            Ok(f) => f,
            Err(e) => {
                println!("{path}: parse failed: {e}");
                failed = true;
                continue;
            }
        };
        let written = match atn::write(&file) {
            Ok(w) => w,
            Err(e) => {
                println!("{path}: write failed: {e}");
                failed = true;
                continue;
            }
        };
        if written == bytes {
            println!("{path}: identical ({} bytes)", bytes.len());
            continue;
        }
        failed = true;
        let at = bytes.iter().zip(&written).position(|(a, b)| a != b).unwrap_or(bytes.len().min(written.len()));
        let lo = at.saturating_sub(24);
        println!("{path}: differs at byte {at} (original {} bytes, rewritten {} bytes)", bytes.len(), written.len());
        println!("  original : {}", hex(bytes.get(lo..(at + 40).min(bytes.len())).unwrap_or(&[])));
        println!("  rewritten: {}", hex(written.get(lo..(at + 40).min(written.len())).unwrap_or(&[])));
        println!(
            "  original : {}",
            String::from_utf8_lossy(bytes.get(lo..(at + 40).min(bytes.len())).unwrap_or(&[])).replace(|c: char| c.is_control(), ".")
        );
    }
    if failed {
        std::process::exit(1);
    }
}
