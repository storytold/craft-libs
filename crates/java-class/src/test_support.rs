pub use craft_testkit::sample_damage::{mutations, noise};

pub mod java {
    use std::collections::HashMap;

    /// A constant-pool builder that interns entries and tracks the double slot after `Long`/`Double`.
    #[derive(Default)]
    struct Pool {
        items: Vec<Vec<u8>>,
        /// Next free index (1-based; accounts for long/double taking two slots).
        count: u16,
        cache: HashMap<String, u16>,
    }

    impl Pool {
        fn new() -> Self {
            Pool { items: Vec::new(), count: 1, cache: HashMap::new() }
        }
        fn add(&mut self, bytes: Vec<u8>, slots: u16) -> u16 {
            let idx = self.count;
            self.items.push(bytes);
            self.count += slots;
            idx
        }
        fn utf8(&mut self, s: &str) -> u16 {
            let key = format!("u:{s}");
            if let Some(&i) = self.cache.get(&key) {
                return i;
            }
            let mut b = vec![1u8];
            b.extend_from_slice(&(s.len() as u16).to_be_bytes());
            b.extend_from_slice(s.as_bytes());
            let i = self.add(b, 1);
            self.cache.insert(key, i);
            i
        }
        fn class(&mut self, name: &str) -> u16 {
            let n = self.utf8(name);
            let key = format!("c:{name}");
            if let Some(&i) = self.cache.get(&key) {
                return i;
            }
            let mut b = vec![7u8];
            b.extend_from_slice(&n.to_be_bytes());
            let i = self.add(b, 1);
            self.cache.insert(key, i);
            i
        }
        fn string(&mut self, s: &str) -> u16 {
            let u = self.utf8(s);
            let mut b = vec![8u8];
            b.extend_from_slice(&u.to_be_bytes());
            self.add(b, 1)
        }
        fn nat(&mut self, name: &str, desc: &str) -> u16 {
            let n = self.utf8(name);
            let d = self.utf8(desc);
            let mut b = vec![12u8];
            b.extend_from_slice(&n.to_be_bytes());
            b.extend_from_slice(&d.to_be_bytes());
            self.add(b, 1)
        }
        fn reff(&mut self, tag: u8, owner: &str, name: &str, desc: &str) -> u16 {
            let c = self.class(owner);
            let nt = self.nat(name, desc);
            let mut b = vec![tag];
            b.extend_from_slice(&c.to_be_bytes());
            b.extend_from_slice(&nt.to_be_bytes());
            self.add(b, 1)
        }
        fn long(&mut self, v: i64) -> u16 {
            let mut b = vec![5u8];
            b.extend_from_slice(&v.to_be_bytes());
            self.add(b, 2)
        }
        /// The serialized constant pool and the count field.
        fn finish(&self) -> (Vec<u8>, u16) {
            (self.items.concat(), self.count)
        }
    }

