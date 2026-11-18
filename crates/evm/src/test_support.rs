pub mod formats {
    pub const EVM_SELECTORS: [u32; 3] = [0xa905_9cbb, 0x70a0_8231, 0xd0e3_0db0];

    /// Runtime bytecode with a compiler-style dispatcher, a fallback that reverts, two non-payable
    /// functions (CALLVALUE check), one payable function, and a CBOR metadata tail (ipfs + solc 0.8.19).
    /// Returns (bytecode, jump destinations of the three functions).
    pub fn evm_runtime() -> (Vec<u8>, [usize; 3]) {
        let mut c: Vec<u8> = vec![0x60, 0x80, 0x60, 0x40, 0x52];
        // if calldatasize < 4 goto fallback
        c.extend_from_slice(&[0x60, 0x04, 0x36, 0x10, 0x61, 0, 0]);
        let fb_patch = c.len() - 2;
        c.push(0x57);
        c.extend_from_slice(&[0x60, 0x00, 0x35, 0x60, 0xe0, 0x1c]);
        let mut patches = Vec::new();
        for sel in EVM_SELECTORS {
            c.push(0x80);
            c.push(0x63);
            c.extend_from_slice(&sel.to_be_bytes());
            c.extend_from_slice(&[0x14, 0x61, 0, 0]);
            patches.push(c.len() - 2);
            c.push(0x57);
        }
        let fallback = c.len();
        c.extend_from_slice(&[0x5b, 0x60, 0x00, 0x80, 0xfd]);
        let mut dests = [0usize; 3];
        for (i, dest) in dests.iter_mut().enumerate() {
            *dest = c.len();
            if i < 2 {
                // JUMPDEST CALLVALUE DUP1 ISZERO PUSH2 ok JUMPI PUSH1 0 DUP1 REVERT JUMPDEST POP STOP
                c.extend_from_slice(&[0x5b, 0x34, 0x80, 0x15, 0x61, 0, 0]);
                let ok_patch = c.len() - 2;
                c.extend_from_slice(&[0x57, 0x60, 0x00, 0x80, 0xfd]);
                let ok = c.len();
                c.extend_from_slice(&[0x5b, 0x50, 0x00]);
                c[ok_patch..ok_patch + 2].copy_from_slice(&(ok as u16).to_be_bytes());
            } else {
                c.extend_from_slice(&[0x5b, 0x60, 0x01, 0x60, 0x00, 0x55, 0x00]);
            }
        }
        c[fb_patch..fb_patch + 2].copy_from_slice(&(fallback as u16).to_be_bytes());
        for (p, d) in patches.iter().zip(dests) {
            c[*p..*p + 2].copy_from_slice(&(d as u16).to_be_bytes());
        }
        c.push(0xfe);
        let mut meta = vec![0xa2, 0x64, b'i', b'p', b'f', b's', 0x58, 0x22, 0x12, 0x20];
        meta.extend((0..32u8).map(|i| i.wrapping_mul(7)));
        meta.extend_from_slice(&[0x64, b's', b'o', b'l', b'c', 0x43, 0x00, 0x08, 0x13]);
        let len = meta.len() as u16;
        c.extend_from_slice(&meta);
        c.extend_from_slice(&len.to_be_bytes());
        (c, dests)
    }

    /// Bytecode as `0x`-prefixed lowercase hex.
    pub fn to_hex(b: &[u8]) -> String {
        let mut s = String::from("0x");
        for x in b {
            s.push_str(&format!("{x:02x}"));
        }
        s
    }
}
pub use craft_testkit::sample_damage::{mutations, noise};
