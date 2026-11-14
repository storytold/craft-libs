//! Encrypt a PDF: `cargo run -p craft-pdf --features cos --example cos_protect in.pdf out.pdf rc4-40|rc4-128|aes-128|aes-256 USER OWNER`
/// 32 fresh bytes for the file key. `RandomState` is seeded by the operating system (as the
/// engine's `entropy` uses it), so every run encrypts with a different key.
fn seed() -> [u8; 32] {
    use std::hash::{BuildHasher, Hasher};
    let mut out = [0u8; 32];
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_usize(i);
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    out
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, alg, user, owner] = &a[..] else { panic!("usage: protect in out alg user owner") };
    let algorithm = match alg.as_str() {
        "rc4-40" => craft_pdf::cos::Algorithm::Rc4_40,
        "rc4-128" => craft_pdf::cos::Algorithm::Rc4_128,
        "aes-128" => craft_pdf::cos::Algorithm::Aes128,
        _ => craft_pdf::cos::Algorithm::Aes256,
    };
    let mut doc = craft_pdf::cos::Document::open(std::sync::Arc::new(std::fs::read(input).unwrap())).unwrap();
    doc.set_encryption(&craft_pdf::cos::NewEncryption {
        algorithm,
        user_password: user,
        owner_password: owner,
        permissions: -1,
        encrypt_metadata: true,
        seed: seed(),
    })
    .unwrap();
    std::fs::write(output, craft_pdf::cos::write_full(&doc, &Default::default()).unwrap()).unwrap();
}
