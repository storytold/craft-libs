use super::*;

fn parser<'a, 'b>(raw: &'a str, budget: &'b mut Budget, keep: &'b dyn Fn() -> bool) -> Parser<'a, 'b> {
    Parser {
        raw,
        at: 0,
        checked_at: 0,
        limits: Limits::default(),
        budget,
        keep,
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
    }
}

#[test]
fn zero_padded_numeric_entities_keep_valid_scalars_in_text_and_attributes() {
    for (digits, expected) in [("65", "A"), ("x41", "A"), ("x1f600", "😀")] {
        let (prefix, number) = digits.strip_prefix('x').map_or(("", digits), |number| ("x", number));
        let entity = format!("&#{prefix}{}{number};", "0".repeat(4096));
        let raw = format!("<markup><span family='{entity}'>{entity}</span></markup>");
        let scan = tokenize(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!(scan.records[0].text, expected);
        assert_eq!(scan.records[0].contexts[1].attributes[0].value, expected);
    }
    for entity in ["&#;", "&#x;", "&#4294967296;", "&#x100000000;", "&#x110000;", "&#xD800;", "&#0;", "&#-1;", "&#x+41;"] {
        let raw = format!("<markup>{entity}</markup>");
        let scan = tokenize(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed, "{entity}");
    }
    let scan = tokenize("<markup>&unavailable;</markup>", Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped.unwrap().reason, StopReason::Unsupported);
}

#[test]
fn long_numeric_entity_polls_inside_decoding_without_bypassing_source_limits() {
    let raw = format!("&#{}65;", "0".repeat(4096));
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 2
    };
    let mut budget = Budget::default();
    // Skip XML pre-validation and cursor traversal so only entity decoding can
    // observe this cancellation; no output has been retained yet.
    let mut parser = parser(&raw, &mut budget, &keep);
    assert!(matches!(parser.decoded(0..raw.len(), false, true), Err(Failure::Cancelled)));
    assert_eq!(calls.get(), 2);
    let raw = format!("<markup>{raw}</markup>");
    let scan = tokenize(&raw, Limits { bytes: raw.len() - 1, ..Limits::default() }, &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped.unwrap().reason, StopReason::PayloadLimit);
    let scan = tokenize(&raw, Limits::default(), &mut Budget::new(16 + 8 * raw.len() - 1, crate::MAX_METADATA_BYTES), &|| true).unwrap();
    assert_eq!(scan.stopped.unwrap().reason, StopReason::WorkLimit);
}

#[test]
fn duplicate_attribute_comparisons_poll_before_the_work_budget_expires() {
    let names = (0..130).map(|index| format!("attribute{index}")).collect::<Vec<_>>();
    let attributes =
        names.iter().map(|name| Attribute { name, value: Cow::Borrowed("value"), kind: AttributeKind::Unhandled, source: 0..0 }).collect::<Vec<_>>();
    let allowance = names.iter().take(65).map(|name| name.len() + "next".len() + 1).sum();
    let mut budget = Budget::new(allowance, 0);
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 2
    };
    let mut cancelled = parser("", &mut budget, &keep);
    assert!(matches!(cancelled.unique_attribute(&attributes, "next"), Err(Failure::Cancelled)));
    assert_eq!(calls.get(), 2);
    assert!(budget.work_remaining() > 0, "cancellation wins before the next work refusal");
    assert!(budget.work_remaining() < allowance, "already compared names stay charged");
    let mut budget = Budget::new(allowance, 0);
    let mut control = parser("", &mut budget, &|| true);
    assert!(matches!(control.unique_attribute(&attributes, "next"), Err(Failure::Stop(StopReason::WorkLimit))));
    let mut budget = Budget::default();
    let mut control = parser("", &mut budget, &|| true);
    assert!(matches!(control.unique_attribute(&attributes, &names[64]), Err(Failure::Stop(StopReason::Malformed))));
}

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
