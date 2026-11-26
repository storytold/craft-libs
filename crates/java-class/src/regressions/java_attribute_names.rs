use crate::java_class::{self, ClassFile, Const};

#[derive(Clone, Copy, Debug)]
enum Scope {
    Class,
    Field,
    Method,
    Code,
}

fn u16b(out: &mut Vec<u8>, n: u16) {
    out.extend_from_slice(&n.to_be_bytes());
}

fn u32b(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}

fn utf8(out: &mut Vec<u8>, text: &str) {
    out.push(1);
    u16b(out, text.len() as u16);
    out.extend_from_slice(text.as_bytes());
}

fn attr(name: u16, info: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    u16b(&mut out, name);
    u32b(&mut out, info.len() as u32);
    out.extend_from_slice(info);
    out
}

fn attributes(out: &mut Vec<u8>, repeated: u16, known: &[Vec<u8>]) {
    u16b(out, repeated + known.len() as u16);
    for _ in 0..repeated {
        out.extend_from_slice(&attr(5, &[]));
    }
    for bytes in known {
        out.extend_from_slice(bytes);
    }
}

// A valid shared CONSTANT_Utf8 is named by every unknown attribute. Known
// attributes follow it, so avoiding name copies must not discard their facts.
fn fixture(scope: Scope, repeated: u16) -> Vec<u8> {
    let mut out = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52];
    u16b(&mut out, 17);
    utf8(&mut out, "AttributeFixture"); // 1
    out.push(7);
    u16b(&mut out, 1); // 2: this class
    utf8(&mut out, "java/lang/Object"); // 3
    out.push(7);
    u16b(&mut out, 3); // 4: superclass
    utf8(&mut out, &format!("org.example.{}", "A".repeat(usize::from(u16::MAX) - 12))); // 5
    for text in ["SourceFile", "AttributeFixture.java", "field", "I"] {
        utf8(&mut out, text); // 6..9
    }
    out.push(3);
    u32b(&mut out, 42); // 10
    for text in ["ConstantValue", "run", "()V", "Code", "LineNumberTable", "Deprecated"] {
        utf8(&mut out, text); // 11..16
    }
    for word in [0x0021, 2, 4, 0, 1, 0x0019, 8, 9] {
        u16b(&mut out, word);
    }
    attributes(&mut out, if matches!(scope, Scope::Field) { repeated } else { 0 }, &[attr(11, &10u16.to_be_bytes()), attr(16, &[])]);
    for word in [1, 0x0009, 12, 13] {
        u16b(&mut out, word);
    }
    let mut code = vec![0, 0, 0, 0, 0, 0, 0, 1, 0xb1, 0, 0];
    attributes(&mut code, if matches!(scope, Scope::Code) { repeated } else { 0 }, &[attr(15, &[0, 1, 0, 0, 0, 7])]);
    attributes(&mut out, if matches!(scope, Scope::Method) { repeated } else { 0 }, &[attr(14, &code)]);
    attributes(&mut out, if matches!(scope, Scope::Class) { repeated } else { 0 }, &[attr(6, &7u16.to_be_bytes())]);
    out
}

fn check_facts(parsed: &ClassFile) {
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    assert_eq!(parsed.this_class, "AttributeFixture");
    assert_eq!(parsed.super_class.as_deref(), Some("java/lang/Object"));
    assert_eq!(parsed.source_file.as_deref(), Some("AttributeFixture.java"));
    assert_eq!(parsed.fields.len(), 1);
    assert_eq!(parsed.fields[0].name, "field");
    assert_eq!(parsed.fields[0].constant_value.as_deref(), Some("42"));
    assert!(parsed.fields[0].deprecated);
    assert_eq!(parsed.methods.len(), 1);
    assert_eq!(parsed.methods[0].name, "run");
    let code = parsed.methods[0].code.as_ref().unwrap();
    assert_eq!(code.bytecode, [0xb1]);
    assert_eq!(code.line_numbers, [(0, 7)]);
    assert!(matches!(&parsed.pool[5], Const::Utf8(name) if name.len() == usize::from(u16::MAX)));
}

#[test]
fn known_attributes_survive_repeated_unknown_names_at_every_scope() {
    for scope in [Scope::Class, Scope::Field, Scope::Method, Scope::Code] {
        check_facts(&java_class::parse(&fixture(scope, 64)).unwrap());
    }
}

fn memory_case(scope: Scope) {
    let bytes = fixture(scope, 4096);
    assert!(bytes.len() < 100_000);
    check_facts(&java_class::parse(&bytes).unwrap());
}

// Run each ignored case separately with a 128 MiB process address-space limit.
// The original parser copies 256 MiB of one attribute name; the fixture itself
// is under 100 KiB. These checks deliberately make no timing assertion.
#[test]
#[ignore = "run separately under a process memory limit"]
fn repeated_class_attribute_names_fit_in_bounded_memory() {
    memory_case(Scope::Class);
}

#[test]
#[ignore = "run separately under a process memory limit"]
fn repeated_field_attribute_names_fit_in_bounded_memory() {
    memory_case(Scope::Field);
}

#[test]
#[ignore = "run separately under a process memory limit"]
fn repeated_method_attribute_names_fit_in_bounded_memory() {
    memory_case(Scope::Method);
}

#[test]
#[ignore = "run separately under a process memory limit"]
fn repeated_code_attribute_names_fit_in_bounded_memory() {
    memory_case(Scope::Code);
}
