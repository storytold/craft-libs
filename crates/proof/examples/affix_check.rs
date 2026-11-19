//! Compare the affix engine with another spell checker, black-box: load a Hunspell `.aff`/`.dic`
//! pair and answer for words, or make candidate compounds to ask both about.
//!
//! ```sh
//! # One word per line in, `word<TAB>1` (right) or `word<TAB>0` out, and the time taken on stderr.
//! cargo run --release -p craft-proof --example affix_check -- check de_DE.aff de_DE.dic < words.txt
//! # 2000 candidates (forms of random stems glued together, some with a letter changed), seed 7.
//! cargo run --release -p craft-proof --example affix_check -- gen de_DE.aff de_DE.dic 2000 7 > words.txt
//! # Single words instead: forms of random stems, and stems with another stem's ending.
//! cargo run --release -p craft-proof --example affix_check -- single de_DE.aff de_DE.dic 2000 7 > words.txt
//! # Suggestions: `word<TAB>microseconds<TAB>suggestion,suggestion…` per line.
//! cargo run --release -p craft-proof --example affix_check -- suggest de_DE.aff de_DE.dic < typos.txt
//! ```
//!
//! The answers are compared with `hunspell -d <path without extension> -l` on the same words
//! (the words it lists are the wrong ones). Nothing of the other checker is read but its answers.

use std::io::{BufRead, Write};

use craft_proof::affix::AffixDict;

fn load(aff: &str, dic: &str) -> Result<AffixDict, String> {
    let aff = std::fs::read(aff).map_err(|e| format!("{aff}: {e}"))?;
    let dic = std::fs::read(dic).map_err(|e| format!("{dic}: {e}"))?;
    let (aff, dic) = craft_proof::hunspell::decode(&aff, &dic)?;
    AffixDict::from_hunspell(&aff, &dic, &[])
}

/// xorshift64*: a reproducible stream of numbers, enough to pick stems.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

/// `w` with its first letter in lower case (a compound's later parts).
fn decapitalize(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_lowercase().chain(c).collect()).unwrap_or_default()
}

/// Attempts per requested word before a generator gives up (a dictionary that yields none).
const MAX_TRIES_PER_WORD: usize = 1000;

fn generate(d: &AffixDict, n: usize, seed: u64) -> Vec<String> {
    let stems: Vec<&str> = d.stems().filter(|s| s.chars().all(char::is_alphabetic)).collect();
    let mut rng = Rng(seed.max(1));
    let form = |rng: &mut Rng| -> String {
        let s = stems.get(rng.below(stems.len())).copied().unwrap_or_default();
        let forms = d.forms(s);
        forms.get(rng.below(forms.len())).cloned().unwrap_or_else(|| s.to_string())
    };
    let mut out = Vec::with_capacity(n);
    // A dictionary without alphabetic stems yields no word: give up instead of looping forever.
    let mut tries = n.saturating_mul(MAX_TRIES_PER_WORD);
    while out.len() < n && !stems.is_empty() && tries > 0 {
        tries -= 1;
        let parts = 2 + rng.below(2);
        let mut w = form(&mut rng);
        for _ in 1..parts {
            if rng.below(4) == 0 {
                w.push('s');
            }
            w.push_str(&decapitalize(&form(&mut rng)));
        }
        if rng.below(3) == 0 {
            // A letter dropped, doubled or changed: mostly wrong words.
            let chars: Vec<char> = w.chars().collect();
            let i = rng.below(chars.len());
            let mut m: String = chars.iter().take(i).collect();
            match rng.below(3) {
                0 => {}
                1 => m.extend(chars.get(i).into_iter().chain(chars.get(i))),
                _ => m.push('e'),
            }
            m.extend(chars.iter().skip(i + 1));
            w = m;
        }
        // Letters only: the other checker's command line splits words at anything else.
        if w.chars().count() >= 4 && w.len() <= 100 && w.chars().all(char::is_alphabetic) {
            out.push(w);
        }
    }
    out
}

/// Single words (no compounds): forms of random stems, and stems with another stem's ending
/// (mostly wrong, some right: an ending the stem's own homonym allows or not).
fn single(d: &AffixDict, n: usize, seed: u64) -> Vec<String> {
    let stems: Vec<&str> = d.stems().filter(|s| s.chars().all(char::is_alphabetic)).collect();
    let mut rng = Rng(seed.max(1));
    let mut endings: Vec<String> = Vec::new();
    for _ in 0..200 {
        let s = stems.get(rng.below(stems.len())).copied().unwrap_or_default();
        endings.extend(d.forms(s).iter().filter_map(|f| f.strip_prefix(s)).filter(|e| !e.is_empty()).map(str::to_string));
    }
    endings.sort();
    endings.dedup();
    let mut out = Vec::with_capacity(n);
    let mut tries = n.saturating_mul(MAX_TRIES_PER_WORD);
    while out.len() < n && !stems.is_empty() && tries > 0 {
        tries -= 1;
        let s = stems.get(rng.below(stems.len())).copied().unwrap_or_default();
        let forms = d.forms(s);
        if let Some(f) = forms.get(rng.below(forms.len())) {
            out.push(f.clone());
        }
        if let Some(e) = endings.get(rng.below(endings.len())) {
            out.push(format!("{s}{e}"));
        }
        out.retain(|w| w.chars().all(char::is_alphabetic));
    }
    // Letters only: the other checker's command line splits words at anything else.
    out.retain(|w| w.chars().all(char::is_alphabetic));
    out.truncate(n);
    out
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, aff, dic] if cmd == "check" => {
            let d = load(aff, dic)?;
            let words: Vec<String> = std::io::stdin().lock().lines().map_while(Result::ok).collect();
            let t = std::time::Instant::now();
            let mut slowest: Vec<(std::time::Duration, &str)> = Vec::new();
            let answers: Vec<bool> = words
                .iter()
                .map(|w| {
                    let t = std::time::Instant::now();
                    let ok = d.check(w);
                    slowest.push((t.elapsed(), w));
                    ok
                })
                .collect();
            let took = t.elapsed();
            slowest.sort_by_key(|s| std::cmp::Reverse(s.0));
            for (t, w) in slowest.iter().take(3) {
                eprintln!("slow: {w} {t:?}");
            }
            let mut out = std::io::stdout().lock();
            for (w, ok) in words.iter().zip(answers) {
                writeln!(out, "{w}\t{}", u8::from(ok)).map_err(|e| e.to_string())?;
            }
            eprintln!("{} words in {took:?}", words.len());
            Ok(())
        }
        [cmd, aff, dic] if cmd == "suggest" => {
            let d = load(aff, dic)?;
            let mut out = std::io::stdout().lock();
            for w in std::io::stdin().lock().lines().map_while(Result::ok) {
                let t = std::time::Instant::now();
                let s = d.suggest(&w, 8);
                writeln!(out, "{w}\t{}\t{}", t.elapsed().as_micros(), s.join(",")).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        [cmd, aff, dic, n, seed] if cmd == "gen" || cmd == "single" => {
            let d = load(aff, dic)?;
            let n: usize = n.parse().map_err(|_| "n is a number")?;
            let seed: u64 = seed.parse().map_err(|_| "seed is a number")?;
            let words = if cmd == "gen" { generate(&d, n, seed) } else { single(&d, n, seed) };
            if words.len() < n {
                return Err(format!("only {} of {n} words could be generated from this dictionary", words.len()));
            }
            for w in words {
                println!("{w}");
            }
            Ok(())
        }
        _ => Err("usage: affix_check check|suggest <aff> <dic> < words | affix_check gen|single <aff> <dic> <n> <seed>".into()),
    }
}
