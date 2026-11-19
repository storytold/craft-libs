// Synthetic fixtures and mutations from Craft Anything testkit, same source revision.
/// Deterministic pseudo-random bytes (xorshift) for fuzz-like loops.
pub fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// Every way to damage a sample used by robustness tests: truncations at many lengths and
/// single-byte corruptions spread across the file.
pub fn mutations(sample: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let n = sample.len();
    let step = (n / 64).max(1);
    let mut len = 0;
    while len < n {
        out.push(sample[..len].to_vec());
        len += step;
    }
    for (i, seed) in (0..n).step_by(step).zip(1u64..) {
        let mut m = sample.to_vec();
        let noise = noise(seed, 8);
        for (j, v) in noise.iter().enumerate() {
            if let Some(b) = m.get_mut(i + j) {
                *b = *v;
            }
        }
        out.push(m);
        let mut m = sample.to_vec();
        if let Some(b) = m.get_mut(i) {
            *b = 0xff;
        }
        out.push(m);
    }
    out
}

pub mod formats {
    fn leb(mut v: u32, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    fn name(s: &str, out: &mut Vec<u8>) {
        leb(s.len() as u32, out);
        out.extend_from_slice(s.as_bytes());
    }

    fn section(id: u8, body: &[u8], out: &mut Vec<u8>) {
        out.push(id);
        leb(body.len() as u32, out);
        out.extend_from_slice(body);
    }

    fn custom(sname: &str, body: &[u8], out: &mut Vec<u8>) {
        let mut c = Vec::new();
        name(sname, &mut c);
        c.extend_from_slice(body);
        section(0, &c, out);
    }

    /// A small valid module: imports `env.log(i32)`, defines `add(i32,i32)->i32` and `_start`,
    /// one memory, one mutable global, one data segment, exports `add`, `memory` and `_start`, and
    /// carries `name`, `producers` and `sourceMappingURL` custom sections.
    pub fn wasm_module() -> Vec<u8> {
        wasm_module_with_source_mapping_url("demo.wasm.map")
    }

    /// The sample module with a caller-provided source map URL.
    pub fn wasm_module_with_source_mapping_url(url: &str) -> Vec<u8> {
        let mut m = b"\0asm\x01\0\0\0".to_vec();
        section(1, &[3, 0x60, 2, 0x7f, 0x7f, 1, 0x7f, 0x60, 0, 0, 0x60, 1, 0x7f, 0], &mut m);
        let mut imp = vec![1];
        name("env", &mut imp);
        name("log", &mut imp);
        imp.extend_from_slice(&[0, 2]);
        section(2, &imp, &mut m);
        section(3, &[2, 0, 1], &mut m);
        section(5, &[1, 0, 1], &mut m);
        section(6, &[1, 0x7f, 1, 0x41, 42, 0x0b], &mut m);
        let mut exp = vec![3];
        name("add", &mut exp);
        exp.extend_from_slice(&[0, 1]);
        name("memory", &mut exp);
        exp.extend_from_slice(&[2, 0]);
        name("_start", &mut exp);
        exp.extend_from_slice(&[0, 2]);
        section(7, &exp, &mut m);
        let add = [0u8, 0x20, 0, 0x20, 1, 0x6a, 0x0b];
        let start = [0u8, 0x41, 7, 0x10, 0, 0x0b];
        let mut code = vec![2, add.len() as u8];
        code.extend_from_slice(&add);
        code.push(start.len() as u8);
        code.extend_from_slice(&start);
        section(10, &code, &mut m);
        section(11, &[1, 0, 0x41, 0, 0x0b, 2, b'h', b'i'], &mut m);
        // name section
        let mut n = Vec::new();
        let mut module_name = Vec::new();
        name("demo", &mut module_name);
        n.push(0);
        leb(module_name.len() as u32, &mut n);
        n.extend_from_slice(&module_name);
        let mut fnames = vec![3];
        for (i, f) in ["log", "add", "_start"].iter().enumerate() {
            leb(i as u32, &mut fnames);
            name(f, &mut fnames);
        }
        n.push(1);
        leb(fnames.len() as u32, &mut n);
        n.extend_from_slice(&fnames);
        custom("name", &n, &mut m);
        let mut p = vec![2];
        name("language", &mut p);
        p.push(1);
        name("Rust", &mut p);
        name("", &mut p);
        name("processed-by", &mut p);
        p.push(1);
        name("rustc", &mut p);
        name("1.80.0", &mut p);
        custom("producers", &p, &mut m);
        let mut s = Vec::new();
        name(url, &mut s);
        custom("sourceMappingURL", &s, &mut m);
        let mut tf = vec![1, b'+'];
        name("mutable-globals", &mut tf);
        custom("target_features", &tf, &mut m);
        m
    }

    /// JavaScript glue that loads the module.
    pub const WASM_GLUE: &str =
        "const imports = { env: { log: (x) => console.log(x) } };\nfetch(\"./demo.wasm\").then(r => WebAssembly.instantiateStreaming(r, imports));\n";
}

pub fn leb(mut value: usize) -> Vec<u8> {
    let mut out = Vec::new();
    while value >= 128 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
    out
}

pub fn string(out: &mut Vec<u8>, value: &str) {
    out.extend(leb(value.len()));
    out.extend(value.as_bytes());
}
