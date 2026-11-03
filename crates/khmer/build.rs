//! Offline, pinned Aksara source conversion for explicit Khmer language builds.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{env, error::Error, fs, io, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rustc-check-cfg=cfg(craft_khmer_data)");
    println!("cargo:rerun-if-env-changed=CRAFT_KHMER_DIR");
    println!("cargo:rerun-if-env-changed=CRAFT_KHMER_REQUIRED");
    println!("cargo:rerun-if-changed=source-pins.json");
    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR is missing"))?);
    let dir = env::var_os("CRAFT_KHMER_DIR").map(PathBuf::from);
    if dir.is_none() && env::var("CRAFT_KHMER_REQUIRED").as_deref() == Ok("1") {
        return Err("CRAFT_KHMER_REQUIRED=1 requires CRAFT_KHMER_DIR containing the pinned raw Aksara pack".into());
    }
    let pins: Value = serde_json::from_str(include_str!("source-pins.json"))?;
    if let Some(dir) = &dir {
        let path = dir.join("LICENCES/LICENSE");
        println!("cargo:rerun-if-changed={}", path.display());
        let licence = fs::read(&path)?;
        if Some(format!("{:x}", Sha256::digest(&licence)).as_str()) != pins["licence_sha256"].as_str() {
            return Err("Aksara MIT licence sha256 mismatch".into());
        }
    }
    let files = pins["files"].as_array().ok_or("invalid source pins")?;
    let symbols = ["FREQUENCIES", "SPELLINGS", "VARIANTS", "TITLES", "AFFIXES", "WEIGHTS"];
    let mut lengths = String::new();
    for (pin, symbol) in files.iter().zip(symbols).rev() {
        let file = pin["file"].as_str().ok_or("missing filename")?;
        let original = if let Some(dir) = &dir {
            let source = pin["source"].as_str().ok_or("missing source")?;
            let name = source.rsplit('/').next().ok_or("missing source name")?;
            let path = dir.join(name);
            println!("cargo:rerun-if-changed={}", path.display());
            let raw = fs::read(path)?;
            let digest = format!("{:x}", Sha256::digest(&raw));
            if Some(digest.as_str()) != pin["source_sha256"].as_str() {
                return Err(format!("{name}: raw sha256 mismatch").into());
            }
            let converted = convert(file, raw)?;
            let digest = format!("{:x}", Sha256::digest(&converted));
            if Some(digest.as_str()) != pin["bundled_sha256"].as_str() {
                return Err(format!("{file}: canonical sha256 mismatch").into());
            }
            converted
        } else {
            Vec::new()
        };
        fs::write(output.join(file), &original)?;
        let parameters = brotli::enc::BrotliEncoderParams { quality: 11, lgwin: 22, ..Default::default() };
        let mut compressed = Vec::new();
        brotli::BrotliCompress(&mut original.as_slice(), &mut compressed, &parameters)?;
        fs::write(output.join(format!("{file}.br")), &compressed)?;
        lengths.push_str(&format!(
            "const {symbol}: Descriptor = Descriptor {{ len: {}, packed_crc: {}, raw_crc: {} }};\n",
            original.len(),
            crc32fast::hash(&compressed),
            crc32fast::hash(&original)
        ));
    }
    fs::write(output.join("asset_descriptors.rs"), lengths)?;
    if dir.is_some() {
        println!("cargo:rustc-cfg=craft_khmer_data");
    }
    Ok(())
}
fn convert(file: &str, raw: Vec<u8>) -> Result<Vec<u8>, Box<dyn Error>> {
    let text = std::str::from_utf8(&raw)?;
    let result = match file {
        "frequency.tsv" | "kcc-weights.tsv" => {
            let json: Value = serde_json::from_str(text)?;
            let map = if file == "frequency.tsv" { &json } else { &json["weights"] };
            let mut out = String::new();
            for (key, value) in map.as_object().ok_or("expected object")? {
                let value = if file == "frequency.tsv" || value.is_i64() || value.is_u64() {
                    value.to_string()
                } else {
                    let n = value.as_f64().ok_or("expected weight")?;
                    if n.fract() == 0.0 { format!("{n:.1}") } else { n.to_string() }
                };
                out.push_str(&format!("{key}\t{value}\n"));
            }
            out
        }
        "titles.txt" => {
            let json: Vec<String> = serde_json::from_str(text)?;
            format!("{}\n", json.join("\n"))
        }
        "affixes.tsv" => {
            let pattern = regex::Regex::new(r"\{\s*text:\s*'([^']+)',\s*type:\s*'(prefix|suffix)',\s*isBreakPoint:\s*(true|false)\s*\}")?;
            pattern.captures_iter(text).map(|c| format!("{}\t{}\t{}\n", &c[1], &c[2], &c[3])).collect()
        }
        _ => return Ok(raw),
    };
    Ok(result.into_bytes())
}
