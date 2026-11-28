//! The curated model registry — a hand-maintained table, compiled in.
//! Runtime-fetching this list would make our supply chain un-auditable.
//!
//! Curation rule: **permissive licenses only** (the suite is MIT OR
//! Apache-2.0 and commercial users run it). CC-BY-NC models like NLLB-200
//! are excluded by policy; their absence is deliberate.
//!
//! `sha256` may be `None`: Hugging Face serves the LFS digest in the
//! `X-Linked-ETag` header of the resolve redirect, which [`crate::fetch`]
//! uses as the expected checksum. We only hard-code a hash when we've
//! verified it ourselves — never a guessed one.
//!
//! The Qwen3 entries pin the GGUF **and** its tokenizer to immutable
//! Hugging Face commit revisions (the GGUF repository ships no tokenizer)
//! and carry the GGUF's SHA-256, so a download is reproducible.

use crate::probe::MachineCaps;

/// Model quality tier — higher is better output, bigger download.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quality {
    Tiny,
    Small,
    Medium,
}

/// Which candle architecture loads the GGUF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    Qwen2,
    /// Dense Qwen3 — loaded by candle's `quantized_qwen3`; the loader also
    /// turns thinking off (see `local::LocalGguf`). Qwen3.5 is a hybrid
    /// linear-attention architecture candle does not implement.
    Qwen3,
    Llama,
}

/// Download size in whole MiB, rounded **up**: the pre-flight disk check
/// must never understate a file that will not fit, and a download budget
/// derived from it must never be shorter than the transfer.
const fn mib(bytes: u64) -> u64 {
    bytes.div_ceil(1 << 20)
}

/// One downloadable model.
#[derive(Clone, Copy, Debug)]
pub struct ModelEntry {
    /// Stable identifier stored in settings (`"qwen2.5-1.5b-q4km"`).
    pub id: &'static str,
    /// Display name for the picker.
    pub name: &'static str,
    /// Canonical download URL (Hugging Face `resolve` endpoint).
    pub url: &'static str,
    /// Filename inside the model cache.
    pub file: &'static str,
    /// Companion tokenizer (needed by local inference; the GGUF ships
    /// vocab only, not the chat-time tokenizer.json).
    pub tokenizer_url: &'static str,
    /// Verified SHA-256 of the GGUF, when we have one.
    pub sha256: Option<&'static str>,
    /// Download size, MiB — shown to the user before they commit.
    pub size_mb: u64,
    /// Below this RAM the model is refused outright.
    pub min_ram_gb: f32,
    /// Recommended RAM for comfortable use.
    pub rec_ram_gb: f32,
    /// SPDX license id, displayed in the UI.
    pub license: &'static str,
    pub quality: Quality,
    pub arch: Arch,
    /// Human-readable language coverage for the picker.
    pub langs: &'static str,
}