    fn u16b(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_be_bytes());
    }
    fn u32b(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_be_bytes());
    }

    /// Build an attribute: name_index, length, body.
    fn attr(name_idx: u16, body: Vec<u8>) -> Vec<u8> {
        let mut v = Vec::new();
        u16b(&mut v, name_idx);
        u32b(&mut v, body.len() as u32);
        v.extend_from_slice(&body);
        v
    }

    /// A method's `Code` attribute body (max_stack, max_locals, code, exception table, no nested attrs).
    fn code_body(max_stack: u16, max_locals: u16, code: &[u8], exc: &[(u16, u16, u16, u16)]) -> Vec<u8> {
        let mut v = Vec::new();
        u16b(&mut v, max_stack);
        u16b(&mut v, max_locals);
        u32b(&mut v, code.len() as u32);
        v.extend_from_slice(code);
        u16b(&mut v, exc.len() as u16);
        for (s, e, h, t) in exc {
            u16b(&mut v, *s);
            u16b(&mut v, *e);
            u16b(&mut v, *h);
            u16b(&mut v, *t);
        }
        u16b(&mut v, 0); // nested attributes count
        v
    }

    /// A method_info: access, name, descriptor, and one already-built Code attribute.
    fn method(p: &mut Pool, access: u16, name: &str, desc: &str, code_attribute: Vec<u8>) -> Vec<u8> {
        let name_idx = p.utf8(name);
        let desc_idx = p.utf8(desc);
        let mut v = Vec::new();
        u16b(&mut v, access);
        u16b(&mut v, name_idx);
        u16b(&mut v, desc_idx);
        u16b(&mut v, 1); // one attribute
        v.extend_from_slice(&code_attribute);
        v
    }

    /// Assemble a whole class from its parts.
    #[allow(clippy::too_many_arguments)]
    fn assemble(
        pool: &Pool,
        access: u16,
        this_c: u16,
        super_c: u16,
        interfaces: &[u16],
        fields: &[Vec<u8>],
        methods: &[Vec<u8>],
        class_attrs: &[Vec<u8>],
    ) -> Vec<u8> {
        let (cp, count) = pool.finish();
        let mut v = Vec::new();
        v.extend_from_slice(&[0xca, 0xfe, 0xba, 0xbe]);
        u16b(&mut v, 0); // minor
        u16b(&mut v, 52); // major (Java 8)
        u16b(&mut v, count);
        v.extend_from_slice(&cp);
        u16b(&mut v, access);
        u16b(&mut v, this_c);
        u16b(&mut v, super_c);
        u16b(&mut v, interfaces.len() as u16);
        for i in interfaces {
            u16b(&mut v, *i);
        }
        u16b(&mut v, fields.len() as u16);
        for f in fields {
            v.extend_from_slice(f);
        }
        u16b(&mut v, methods.len() as u16);
        for m in methods {
            v.extend_from_slice(m);
        }
        u16b(&mut v, class_attrs.len() as u16);
        for a in class_attrs {
            v.extend_from_slice(a);
        }
        v
    }

    pub const HELLO: &str = "Hello, Craft!";

    /// `public class com.example.Hello implements Runnable` with a `count` field, a `BIG` long
    /// constant, `<init>`, `run()` (System.out.println("Hello, Craft!")), `pick(int)` (a tableswitch)
    /// and `guarded()` (a try/catch over the println call).
    pub fn hello_class() -> Vec<u8> {
        let mut p = Pool::new();
        let this_c = p.class("com/example/Hello");
        let obj_c = p.class("java/lang/Object");
        let runnable_c = p.class("java/lang/Runnable");
        let code_attr = p.utf8("Code");

        // fields: int count; static final long BIG;
        let count_name = p.utf8("count");
        let i_desc = p.utf8("I");
        let big_name = p.utf8("BIG");
        let j_desc = p.utf8("J");
        let constval_attr = p.utf8("ConstantValue");
        let big_val = p.long(0x1122_3344_5566_7788);

        // refs used by code
        let obj_init = p.reff(10, "java/lang/Object", "<init>", "()V");
        let sysout = p.reff(9, "java/lang/System", "out", "Ljava/io/PrintStream;");
        let hello = p.string(HELLO);
        let println = p.reff(10, "java/io/PrintStream", "println", "(Ljava/lang/String;)V");
        let rt_exc = p.class("java/lang/RuntimeException");

        // <init>(): aload_0; invokespecial Object.<init>; return
        let mut init = vec![0x2a, 0xb7];
        init.extend_from_slice(&obj_init.to_be_bytes());
        init.push(0xb1);
        let init_m = method(&mut p, 0x0001, "<init>", "()V", attr(code_attr, code_body(1, 1, &init, &[])));

        // run(): getstatic System.out; ldc "Hello"; invokevirtual println; return
        let mut run = vec![0xb2];
        run.extend_from_slice(&sysout.to_be_bytes());
        run.push(0x12);
        run.push(hello as u8); // ldc (1-byte index; pool is small)
        run.push(0xb6);
        run.extend_from_slice(&println.to_be_bytes());
        run.push(0xb1);
        let run_m = method(&mut p, 0x0001, "run", "()V", attr(code_attr, code_body(2, 1, &run, &[])));

        // pick(int): iload_0; tableswitch{0->L0,1->L1,default->Ldef}; L0: bipush 10; ireturn; L1: bipush 20; ireturn; Ldef: iconst_m1; ireturn
        // offsets: iload_1@0; tableswitch@1 (pad 2) ends @24; L0@24; L1@27; Ldef@30
        let mut pick = vec![0x1b, 0xaa, 0x00, 0x00];
        u32b(&mut pick, 29); // default -> 1+29 = 30
        u32b(&mut pick, 0); // low
        u32b(&mut pick, 1); // high
        u32b(&mut pick, 23); // case 0 -> 1+23 = 24
        u32b(&mut pick, 26); // case 1 -> 1+26 = 27
        debug_assert_eq!(pick.len(), 24);
        pick.extend_from_slice(&[0x10, 10, 0xac]); // @24: bipush 10; ireturn
        pick.extend_from_slice(&[0x10, 20, 0xac]); // @27: bipush 20; ireturn
        pick.extend_from_slice(&[0x02, 0xac]); // @30: iconst_m1; ireturn
        let pick_m = method(&mut p, 0x0001, "pick", "(I)I", attr(code_attr, code_body(1, 1, &pick, &[])));

        // guarded(): try { System.out.println("Hello"); } catch (RuntimeException e) {}  (then return)
        // 0: getstatic(3) ; 3: ldc(2) ; 5: invokevirtual(3) ; 8: goto +5 ->13 ; 11: astore_1 ; 12: return ; 13: return
        let mut g = vec![0xb2];
        g.extend_from_slice(&sysout.to_be_bytes());
        g.push(0x12);
        g.push(hello as u8);
        g.push(0xb6);
        g.extend_from_slice(&println.to_be_bytes());
        g.extend_from_slice(&[0xa7, 0x00, 0x05]); // @8 goto -> 13
        g.push(0x4c); // @11 astore_1
        g.push(0xb1); // @12 return
        g.push(0xb1); // @13 return
        let guarded_m = method(&mut p, 0x0001, "guarded", "()V", attr(code_attr, code_body(2, 2, &g, &[(0, 8, 11, rt_exc)])));

        // field infos
        let field = |p: &mut Pool, access: u16, name_idx: u16, desc_idx: u16, attrs: &[Vec<u8>]| -> Vec<u8> {
            let _ = p;
            let mut v = Vec::new();
            u16b(&mut v, access);
            u16b(&mut v, name_idx);
            u16b(&mut v, desc_idx);
            u16b(&mut v, attrs.len() as u16);
            for a in attrs {
                v.extend_from_slice(a);
            }
            v
        };
        let count_f = field(&mut p, 0x0002, count_name, i_desc, &[]); // private int count
        let mut cv = Vec::new();
        u16b(&mut cv, big_val);
        let big_f = field(&mut p, 0x0019, big_name, j_desc, &[attr(constval_attr, cv)]); // public static final long BIG

        let source_attr = p.utf8("SourceFile");
        let source_val = p.utf8("Hello.java");
        let mut sf = Vec::new();
        u16b(&mut sf, source_val);
        let source = attr(source_attr, sf);

        assemble(
            &p,
            0x0021, // public super
            this_c,
            obj_c,
            &[runnable_c],
            &[count_f, big_f],
            &[init_m, run_m, pick_m, guarded_m],
            &[source],
        )
    }
}
