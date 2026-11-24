use super::*;

#[test]
fn optional_notices_do_not_displace_required_markup_storage() {
    let raw = "<odd/>";
    let mut measured = Budget::default();
    let initial = measured.retained_remaining();
    let control = tokenize(raw, Limits { notices: 0, ..Limits::default() }, &mut measured, &|| true).unwrap();
    assert_eq!(control.stopped, None);
    let required = initial - measured.retained_remaining();
    for notices in [0, 1, 128] {
        let mut budget = Budget::new(4096, required);
        let scan = tokenize(raw, Limits { notices, ..Limits::default() }, &mut budget, &|| true).unwrap();
        assert_eq!(scan.stopped, None, "notice allowance {notices}");
        let markup = &scan.records[0];
        assert_eq!(markup.contexts.len(), 1);
        assert_eq!(markup.contexts[0].name, "odd");
        assert!(markup.has_unhandled && markup.notices_truncated);
        assert!(markup.notices.is_empty());
        assert_eq!(budget.retained_remaining(), 0);
    }
}

#[test]
fn cancellation_after_context_allocation_refunds_retention_not_work() {
    let raw = "<markup>x</markup>";
    let mut budget = Budget::default();
    let retained = budget.retained_remaining();
    let work = budget.work_remaining();
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 3 // admission, root creation, then first text node.
    };
    assert!(matches!(tokenize(raw, Limits::default(), &mut budget, &keep), Err(Error::Cancelled)));
    assert_eq!(calls.get(), 3);
    assert_eq!(budget.retained_remaining(), retained);
    assert!(budget.work_remaining() < work);
    let retry = tokenize(raw, Limits::default(), &mut budget, &|| true).unwrap();
    assert_eq!(retry.stopped, None);
    assert_eq!(retry.records[0].text, "x");
}

#[test]
fn cancellation_inside_attribute_scan_follows_an_allocated_root() {
    // Direct parser entry separates this check from tokenize's preliminary XML
    // character validation; removing attribute polling must make it fail.
    let raw = format!("<markup><span unknown='{}'>text</span></markup>", "x".repeat(4096));
    let mut budget = Budget::default();
    let before = budget.retained_remaining();
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 2
    };
    let mut parser = Parser {
        raw: &raw,
        at: 0,
        checked_at: 0,
        limits: Limits::default(),
        budget: &mut budget,
        keep: &keep,
        output: Vec::new(),
        runs: Vec::new(),
        contexts: Vec::new(),
        stack: Vec::new(),
        stack_charge: 0,
        notices: Vec::new(),
        has_unhandled: false,
        notices_truncated: false,
        nodes: 0,
        attributes: 0,
        root_seen: false,
        root_closed: false,
    };
    assert!(parser.opening().is_ok());
    assert_eq!(parser.contexts.len(), 1);
    assert!(parser.budget.retained_remaining() < before);
    assert!(matches!(parser.opening(), Err(Failure::Cancelled)));
    assert!(parser.at < raw.find("'>").unwrap(), "cancellation must precede attribute decoding");
    assert_eq!(parser.contexts.len(), 1);
    assert_eq!(calls.get(), 2);
}