/// The registry, smallest first.
pub static MODELS: &[ModelEntry] = &[
    ModelEntry {
        id: "qwen2.5-0.5b-q4km",
        name: "Qwen2.5 0.5B Instruct (Q4_K_M)",
        url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
        file: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
        tokenizer_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/resolve/main/tokenizer.json",
        sha256: None,
        size_mb: 389,
        min_ram_gb: 2.0,
        rec_ram_gb: 4.0,
        license: "Apache-2.0",
        quality: Quality::Tiny,
        arch: Arch::Qwen2,
        langs: "29+ languages incl. EN/ZH/FR/DE/ES/JA/KO/RU",
    },
    ModelEntry {
        id: "qwen3-0.6b-q8_0",
        name: "Qwen3 0.6B (Q8_0)",
        url: "https://huggingface.co/Qwen/Qwen3-0.6B-GGUF/resolve/23749fefcc72300e3a2ad315e1317431b06b590a/Qwen3-0.6B-Q8_0.gguf",
        file: "Qwen3-0.6B-Q8_0.gguf",
        tokenizer_url: "https://huggingface.co/Qwen/Qwen3-0.6B/resolve/c1899de289a04d12100db370d81485cdf75e47ca/tokenizer.json",
        sha256: Some("9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031"),
        size_mb: mib(639_446_688),
        min_ram_gb: 1.5,
        rec_ram_gb: 3.0,
        license: "Apache-2.0",
        quality: Quality::Tiny,
        arch: Arch::Qwen3,
        langs: "119 languages incl. EN/ZH/FR/DE/ES/JA/KO/RU",
    },
    ModelEntry {
        id: "smollm2-1.7b-q4km",
        name: "SmolLM2 1.7B Instruct (Q4_K_M)",
        url: "https://huggingface.co/bartowski/SmolLM2-1.7B-Instruct-GGUF/resolve/main/SmolLM2-1.7B-Instruct-Q4_K_M.gguf",
        file: "SmolLM2-1.7B-Instruct-Q4_K_M.gguf",
        tokenizer_url: "https://huggingface.co/HuggingFaceTB/SmolLM2-1.7B-Instruct/resolve/main/tokenizer.json",
        sha256: None,
        size_mb: 1033,
        min_ram_gb: 4.0,
        rec_ram_gb: 8.0,
        license: "Apache-2.0",
        quality: Quality::Small,
        arch: Arch::Llama,
        langs: "primarily EN/FR/DE/ES/IT",
    },
    ModelEntry {
        id: "qwen2.5-1.5b-q4km",
        name: "Qwen2.5 1.5B Instruct (Q4_K_M)",
        url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        file: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        tokenizer_url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct/resolve/main/tokenizer.json",
        sha256: None,
        size_mb: 986,
        min_ram_gb: 4.0,
        rec_ram_gb: 8.0,
        license: "Apache-2.0",
        quality: Quality::Small,
        arch: Arch::Qwen2,
        langs: "29+ languages incl. EN/ZH/FR/DE/ES/JA/KO/RU",
    },
    ModelEntry {
        id: "qwen3-1.7b-q8_0",
        name: "Qwen3 1.7B (Q8_0)",
        url: "https://huggingface.co/Qwen/Qwen3-1.7B-GGUF/resolve/90862c4b9d2787eaed51d12237eafdfe7c5f6077/Qwen3-1.7B-Q8_0.gguf",
        file: "Qwen3-1.7B-Q8_0.gguf",
        tokenizer_url: "https://huggingface.co/Qwen/Qwen3-1.7B/resolve/70d244cc86ccca08cf5af4e1e306ecf908b1ad5e/tokenizer.json",
        sha256: Some("061b54daade076b5d3362dac252678d17da8c68f07560be70818cace6590cb1a"),
        size_mb: mib(1_834_426_016),
        min_ram_gb: 3.0,
        rec_ram_gb: 6.0,
        license: "Apache-2.0",
        quality: Quality::Small,
        arch: Arch::Qwen3,
        langs: "119 languages incl. EN/ZH/FR/DE/ES/JA/KO/RU",
    },
    // NOTE: Qwen2.5-3B was removed — its `qwen-research` license is
    // non-commercial and breaks the permissive-only rule above. No
    // permissively-licensed Medium-tier candidate is currently curated.
];

/// Look up a model by stable id.
pub fn by_id(id: &str) -> Option<&'static ModelEntry> {
    MODELS.iter().find(|m| m.id == id)
}

/// Whether `entry` can run at all on this machine.
pub fn fits(caps: &MachineCaps, entry: &ModelEntry) -> bool {
    let ram_ok = caps.ram_gb >= entry.min_ram_gb;
    let disk_ok = caps.disk_free_gb >= entry.size_mb as f32 / 1024.0 * 1.3; // file + headroom
    ram_ok && disk_ok
}

