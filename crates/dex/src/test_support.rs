// Synthetic DEX fixture and deterministic mutations retained from Craft Anything.
pub use craft_testkit::sample_damage::{mutations, noise};

pub mod android {
    pub const HELLO: &str = "Hello, Craft!";
    fn u16le(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn u32le(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn uleb(v: &mut Vec<u8>, mut x: u32) {
        loop {
            let b = (x & 0x7f) as u8;
            x >>= 7;
            if x == 0 {
                v.push(b);
                break;
            }
            v.push(b | 0x80);
        }
    }
    fn sleb(v: &mut Vec<u8>, mut x: i32) {
        loop {
            let b = (x & 0x7f) as u8;
            x >>= 7;
            let done = (x == 0 && b & 0x40 == 0) || (x == -1 && b & 0x40 != 0);
            v.push(if done { b } else { b | 0x80 });
            if done {
                break;
            }
        }
    }
    fn align4(v: &mut Vec<u8>) {
        while !v.len().is_multiple_of(4) {
            v.push(0);
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct TrySpec {
        pub start: u32,
        pub count: u16,
        /// (type idx, handler address)
        pub handlers: Vec<(u32, u32)>,
        pub catch_all: Option<u32>,
    }

    #[derive(Debug, Clone, Default)]
    pub struct CodeSpec {
        pub registers: u16,
        pub ins: u16,
        pub outs: u16,
        pub insns: Vec<u16>,
        pub tries: Vec<TrySpec>,
    }

    #[derive(Debug, Clone)]
    pub struct MethodSpec {
        pub method: u32,
        pub access: u32,
        pub code: Option<CodeSpec>,
    }

    #[derive(Debug, Clone, Default)]
    pub struct ClassSpec {
        pub class: u32,
        pub access: u32,
        pub superclass: u32,
        pub interfaces: Vec<u32>,
        pub source_file: u32,
        pub annotations: Vec<u32>,
        pub static_fields: Vec<(u32, u32)>,
        pub instance_fields: Vec<(u32, u32)>,
        pub direct: Vec<MethodSpec>,
        pub virtuals: Vec<MethodSpec>,
    }

    #[derive(Debug, Clone, Default)]
    pub struct DexBuilder {
        strings: Vec<String>,
        types: Vec<u32>,
        protos: Vec<(u32, u32, Vec<u32>)>,
        fields: Vec<(u32, u32, u32)>,
        methods: Vec<(u32, u32, u32)>,
        pub classes: Vec<ClassSpec>,
    }

    impl DexBuilder {
        pub fn string(&mut self, s: &str) -> u32 {
            if let Some(i) = self.strings.iter().position(|x| x == s) {
                return i as u32;
            }
            self.strings.push(s.to_string());
            (self.strings.len() - 1) as u32
        }

        pub fn ty(&mut self, desc: &str) -> u32 {
            let s = self.string(desc);
            if let Some(i) = self.types.iter().position(|&x| x == s) {
                return i as u32;
            }
            self.types.push(s);
            (self.types.len() - 1) as u32
        }

        pub fn proto(&mut self, ret: &str, params: &[&str]) -> u32 {
            let short = |d: &str| if d.starts_with('L') || d.starts_with('[') { 'L' } else { d.chars().next().unwrap() };
            let shorty: String = std::iter::once(short(ret)).chain(params.iter().map(|p| short(p))).collect();
            let s = self.string(&shorty);
            let r = self.ty(ret);
            let ps: Vec<u32> = params.iter().map(|p| self.ty(p)).collect();
            if let Some(i) = self.protos.iter().position(|p| p.1 == r && p.2 == ps) {
                return i as u32;
            }
            self.protos.push((s, r, ps));
            (self.protos.len() - 1) as u32
        }

        pub fn field(&mut self, cls: &str, ty: &str, name: &str) -> u32 {
            let c = self.ty(cls);
            let t = self.ty(ty);
            let n = self.string(name);
            if let Some(i) = self.fields.iter().position(|&f| f == (c, t, n)) {
                return i as u32;
            }
            self.fields.push((c, t, n));
            (self.fields.len() - 1) as u32
        }

        pub fn method(&mut self, cls: &str, name: &str, ret: &str, params: &[&str]) -> u32 {
            let c = self.ty(cls);
            let p = self.proto(ret, params);
            let n = self.string(name);
            if let Some(i) = self.methods.iter().position(|&m| m == (c, p, n)) {
                return i as u32;
            }
            self.methods.push((c, p, n));
            (self.methods.len() - 1) as u32
        }

        pub fn build(self) -> Vec<u8> {
            let ns = self.strings.len();
            let nt = self.types.len();
            let np = self.protos.len();
            let nf = self.fields.len();
            let nm = self.methods.len();
            let nc = self.classes.len();
            let string_ids = 0x70;
            let type_ids = string_ids + ns * 4;
            let proto_ids = type_ids + nt * 4;
            let field_ids = proto_ids + np * 12;
            let method_ids = field_ids + nf * 8;
            let class_defs = method_ids + nm * 8;
            let mut data_start = class_defs + nc * 32;
            data_start = (data_start + 3) & !3;
            let mut d: Vec<u8> = Vec::new();
            let at = |d: &Vec<u8>| (data_start + d.len()) as u32;
            // type lists
            let tl = |d: &mut Vec<u8>, list: &[u32]| -> u32 {
                if list.is_empty() {
                    return 0;
                }
                align4(d);
                let o = at(d);
                u32le(d, list.len() as u32);
                for &t in list {
                    u16le(d, t as u16);
                }
                o
            };
            let proto_params: Vec<u32> = self.protos.iter().map(|p| tl(&mut d, &p.2)).collect();
            let class_ifaces: Vec<u32> = self.classes.iter().map(|c| tl(&mut d, &c.interfaces)).collect();
            // string data
            let mut string_offs = Vec::new();
            for s in &self.strings {
                string_offs.push(at(&d));
                uleb(&mut d, s.encode_utf16().count() as u32);
                for c in s.encode_utf16() {
                    if c == 0 {
                        d.extend_from_slice(&[0xc0, 0x80]);
                    } else if c < 0x80 {
                        d.push(c as u8);
                    } else if c < 0x800 {
                        d.extend_from_slice(&[0xc0 | (c >> 6) as u8, 0x80 | (c & 0x3f) as u8]);
                    } else {
                        d.extend_from_slice(&[0xe0 | (c >> 12) as u8, 0x80 | ((c >> 6) & 0x3f) as u8, 0x80 | (c & 0x3f) as u8]);
                    }
                }
                d.push(0);
            }
            // code items, class data, annotations
            let mut class_data_offs = Vec::new();
            let mut annot_offs = Vec::new();
            for c in &self.classes {
                let code_off = |d: &mut Vec<u8>, m: &MethodSpec| -> u32 {
                    let Some(code) = &m.code else { return 0 };
                    align4(d);
                    let o = at(d);
                    u16le(d, code.registers);
                    u16le(d, code.ins);
                    u16le(d, code.outs);
                    u16le(d, code.tries.len() as u16);
                    u32le(d, 0);
                    u32le(d, code.insns.len() as u32);
                    for &i in &code.insns {
                        u16le(d, i);
                    }
                    if !code.tries.is_empty() {
                        if code.insns.len() % 2 == 1 {
                            u16le(d, 0);
                        }
                        let mut list = Vec::new();
                        uleb(&mut list, code.tries.len() as u32);
                        let mut offs = Vec::new();
                        for t in &code.tries {
                            offs.push(list.len() as u16);
                            let n = t.handlers.len() as i32;
                            sleb(&mut list, if t.catch_all.is_some() { -n } else { n });
                            for &(ty, addr) in &t.handlers {
                                uleb(&mut list, ty);
                                uleb(&mut list, addr);
                            }
                            if let Some(a) = t.catch_all {
                                uleb(&mut list, a);
                            }
                        }
                        for (t, off) in code.tries.iter().zip(offs) {
                            u32le(d, t.start);
                            u16le(d, t.count);
                            u16le(d, off);
                        }
                        d.extend_from_slice(&list);
                    }
                    o
                };
                let mut direct = c.direct.clone();
                direct.sort_by_key(|m| m.method);
                let mut virtuals = c.virtuals.clone();
                virtuals.sort_by_key(|m| m.method);
                let direct_offs: Vec<u32> = direct.iter().map(|m| code_off(&mut d, m)).collect();
                let virtual_offs: Vec<u32> = virtuals.iter().map(|m| code_off(&mut d, m)).collect();
                let cd = at(&d);
                let mut sf = c.static_fields.clone();
                sf.sort();
                let mut inf = c.instance_fields.clone();
                inf.sort();
                uleb(&mut d, sf.len() as u32);
                uleb(&mut d, inf.len() as u32);
                uleb(&mut d, direct.len() as u32);
                uleb(&mut d, virtuals.len() as u32);
                for list in [&sf, &inf] {
                    let mut prev = 0;
                    for &(f, a) in list.iter() {
                        uleb(&mut d, f - prev);
                        uleb(&mut d, a);
                        prev = f;
                    }
                }
                for (list, offs) in [(&direct, &direct_offs), (&virtuals, &virtual_offs)] {
                    let mut prev = 0;
                    for (m, &o) in list.iter().zip(offs.iter()) {
                        uleb(&mut d, m.method - prev);
                        uleb(&mut d, m.access);
                        uleb(&mut d, o);
                        prev = m.method;
                    }
                }
                class_data_offs.push(cd);
                if c.annotations.is_empty() {
                    annot_offs.push(0);
                } else {
                    let mut items = Vec::new();
                    for &t in &c.annotations {
                        items.push(at(&d));
                        d.push(1); // runtime visibility
                        uleb(&mut d, t);
                        uleb(&mut d, 0);
                    }
                    align4(&mut d);
                    let set_off = at(&d);
                    u32le(&mut d, items.len() as u32);
                    for i in items {
                        u32le(&mut d, i);
                    }
                    let dir = at(&d);
                    u32le(&mut d, set_off);
                    u32le(&mut d, 0);
                    u32le(&mut d, 0);
                    u32le(&mut d, 0);
                    annot_offs.push(dir);
                }
            }
            align4(&mut d);
            let file_size = data_start + d.len();
            let mut out = vec![0u8; data_start];
            out[..8].copy_from_slice(b"dex\n035\0");
            let put = |out: &mut Vec<u8>, off: usize, v: u32| out[off..off + 4].copy_from_slice(&v.to_le_bytes());
            put(&mut out, 32, file_size as u32);
            put(&mut out, 36, 0x70);
            put(&mut out, 40, 0x1234_5678);
            for (i, (n, o)) in
                [(ns, string_ids), (nt, type_ids), (np, proto_ids), (nf, field_ids), (nm, method_ids), (nc, class_defs)].iter().enumerate()
            {
                put(&mut out, 56 + i * 8, *n as u32);
                put(&mut out, 60 + i * 8, if *n == 0 { 0 } else { *o as u32 });
            }
            put(&mut out, 104, d.len() as u32);
            put(&mut out, 108, data_start as u32);
            for (i, o) in string_offs.iter().enumerate() {
                put(&mut out, string_ids + i * 4, *o);
            }
            for (i, t) in self.types.iter().enumerate() {
                put(&mut out, type_ids + i * 4, *t);
            }
            for (i, p) in self.protos.iter().enumerate() {
                put(&mut out, proto_ids + i * 12, p.0);
                put(&mut out, proto_ids + i * 12 + 4, p.1);
                put(&mut out, proto_ids + i * 12 + 8, proto_params[i]);
            }
            for (i, (c, t, n)) in self.fields.iter().enumerate() {
                out[field_ids + i * 8..field_ids + i * 8 + 2].copy_from_slice(&(*c as u16).to_le_bytes());
                out[field_ids + i * 8 + 2..field_ids + i * 8 + 4].copy_from_slice(&(*t as u16).to_le_bytes());
                put(&mut out, field_ids + i * 8 + 4, *n);
            }
            for (i, (c, p, n)) in self.methods.iter().enumerate() {
                out[method_ids + i * 8..method_ids + i * 8 + 2].copy_from_slice(&(*c as u16).to_le_bytes());
                out[method_ids + i * 8 + 2..method_ids + i * 8 + 4].copy_from_slice(&(*p as u16).to_le_bytes());
                put(&mut out, method_ids + i * 8 + 4, *n);
            }
            for (i, c) in self.classes.iter().enumerate() {
                let o = class_defs + i * 32;
                for (k, v) in
                    [c.class, c.access, c.superclass, class_ifaces[i], c.source_file, annot_offs[i], class_data_offs[i], 0].iter().enumerate()
                {
                    put(&mut out, o + k * 4, *v);
                }
            }
            out.extend_from_slice(&d);
            let sum = adler32(&out[12..]);
            put(&mut out, 8, sum);
            out
        }
    }

    fn adler32(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &x in data {
            a = (a + u32::from(x)) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }

    const MAIN: &str = "Lcom/example/notes/MainActivity;";
    const UTIL: &str = "Lcom/example/notes/Util;";
    const STRING: &str = "Ljava/lang/String;";

    /// `classes.dex` of the Notes app: `MainActivity` (constructor, `onCreate`, `run`, `pick` with a
    /// packed switch inside a try/catch) and `Util` (two `log` overloads printing through
    /// `System.out`).
    pub fn notes_dex() -> Vec<u8> {
        let mut b = DexBuilder::default();
        let activity = b.ty("Landroid/app/Activity;");
        let main = b.ty(MAIN);
        let runnable = b.ty("Ljava/lang/Runnable;");
        let deprecated = b.ty("Ljava/lang/Deprecated;");
        let rte = b.ty("Ljava/lang/RuntimeException;");
        let object = b.ty("Ljava/lang/Object;");
        let util = b.ty(UTIL);
        let tag = b.field(MAIN, STRING, "TAG");
        let count = b.field(MAIN, "I", "count");
        let out = b.field("Ljava/lang/System;", "Ljava/io/PrintStream;", "out");
        let super_init = b.method("Landroid/app/Activity;", "<init>", "V", &[]);
        let super_create = b.method("Landroid/app/Activity;", "onCreate", "V", &["Landroid/os/Bundle;"]);
        let init = b.method(MAIN, "<init>", "V", &[]);
        let on_create = b.method(MAIN, "onCreate", "V", &["Landroid/os/Bundle;"]);
        let run = b.method(MAIN, "run", "V", &[]);
        let pick = b.method(MAIN, "pick", "I", &["I"]);
        let log_s = b.method(UTIL, "log", "V", &[STRING]);
        let log_i = b.method(UTIL, "log", "V", &["I"]);
        let println = b.method("Ljava/io/PrintStream;", "println", "V", &[STRING]);
        let hello = b.string(HELLO);
        let main_src = b.string("MainActivity.java");
        let util_src = b.string("Util.java");
        let code = |registers, ins, outs, insns: Vec<u16>| Some(CodeSpec { registers, ins, outs, insns, tries: vec![] });
        b.classes.push(ClassSpec {
            class: main,
            access: 0x1,
            superclass: activity,
            interfaces: vec![runnable],
            source_file: main_src,
            annotations: vec![deprecated],
            static_fields: vec![(tag, 0x1a)],
            instance_fields: vec![(count, 0x2)],
            direct: vec![MethodSpec { method: init, access: 0x10001, code: code(1, 1, 1, vec![0x1070, super_init as u16, 0x0000, 0x000e]) }],
            virtuals: vec![
                MethodSpec {
                    method: on_create,
                    access: 0x1,
                    code: code(3, 2, 2, vec![0x001a, hello as u16, 0x1071, log_s as u16, 0x0000, 0x206f, super_create as u16, 0x0021, 0x000e]),
                },
                MethodSpec { method: run, access: 0x1, code: code(1, 1, 1, vec![0x7012, 0x1071, log_i as u16, 0x0000, 0x000e]) },
                MethodSpec {
                    method: pick,
                    access: 0x1,
                    code: Some(CodeSpec {
                        registers: 3,
                        ins: 2,
                        outs: 0,
                        insns: vec![0x022b, 10, 0, 0x0012, 0x000f, 0x1012, 0x000f, 0x2012, 0x000f, 0x0000, 0x0100, 2, 1, 0, 5, 0, 7, 0],
                        tries: vec![TrySpec { start: 0, count: 3, handlers: vec![(rte, 3)], catch_all: Some(3) }],
                    }),
                },
            ],
        });
        b.classes.push(ClassSpec {
            class: util,
            access: 0x11,
            superclass: object,
            source_file: util_src,
            direct: vec![
                MethodSpec { method: log_s, access: 0x9, code: code(2, 1, 2, vec![0x0062, out as u16, 0x206e, println as u16, 0x0010, 0x000e]) },
                MethodSpec { method: log_i, access: 0x9, code: code(1, 1, 0, vec![0x000e]) },
            ],
            ..ClassSpec::default()
        });
        b.build()
    }
}
