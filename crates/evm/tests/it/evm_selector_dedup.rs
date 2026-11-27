use craft_evm as evm;

fn prefix() -> Vec<u8> {
    vec![0x5b, 0x00, 0x60, 0x00, 0x35, 0x60, 0xe0, 0x1c]
}

fn comparison(code: &mut Vec<u8>, selector: u32, width: usize, destination: u8) {
    code.push(0x5f + u8::try_from(width).unwrap());
    code.extend_from_slice(&selector.to_be_bytes()[4 - width..]);
    code.extend([0x14, 0x60, destination, 0x57]);
}

#[test]
fn repeated_comparisons_keep_the_first_destination_and_encounter_order() {
    let mut code = prefix();
    comparison(&mut code, 0xfeed0002, 4, 0);
    comparison(&mut code, 0xfeed0001, 4, 0);
    let expected = evm::analyze(&code).unwrap().functions;
    for _ in 0..2000 {
        comparison(&mut code, 0xfeed0001, 4, 1);
        comparison(&mut code, 0xfeed0002, 4, 1);
    }
    assert_eq!(evm::analyze(&code).unwrap().functions, expected);
}

#[test]
fn different_push_widths_share_the_same_numeric_selector() {
    let mut code = prefix();
    for width in 1..=4 {
        comparison(&mut code, 17, width, if width == 1 { 0 } else { 1 });
    }
    let result = evm::analyze(&code).unwrap();
    assert_eq!(result.functions.len(), 1);
    assert_eq!(result.functions[0].selector, "0x00000011");
    assert_eq!(result.functions[0].bytecode_offset, 0);
}

#[test]
fn rejected_candidates_do_not_spend_selector_identity() {
    let mut code = vec![0x5b, 0x00];
    // Short comparisons without selector extraction are not considered functions.
    comparison(&mut code, 17, 1, 1);
    // A selector-shaped immediate without a comparison is not a function either.
    code.extend([0x63, 0, 0, 0, 17, 0x50]);
    comparison(&mut code, 17, 4, 0);
    let result = evm::analyze(&code).unwrap();
    assert_eq!(result.functions.len(), 1);
    assert_eq!(result.functions[0].bytecode_offset, 0);
    assert!(result.functions[0].jumpdest_valid);
}

#[test]
fn selector_cap_counts_unique_matches() {
    let mut code = prefix();
    for selector in 0..4095 {
        comparison(&mut code, selector, 4, 0);
    }
    for _ in 0..10000 {
        comparison(&mut code, 4094, 4, 1);
    }
    let below = evm::analyze(&code).unwrap();
    assert_eq!(below.functions.len(), 4095);
    assert!(!below.warnings.iter().any(|warning| warning.contains("stopped at")));
    comparison(&mut code, 4095, 4, 0);
    comparison(&mut code, 4096, 4, 0);
    let capped = evm::analyze(&code).unwrap();
    assert_eq!(capped.functions.len(), 4096);
    assert_eq!(capped.functions.last().unwrap().selector, "0x00000fff");
    assert!(capped.warnings.iter().any(|warning| warning == "stopped at 4096 selectors"));
}

#[test]
fn repeated_comparisons_keep_fallback_scan_after_the_last_dispatch() {
    for (ending, expected) in [(&[0x5f, 0x5f, 0xfd][..], "absent"), (&[0x00][..], "present")] {
        let mut code = prefix();
        comparison(&mut code, 0xfeed0001, 4, 0);
        comparison(&mut code, 0xfeed0001, 4, 1);
        code.extend_from_slice(ending);
        let report = evm::analyze(&code).unwrap();
        assert_eq!(report.functions.len(), 1);
        assert_eq!(report.functions[0].bytecode_offset, 0);
        assert_eq!(report.fallback, expected);
    }
}

#[test]
fn selector_cap_leaves_fallback_unknown() {
    for ending in [&[0x5f, 0x5f, 0xfd][..], &[0x00][..]] {
        let mut code = prefix();
        for selector in 0..4097 {
            comparison(&mut code, selector, 4, 0);
        }
        code.extend_from_slice(ending);
        let report = evm::analyze(&code).unwrap();
        assert_eq!(report.functions.len(), 4096);
        assert_eq!(report.fallback, "unknown");
    }
}

#[test]
fn payability_only_follows_jumps_to_jumpdest() {
    for (target, expected) in [(&[0x5b, 0x00][..], "payable"), (&[0x00][..], "unknown")] {
        let mut code = prefix();
        let entry = u8::try_from(code.len() + 10).unwrap();
        comparison(&mut code, 0xfeed0001, 4, entry);
        code.push(0x00);
        assert_eq!(code.len(), usize::from(entry));
        code.extend([0x5b, 0x60, entry + 4, 0x56]);
        code.extend_from_slice(target);
        let report = evm::analyze(&code).unwrap();
        assert_eq!(report.functions.len(), 1);
        assert!(report.functions[0].jumpdest_valid);
        assert_eq!(report.functions[0].mutability, expected);
    }
}