/// The default suggestion for `caps`: the best quality tier the machine can
/// run, and inside a tier the newest entry in this table (Qwen3 over Qwen2.5).
/// Returns `None` when nothing fits — the UI then says so honestly instead of
/// offering a download that can't run.
pub fn suggest(caps: &MachineCaps) -> Option<&'static ModelEntry> {
    let capable: Vec<&ModelEntry> = MODELS.iter().filter(|m| fits(caps, m)).collect();
    let best_allowed = if caps.vram_gb.is_some_and(|v| v >= 6.0) || caps.ram_gb >= 16.0 {
        Quality::Medium
    } else if caps.ram_gb >= 8.0 {
        Quality::Small
    } else {
        Quality::Tiny
    };
    capable.into_iter().filter(|m| m.quality <= best_allowed).max_by_key(|m| m.quality)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(ram_gb: f32, disk_free_gb: f32) -> MachineCaps {
        MachineCaps { ram_gb, disk_free_gb, threads: 8, avx2: true, vram_gb: None }
    }

    #[test]
    fn qwen3_entries_are_looked_up_and_pinned() {
        for (id, file, gguf_rev, tok_rev, sha) in [
            (
                "qwen3-1.7b-q8_0",
                "Qwen3-1.7B-Q8_0.gguf",
                "90862c4b9d2787eaed51d12237eafdfe7c5f6077",
                "70d244cc86ccca08cf5af4e1e306ecf908b1ad5e",
                "061b54daade076b5d3362dac252678d17da8c68f07560be70818cace6590cb1a",
            ),
            (
                "qwen3-0.6b-q8_0",
                "Qwen3-0.6B-Q8_0.gguf",
                "23749fefcc72300e3a2ad315e1317431b06b590a",
                "c1899de289a04d12100db370d81485cdf75e47ca",
                "9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031",
            ),
        ] {
            let entry = by_id(id).unwrap_or_else(|| panic!("{id} is registered"));
            assert_eq!(entry.file, file);
            assert_eq!(entry.arch, Arch::Qwen3);
            assert_eq!(entry.license, "Apache-2.0");
            assert_eq!(entry.sha256, Some(sha));
            // Immutable commit revisions, never `main` or `refs/pr/N`.
            assert!(entry.url.contains(gguf_rev), "{}", entry.url);
            assert!(entry.tokenizer_url.contains(tok_rev), "{}", entry.tokenizer_url);
            assert!(entry.url.ends_with(file));
            // `size_mb` is MiB rounded up from the pinned byte size.
            assert!(entry.size_mb > 0);
        }
    }

    #[test]
    fn every_pinned_digest_is_a_lowercase_sha256() {
        for entry in MODELS {
            if let Some(sha) = entry.sha256 {
                assert_eq!(sha.len(), 64, "{}", entry.id);
                assert!(sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{}", entry.id);
            }
        }
    }

    #[test]
    fn rounded_up_size_matches_the_pinned_bytes() {
        assert_eq!(mib(1_834_426_016), 1750);
        assert_eq!(mib(639_446_688), 610);
        // A whole-MiB file is not rounded past its size; a single extra byte is.
        assert_eq!(mib(1 << 20), 1);
        assert_eq!(mib((1 << 20) + 1), 2);
    }

    #[test]
    fn qwen3_fits_at_its_pinned_minimum_ram() {
        let big = by_id("qwen3-1.7b-q8_0").expect("entry");
        let small = by_id("qwen3-0.6b-q8_0").expect("entry");
        assert!(!fits(&caps(2.9, 50.0), big));
        assert!(fits(&caps(3.0, 50.0), big));
        assert!(!fits(&caps(50.0, 2.0), big), "file + headroom is ~2.2 GiB");
        assert!(fits(&caps(50.0, 2.3), big));
        assert!(!fits(&caps(1.4, 50.0), small));
        assert!(fits(&caps(1.5, 50.0), small));
    }

    #[test]
    fn suggestion_prefers_qwen3_inside_a_tier() {
        // Roomy machine: the Small tier, newest entry last in the table.
        assert_eq!(suggest(&caps(16.0, 50.0)).map(|m| m.id), Some("qwen3-1.7b-q8_0"));
        // Below 8 GiB the tier is Tiny even though the 1.7B would fit.
        assert_eq!(suggest(&caps(6.0, 50.0)).map(|m| m.id), Some("qwen3-0.6b-q8_0"));
        // No disk, no suggestion.
        assert_eq!(suggest(&caps(16.0, 0.0)).map(|m| m.id), None);
    }
}
