//! Run fresh processes to measure cold startup; repeat workloads for warm timings.
use std::{hint::black_box, time::Instant};

fn main() {
    if !cfg!(craft_khmer_data) {
        eprintln!("This benchmark requires CRAFT_KHMER_DIR and CRAFT_KHMER_REQUIRED=1.");
        std::process::exit(2);
    }
    let sentence = "ខ្ញុំស្រឡាញ់ភាសាខ្មែរ ព្រះជាម្ចាស់ទ្រង់មានព្រះបន្ទូល ការសិក្សាចំណេះដឹងរូបភាព";
    let started = Instant::now();
    let mut checksum = craft_khmer::word_ranges(sentence).len();
    checksum += usize::from(craft_khmer::is_correct("ខ្មែរ"));
    checksum += usize::from(craft_khmer::is_title("លោក"));
    checksum += craft_khmer::grammar_matches("កំណត់សំគាល់ ខាងវិញ្ញាណ").len();
    checksum += craft_khmer::suggest("ខ្មែ", 8).len();
    let cold = started.elapsed();
    let started = Instant::now();
    for _ in 0..500 {
        checksum += black_box(craft_khmer::word_ranges(black_box(sentence))).len();
    }
    let segmentation = started.elapsed();
    let started = Instant::now();
    for _ in 0..100 {
        checksum += black_box(craft_khmer::suggest(black_box("ខ្មែ"), 8)).len();
    }
    let suggestions = started.elapsed();
    println!(
        "{{\"cold_us\":{},\"segmentation_500_us\":{},\"suggestions_100_us\":{},\"checksum\":{}}}",
        cold.as_micros(),
        segmentation.as_micros(),
        suggestions.as_micros(),
        black_box(checksum)
    );
}
