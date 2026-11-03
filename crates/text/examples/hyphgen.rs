//! Regenerates the bundled English hyphenation data from the Moby Hyphenator II word list.
//!
//! ```text
//! curl -LO https://www.gutenberg.org/files/3204/files/mhyph.txt   # Project Gutenberg #3204, public domain
//! cargo run --release -p craft-text --features generator --example hyphgen -- mhyph.txt crates/text/assets/hyphen [--validate]
//! ```
//!
//! Writes `en-us.dic` (the word list, front-coded and deflated; see `craft_text::hyphen::dict`)
//! and `en-us.pat` (Liang patterns trained from the list by `craft_text::hyphen::patgen`).
//! `--validate` first trains on 90% of the words and reports accuracy on the held-out 10%.

use std::collections::HashMap;
use std::path::PathBuf;

use craft_text::hyphen::dict::{Dictionary, hyphenated};
use craft_text::hyphen::patgen::{self, DEFAULT_LEVELS};

/// Mac OS Roman, 0x80..=0xFF (the list's encoding; 0xA5 `•` marks breaks).
const MAC_ROMAN_HIGH: &str =
    "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»…\u{A0}ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{F8FF}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";

fn mac_roman(bytes: &[u8]) -> String {
    let high: Vec<char> = MAC_ROMAN_HIGH.chars().collect();
    assert_eq!(high.len(), 128);
    bytes.iter().map(|&b| if b < 0x80 { b as char } else { high[b as usize - 0x80] }).collect()
}

const BREAK: char = '•';

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '\'' || c == BREAK
}

