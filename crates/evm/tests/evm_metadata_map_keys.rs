use craft_anything_formats::evm::analyze;

fn contract_with_trailer(cbor: &[u8]) -> Vec<u8> {
    let mut code = vec![0x5b, 0x00];
    code.extend_from_slice(cbor);
    code.extend_from_slice(&u16::try_from(cbor.len()).unwrap().to_be_bytes());
    code
}

fn assert_unrecognized(cbor: &[u8]) {
    let code = contract_with_trailer(cbor);
    let result = analyze(&code).unwrap();
    assert!(result.metadata.is_none(), "ambiguous map must remain bytecode: {cbor:02x?}");
    assert_eq!(result.analysed_size, code.len());
}

#[test]
fn duplicate_compiler_keys_do_not_replace_the_reported_version() {
    assert_unrecognized(b"\xa2\x64solc\x43\x00\x08\x13\x64solc\x43\x00\x08\x1a");
}

#[test]
fn identical_duplicate_keys_are_also_ambiguous() {
    assert_unrecognized(b"\xa2\x64solc\x43\x00\x08\x13\x64solc\x43\x00\x08\x13");
}

#[test]
fn numeric_keys_cannot_collide_with_text_keys() {
    assert_unrecognized(b"\xa2\x01\x00\x61\x31\x01");
}

#[test]
fn metadata_map_keys_must_be_encoded_as_text() {
    for key in [vec![0x01], vec![0x41, b'x'], vec![0x83, 0, 1, 2], vec![0xf5], vec![0xf6], vec![0xa0]] {
        let mut cbor = vec![0xa1];
        cbor.extend(key);
        cbor.push(0);
        assert_unrecognized(&cbor);
    }
}

#[test]
fn nested_maps_do_not_silently_overwrite_keys() {
    assert_unrecognized(b"\xa1\x65extra\xa2\x61x\x00\x61x\x01");
    assert_unrecognized(b"\xa1\x65extra\xa1\x01\x00");
}

#[test]
fn ordinary_compiler_metadata_and_unique_nested_text_keys_still_decode() {
    let code = contract_with_trailer(b"\xa2\x64solc\x43\x00\x08\x13\x65extra\xa2\x61x\x00\x61y\x01");
    let result = analyze(&code).unwrap();
    assert_eq!(result.analysed_size, 2);
    let metadata = result.metadata.unwrap();
    assert_eq!(metadata.compiler.as_deref(), Some("solc 0.8.19"));
    assert_eq!(metadata.entries["extra"]["x"], 0);
    assert_eq!(metadata.entries["extra"]["y"], 1);
    assert_eq!(metadata.length, code.len() - 2);
}

#[test]
fn equal_keys_in_separate_maps_are_independent() {
    let code = contract_with_trailer(b"\xa2\x61a\xa1\x61x\x00\x61b\xa1\x61x\x01");
    let result = analyze(&code).unwrap();
    assert_eq!(result.analysed_size, 2);
    let entries = result.metadata.unwrap().entries;
    assert_eq!(entries["a"]["x"], 0);
    assert_eq!(entries["b"]["x"], 1);
}
