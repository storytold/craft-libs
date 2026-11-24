use crate::java_class::{self, ClassFile, Const, MAX_COPIED_TEXT, Names};
use std::cell::Cell;

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
    Constants,
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
// `Class` at entry 2 or the `String` at entry 11. The file stays well under 100 KiB.
fn fixture(scope: Scope) -> Vec<u8> {
    let mut out = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52];
    u16b(&mut out, 12);
    utf8(&mut out, &"A".repeat(BIG)); // 1
    out.extend_from_slice(&[7, 0, 1]); // 2
    utf8(&mut out, "java/lang/Object"); // 3
    out.extend_from_slice(&[7, 0, 3]); // 4
    utf8(&mut out, "Sample"); // 5
    out.extend_from_slice(&[7, 0, 5]); // 6
    for text in ["Exceptions", "InnerClasses", "Code", "ConstantValue"] {
        utf8(&mut out, text); // 7..10
    }
    out.extend_from_slice(&[8, 0, 1]); // 11
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
        } else if matches!((scope, table), (Scope::Constants, Scope::Fields)) {
            // Fields named `Sample` whose `ConstantValue` is the string at entry 11.
            u16b(&mut out, REPEATS);
            for _ in 0..REPEATS {
                for word in [0x0018, 5, 5, 1] {
                    u16b(&mut out, word);
                }
                attr(&mut out, 10, &[0, 11]);
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
        Scope::Constants => out.extend(parsed.fields.iter().filter_map(|m| m.constant_value.as_deref())),
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
    // A string constant is shown quoted, two bytes longer than its Utf8.
    let full: Vec<usize> = names.iter().map(|n| n.len()).filter(|&n| n >= BIG).collect();
    assert!(!full.is_empty(), "{scope:?}: the first names are copied in full");
    let copied: usize = full.iter().sum();
    assert!(copied <= MAX_COPIED_TEXT, "{scope:?}: {copied} bytes of names copied");
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

#[test]
fn repeated_constant_values_stay_within_the_copy_budget() {
    check(Scope::Constants);
}

thread_local! {
    static RENDERS: Cell<usize> = const { Cell::new(0) };
}

fn counted_render(pool: &[Const], idx: u16) -> String {
    RENDERS.with(|n| n.set(n.get() + 1));
    match pool.get(usize::from(idx)) {
        Some(Const::Utf8(s)) => format!("{s:?}"),
        Some(Const::String(s)) => match pool.get(usize::from(*s)) {
            Some(Const::Utf8(text)) => format!("{text:?}"),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

#[test]
fn text_over_the_budget_is_rendered_once() {
    let pool = vec![Const::Unusable, Const::Utf8("A".repeat(BIG))];
    let mut names = Names::new(&pool);
    RENDERS.with(|n| n.set(0));
    let kept: Vec<String> = (0..2 * MAX_COPIED_TEXT / BIG).map(|_| names.keep("const", 1, counted_render)).collect();
    let copied = kept.iter().filter(|t| t.len() == BIG + 2).count();
    assert!(copied >= 1 && copied < kept.len(), "{copied} of {} copied", kept.len());
    assert_eq!(kept.last().map(String::as_str), Some("<const#1>"));
    // Only the copied texts are rendered; the refusals reuse the length kept from the first one.
    assert_eq!(RENDERS.with(Cell::get), copied);
}

#[test]
fn string_constants_over_one_utf8_are_rendered_once_past_the_budget() {
    let entries = 2 * MAX_COPIED_TEXT / BIG;
    let mut pool = vec![Const::Unusable, Const::Utf8("A".repeat(BIG))];
    pool.extend((0..entries).map(|_| Const::String(1)));
    let mut names = Names::new(&pool);
    RENDERS.with(|n| n.set(0));
    let kept: Vec<String> = (2..pool.len()).map(|i| names.constant(i as u16, counted_render)).collect();
    let copied = kept.iter().filter(|t| t.len() == BIG + 2).count();
    assert!(copied >= 1 && copied < kept.len(), "{copied} of {} copied", kept.len());
    assert_eq!(kept.last().map(String::as_str), Some(format!("<const#{}>", pool.len() - 1).as_str()));
    // Each `String` entry is its own pool index, but all of them quote entry 1, so the refusals
    // reuse the length kept for entry 1 instead of building the text again.
    assert_eq!(RENDERS.with(Cell::get), copied);
}