/// `(lowercase word, mask)` of one token like `Aa•ron's`, or None if it isn't a plain word.
fn token(t: &str) -> Option<(String, u64)> {
    let t = t.trim_matches('\'');
    if t.is_empty() || !t.chars().all(is_word_char) || t.starts_with(BREAK) || t.ends_with(BREAK) {
        return None;
    }
    let mut w = String::new();
    let mut m = 0u64;
    let mut n = 0usize;
    for c in t.chars() {
        if c == BREAK {
            if n < 64 {
                m |= 1 << n;
            }
        } else {
            w.extend(c.to_lowercase());
            n += 1;
        }
    }
    ((1..64).contains(&n) && w.chars().count() == n).then_some((w, m))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let validate = args.iter().any(|a| a == "--validate");
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let Some(src) = pos.first() else {
        eprintln!("usage: hyphgen <mhyph.txt> [out-dir] [--validate]");
        std::process::exit(2);
    };
    let out_dir = PathBuf::from(pos.get(1).map_or("crates/text/assets/hyphen", |s| s.as_str()));
    let text = mac_roman(&std::fs::read(src).expect("read word list"));

    // key → (lowercase-origin masks, other masks); compound parts only fill gaps.
    let mut primary: HashMap<String, (Vec<u64>, Vec<u64>)> = HashMap::new();
    let mut parts: HashMap<String, u64> = HashMap::new();
    let mut entries = 0usize;
    for line in text.lines() {
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        entries += 1;
        if let Some((w, m)) = token(line) {
            let lower = !line.chars().any(char::is_uppercase);
            let e = primary.entry(w).or_default();
            if lower { e.0.push(m) } else { e.1.push(m) }
        } else {
            for t in line.split([' ', '-', '/']) {
                if let Some((w, m)) = token(t) {
                    parts.entry(w).or_insert(m);
                }
            }
        }
    }
    let mut list: Vec<(String, u64)> = primary
        .into_iter()
        .map(|(w, (lower, other))| {
            // Prefer lowercase entries; where spellings disagree (re•cord / rec•ord) keep only the
            // breaks they share.
            let v = if lower.is_empty() { other } else { lower };
            (w, v.iter().fold(u64::MAX, |a, b| a & b))
        })
        .collect();
    let have: std::collections::HashSet<String> = list.iter().map(|x| x.0.clone()).collect();
    let extra: Vec<(String, u64)> = parts.into_iter().filter(|(w, _)| !have.contains(w)).collect();
    println!("{entries} entries → {} words (+{} from compounds)", list.len(), extra.len());
    list.extend(extra);
    let dict = Dictionary::from_entries(list);
    let bytes = dict.to_bytes();
    std::fs::create_dir_all(&out_dir).expect("mkdir");
    std::fs::write(out_dir.join("en-us.dic"), &bytes).expect("write en-us.dic");
    let back = Dictionary::from_bytes(&bytes).expect("roundtrip");
    assert_eq!(back.len(), dict.len());
    println!("en-us.dic: {} words, {} bytes", dict.len(), bytes.len());

    // Training data: plain lowercase ASCII words (+ apostrophes).
    let train: Vec<(Vec<char>, Vec<usize>)> = dict
        .iter()
        .filter(|(w, _)| w.chars().all(|c| c.is_ascii_lowercase() || c == '\''))
        .map(|(w, m)| (w.chars().collect(), (1..w.len().min(64)).filter(|&i| m & (1 << i) != 0).collect()))
        .collect();
    println!("training on {} words", train.len());
    let mut log = |s: String| println!("  {s}");
    // Optional override for tuning: HYPHGEN_LEVELS="1,4,1,2,20;2,5,2,1,8;…" (min,max,good,bad,threshold).
    let custom: Option<Vec<patgen::Level>> = std::env::var("HYPHGEN_LEVELS").ok().map(|s| {
        s.split(';')
            .map(|l| {
                let v: Vec<u32> = l.split(',').map(|x| x.trim().parse().expect("number")).collect();
                patgen::Level { min_len: v[0] as usize, max_len: v[1] as usize, good_weight: v[2], bad_weight: v[3], threshold: v[4] }
            })
            .collect()
    });
    let levels = custom.as_deref().unwrap_or(DEFAULT_LEVELS);
    if validate {
        let (fit, hold): (Vec<_>, Vec<_>) = train.iter().cloned().enumerate().partition(|(i, _)| i % 10 != 0);
        let fit: Vec<_> = fit.into_iter().map(|x| x.1).collect();
        let hold: Vec<_> = hold.into_iter().map(|x| x.1).collect();
        let p = patgen::train(&fit, levels, &mut log);
        let s = patgen::score(&p, &hold);
        println!(
            "held-out: {} words, {} breaks: found {} ({:.1}%), missed {}, wrong {} ({:.2}% of reported)",
            s.words,
            s.breaks,
            s.found,
            100.0 * s.found as f64 / s.breaks.max(1) as f64,
            s.missed(),
            s.wrong,
            100.0 * s.wrong as f64 / (s.found + s.wrong).max(1) as f64
        );
    }
    if std::env::var_os("HYPHGEN_VALIDATE_ONLY").is_some() {
        return;
    }
    let pats = patgen::train(&train, levels, &mut log);
    let s = patgen::score(&pats, &train);
    println!(
        "training set: found {}/{} ({:.1}%), wrong {} — {} patterns",
        s.found,
        s.breaks,
        100.0 * s.found as f64 / s.breaks.max(1) as f64,
        s.wrong,
        pats.len()
    );
    let mut out = String::new();
    out.push_str("% Craft US English hyphenation patterns (Liang/TeX notation).\n");
    out.push_str("% Generated by crates/text/examples/hyphgen.rs: trained with craft-text's patgen-style trainer\n");
    out.push_str("% from the Moby Hyphenator II word list by Grady Ward (public domain, Project Gutenberg #3204).\n");
    out.push_str("% Released into the public domain (CC0-1.0) by the DesignCraft and craft-libs contributors.\n");
    for line in pats.to_tex().chunks(12) {
        out.push_str(&line.join(" "));
        out.push('\n');
    }
    std::fs::write(out_dir.join("en-us.pat"), &out).expect("write en-us.pat");
    println!("en-us.pat: {} patterns, {} bytes", pats.len(), out.len());
    for w in ["hyphenation", "typography", "composition", "photographer", "unquestionably", "localization", "blockchain", "smartphone"] {
        let c: Vec<char> = w.chars().collect();
        let m = pats.points(&c).unwrap_or_default().iter().fold(0u64, |m, &i| m | 1 << i);
        println!("  patterns: {}", hyphenated(w, m));
    }
}
