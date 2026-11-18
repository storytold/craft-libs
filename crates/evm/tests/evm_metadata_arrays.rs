use craft_evm as evm;
use serde_json::json;

/// Unrelated three-integer arrays stay arrays while compiler versions stay readable.
#[test]
fn cbor_arrays_are_not_all_compiler_versions() {
    // {"sizes": [1, 2, 3], "vyper": [0, 3, 10]}
    for (cbor, compiler) in [
        (b"\xa2\x65sizes\x83\x01\x02\x03\x65vyper\x83\x00\x03\x0a".as_slice(), "vyper 0.3.10"),
        (b"\xa2\x65sizes\x83\x01\x02\x03\x64solc\x83\x00\x08\x13".as_slice(), "solc 0.8.19"),
    ] {
        let mut code = vec![0x00];
        code.extend(cbor);
        code.extend((cbor.len() as u16).to_be_bytes());
        let result = evm::analyze(&code).unwrap();
        let meta = result.metadata.unwrap();
        assert_eq!(meta.entries["sizes"], json!([1, 2, 3]));
        assert_eq!(meta.compiler.as_deref(), Some(compiler));
        assert_eq!(result.analysed_size, 1);
    }
}

/// A non-version Vyper array stays typed, and a truncated array stays bytecode.
#[test]
fn invalid_version_shapes_are_not_invented_or_silently_consumed() {
    let cbor = b"\xa1\x65vyper\x82\x00\x03";
    let mut code = vec![0x00];
    code.extend(cbor);
    code.extend((cbor.len() as u16).to_be_bytes());
    let meta = evm::analyze(&code).unwrap().metadata.unwrap();
    assert_eq!(meta.compiler, None);
    assert_eq!(meta.entries["vyper"], json!([0, 3]));

    let mut truncated = code;
    truncated[8] = 0x83; // Declare three items but keep only two.
    let result = evm::analyze(&truncated).unwrap();
    assert!(result.metadata.is_none());
    assert_eq!(result.analysed_size, truncated.len());
}
