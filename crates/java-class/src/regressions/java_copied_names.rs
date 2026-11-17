use crate::java_class::{self, ClassFile, MAX_COPIED_TEXT};

const BIG: usize = u16::MAX as usize;
// Each repeat names the 65,535-byte Utf8 at least once, so 1,100 repeats exceed the limit.
const REPEATS: u16 = 1100;

#[derive(Clone, Copy, Debug)]
enum Scope {
    Interfaces,
    Fields,
    Methods,
    Throws,
    Catches,
    InnerClasses,
}

fn u16b(out: &mut Vec<u8>, n: u16) {
    out.extend_from_slice(&n.to_be_bytes());
}

fn utf8(out: &mut Vec<u8>, text: &str) {
    out.push(1);
    u16b(out, text.len() as u16);
    out.extend_from_slice(text.as_bytes());
}

fn attr(out: &mut Vec<u8>, name: u16, info: &[u8]) {
    u16b(out, name);
    out.extend_from_slice(&(info.len() as u32).to_be_bytes());
    out.extend_from_slice(info);
}

// Every repeated entry names pool entry 1 (a 65,535-byte Utf8) directly or through the
// `Class` at entry 2. The file stays well under 100 KiB.
fn fixture(scope: Scope) -> Vec<u8> {
    let mut out = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52];
    u16b(&mut out, 10);
    utf8(&mut out, &"A".repeat(BIG)); // 1
    out.extend_from_slice(&[7, 0, 1]); // 2
    utf8(&mut out, "java/lang/Object"); // 3
    out.extend_from_slice(&[7, 0, 3]); // 4
    utf8(&mut out, "Sample"); // 5
    out.extend_from_slice(&[7, 0, 5]); // 6
    for text in ["Exceptions", "InnerClasses", "Code"] {
        utf8(&mut out, text); // 7..9
    }
    for word in [0x0021, 6, 4] {
        u16b(&mut out, word);
    }
    // Interfaces
    if matches!(scope, Scope::Interfaces) {
        u16b(&mut out, REPEATS);
        for _ in 0..REPEATS {
            u16b(&mut out, 2);
        }
    } else {
        u16b(&mut out, 0);
    }
    // Fields and methods named and described by entry 1.
    for table in [Scope::Fields, Scope::Methods] {
        if std::mem::discriminant(&scope) == std::mem::discriminant(&table) {
            u16b(&mut out, REPEATS);
            for _ in 0..REPEATS {
                for word in [0, 1, 1, 0] {
                    u16b(&mut out, word);
                }
            }
        } else if matches!((scope, table), (Scope::Throws | Scope::Catches, Scope::Methods)) {
            u16b(&mut out, 1);
            for word in [0x0009, 5, 5, 1] {
                u16b(&mut out, word);
            }
            let mut info = Vec::new();
            if matches!(scope, Scope::Throws) {
                u16b(&mut info, REPEATS);
                for _ in 0..REPEATS {
                    u16b(&mut info, 2);
                }
                attr(&mut out, 7, &info);
            } else {
                info.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1, 0xb1]);
                u16b(&mut info, REPEATS);
                for _ in 0..REPEATS {
                    for word in [0, 1, 0, 2] {
                        u16b(&mut info, word);
                    }
                }
                u16b(&mut info, 0);
                attr(&mut out, 9, &info);
            }
        } else {
            u16b(&mut out, 0);
        }
    }
    // Class attributes
    if matches!(scope, Scope::InnerClasses) {
        u16b(&mut out, 1);
        let mut info = Vec::new();
        u16b(&mut info, REPEATS);
        for _ in 0..REPEATS {
            for word in [2, 0, 1, 0] {
                u16b(&mut info, word);
            }
        }
        attr(&mut out, 8, &info);
    } else {
        u16b(&mut out, 0);
    }
    out
}

/// The copied names of one scope's repeated entries.
fn names(parsed: &ClassFile, scope: Scope) -> Vec<&str> {
    let mut out = Vec::new();
    match scope {
        Scope::Interfaces => out.extend(parsed.interfaces.iter().map(String::as_str)),
        Scope::Fields | Scope::Methods => {
            let members = if matches!(scope, Scope::Fields) { &parsed.fields } else { &parsed.methods };
            for m in members {
                out.extend([m.name.as_str(), m.descriptor.as_str()]);
            }
        }
        Scope::Throws => out.extend(parsed.methods.iter().flat_map(|m| m.exceptions.iter().map(String::as_str))),
        Scope::Catches => {
            let codes = parsed.methods.iter().filter_map(|m| m.code.as_ref());
            out.extend(codes.flat_map(|c| c.exceptions.iter().filter_map(|e| e.catch_type.as_deref())));
        }
        Scope::InnerClasses => {
            for row in &parsed.inner_classes {
                out.extend(row.inner.as_deref());
                out.extend(row.name.as_deref());
            }
        }
    }
    out
}

fn check(scope: Scope) {
    let bytes = fixture(scope);
    assert!(bytes.len() < 100_000, "{scope:?}: {}", bytes.len());
    let parsed = java_class::parse(&bytes).unwrap();
    let names = names(&parsed, scope);
    let per_entry = if matches!(scope, Scope::Fields | Scope::Methods | Scope::InnerClasses) { 2 } else { 1 };
    // Every entry is still listed; the budget replaces names, not rows.
    assert_eq!(names.len(), usize::from(REPEATS) * per_entry, "{scope:?}");
    let full = names.iter().filter(|n| n.len() == BIG).count();
    assert!(full >= 1, "{scope:?}: the first names are copied in full");
    assert!(full * BIG <= MAX_COPIED_TEXT, "{scope:?}: {} bytes of names copied", full * BIG);
    assert!(names.iter().any(|n| n.starts_with('<') && n.ends_with('>')), "{scope:?}: names past the budget are placeholders");
    assert!(parsed.warnings.iter().any(|w| w.contains("pool indices")), "{scope:?}: {:?}", parsed.warnings);
    assert_eq!(parsed.this_class, "Sample");
    assert_eq!(parsed.super_class.as_deref(), Some("java/lang/Object"));
}

#[test]
fn repeated_interface_names_stay_within_the_copy_budget() {
    check(Scope::Interfaces);
}

#[test]
fn repeated_field_names_stay_within_the_copy_budget() {
    check(Scope::Fields);
}

#[test]
fn repeated_method_names_stay_within_the_copy_budget() {
    check(Scope::Methods);
}

#[test]
fn repeated_thrown_type_names_stay_within_the_copy_budget() {
    check(Scope::Throws);
}

#[test]
fn repeated_caught_type_names_stay_within_the_copy_budget() {
    check(Scope::Catches);
}

#[test]
fn repeated_inner_class_names_stay_within_the_copy_budget() {
    check(Scope::InnerClasses);
}
