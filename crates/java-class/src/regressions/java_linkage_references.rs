use crate::java_class::{parse, references};

/// A method with a chosen load or call, plus a dynamic call site and a dynamic constant with distinct bootstrap signatures.
fn class(code: &[u8]) -> Vec<u8> {
    fn utf(s: &str) -> Vec<u8> {
        let mut b = vec![1];
        b.extend((s.len() as u16).to_be_bytes());
        b.extend(s.bytes());
        b
    }
    let pool = [
        utf("Uses"),
        vec![7, 0, 1],
        utf("java/lang/Object"),
        vec![7, 0, 3],
        utf("run"),
        utf("()V"),
        utf("Code"),
        utf("BootstrapMethods"),
        utf("Target"),
        vec![7, 0, 9],
        utf("apply"),
        vec![12, 0, 11, 0, 6],
        vec![10, 0, 10, 0, 12],
        vec![15, 6, 0, 13],
        utf("Factory"),
        vec![7, 0, 15],
        utf("boot"),
        vec![12, 0, 17, 0, 28],
        vec![10, 0, 16, 0, 18],
        vec![15, 6, 0, 19],
        utf("([LTarget;)V"),
        vec![16, 0, 21],
        vec![18, 0, 0, 0, 12],
        utf("value"),
        utf("I"),
        vec![12, 0, 24, 0, 25],
        vec![17, 0, 1, 0, 26],
        utf(
            "(Ljava/lang/invoke/MethodHandles$Lookup;Ljava/lang/String;Ljava/lang/invoke/MethodType;Ljava/lang/invoke/MethodHandle;)Ljava/lang/invoke/CallSite;",
        ),
        utf("(Ljava/lang/invoke/MethodHandles$Lookup;Ljava/lang/String;Ljava/lang/Class;)I"),
        vec![12, 0, 17, 0, 29],
        vec![10, 0, 16, 0, 30],
        vec![15, 6, 0, 31],
    ];
    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 55];
    bytes.extend((pool.len() as u16 + 1).to_be_bytes());
    for e in pool {
        bytes.extend(e);
    }
    bytes.extend([0, 0x21, 0, 2, 0, 4, 0, 0, 0, 0, 0, 1, 0, 9, 0, 5, 0, 6, 0, 1, 0, 7]);
    bytes.extend(((12 + code.len()) as u32).to_be_bytes());
    bytes.extend([0, 3, 0, 0]);
    bytes.extend((code.len() as u32).to_be_bytes());
    bytes.extend(code);
    bytes.extend([0, 0, 0, 0]);
    let boots = [0, 2, 0, 20, 0, 1, 0, 14, 0, 32, 0, 0];
    bytes.extend([0, 1, 0, 8]);
    bytes.extend((boots.len() as u32).to_be_bytes());
    bytes.extend(boots);
    bytes
}
fn kinds(code: &[u8], owner: &str, method: Option<&str>) -> Vec<&'static str> {
    let cf = parse(&class(code)).unwrap();
    let (hits, partial) = references(&cf, owner, method, 10);
    assert!(!partial);
    hits.into_iter().map(|r| r.kind).collect()
}
#[test]
fn class_literal_has_a_use_site() {
    assert_eq!(kinds(&[0x12, 10, 0x57, 0xb1], "Target", None), ["type"]);
}
#[test]
fn literal_method_handle_finds_its_member() {
    assert_eq!(kinds(&[0x12, 14, 0x57, 0xb1], "Target", Some("apply")), ["method-handle"]);
}
#[test]
fn method_type_mentions_object_array_components() {
    assert_eq!(kinds(&[0x12, 22, 0x57, 0xb1], "Target", None), ["method-type"]);
}
#[test]
fn dynamic_call_exposes_factory_dependency() {
    assert_eq!(kinds(&[0xba, 0, 23, 0, 0, 0xb1], "Factory", Some("boot")), ["bootstrap-method"]);
}
#[test]
fn dynamic_call_exposes_static_method_handle_argument() {
    assert_eq!(kinds(&[0xba, 0, 23, 0, 0, 0xb1], "Target", Some("apply")), ["method-handle"]);
}
#[test]
fn dynamic_constant_exposes_bootstrap_dependency() {
    assert_eq!(kinds(&[0x12, 27, 0x57, 0xb1], "Factory", Some("boot")), ["bootstrap-method"]);
}
#[test]
fn unused_bootstrap_does_not_introduce_a_reference() {
    assert!(kinds(&[0xb1], "Factory", None).is_empty());
}
