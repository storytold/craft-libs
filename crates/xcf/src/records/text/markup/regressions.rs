use super::*;

fn wrapper_tables(contexts: usize, runs: usize) -> (Vec<Context<'static>>, Vec<Run>) {
    let table = (0..contexts)
        .map(|index| Context {
            parent: match index {
                0 => None,
                1 => Some(0),
                _ => Some(1),
            },
            name: if index < 2 { "markup" } else { "b" },
            role: if index < 2 { Role::Markup } else { Role::Bold },
            attributes: Vec::new(),
            source: if index == 0 { 0..0 } else { 0..8 },
        })
        .collect();
    let runs = (0..runs).map(|_| Run { context: 1, text: 0..1, source: 8..9 }).collect();
    (table, runs)
}

#[test]
fn wrapper_compaction_polls_each_eligibility_and_rebase_walk() {
    // Test the private post-parse boundary directly: no earlier source or notice
    // scan can satisfy these cancellation assertions. The table shape is the
    // same explicit wrapper plus sibling child contexts produced by parsing.
    for (contexts, runs, stop, removed) in [(258, 0, 2, false), (2, 256, 2, false), (258, 0, 6, true), (2, 256, 7, true)] {
        let mut budget = Budget::default();
        let calls = std::cell::Cell::new(0);
        let keep = || {
            calls.set(calls.get() + 1);
            calls.get() < stop
        };
        let mut parser = parser("", &mut budget, &keep);
        (parser.contexts, parser.runs) = wrapper_tables(contexts, runs);
        assert!(matches!(parser.compact_wrapper(), Err(Failure::Cancelled)), "contexts={contexts} runs={runs} stop={stop}");
        assert_eq!(calls.get(), stop);
        assert_eq!(parser.contexts.len(), contexts - usize::from(removed));
        if runs != 0 && removed {
            assert_eq!(parser.runs[0].context, 1, "run rebasing must cancel before publishing a partial walk");
        }
    }
    let mut budget = Budget::default();
    let mut parser = parser("", &mut budget, &|| true);
    (parser.contexts, parser.runs) = wrapper_tables(258, 256);
    parser.compact_wrapper().unwrap_or_else(|_| panic!("valid wrapper"));
    assert_eq!(parser.contexts.len(), 257);
    assert_eq!(parser.contexts[0].parent, None);
    assert!(parser.contexts.iter().skip(1).all(|context| context.parent == Some(0)));
    assert!(parser.runs.iter().all(|run| run.context == 0));
}

#[test]
fn notice_attribute_scan_polls_recognized_fields_before_filtering() {
    for raw in ["<markup font='Example' weight='bold'/>", "<markup unknown='value'/>"] {
        let mut budget = Budget::default();
        let mut markup = parser(raw, &mut budget, &|| true).parse().unwrap_or_else(|_| panic!("valid fixture"));
        assert!(!markup.contexts[0].attributes.is_empty());
        let calls = std::cell::Cell::new(0);
        let keep = || {
            calls.set(calls.get() + 1);
            calls.get() < 2
        };
        assert!(matches!(record_notices(&mut markup, Limits::default().notices, &mut budget, &keep), Err(Failure::Cancelled)));
        assert_eq!(calls.get(), 2, "context then attribute, including recognized attributes");
        assert!(markup.notices.is_empty());
    }
}

#[test]
fn notice_attribute_scan_cancels_after_a_bounded_prefix_and_preserves_notice_caps() {
    let attributes = (0..130).map(|index| format!(" x{index}='value'")).collect::<String>();
    let raw = format!("<markup{attributes}/>");
    let mut budget = Budget::default();
    let mut markup = parser(&raw, &mut budget, &|| true).parse().unwrap_or_else(|_| panic!("valid fixture"));
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 3
    };
    assert!(matches!(record_notices(&mut markup, 128, &mut budget, &keep), Err(Failure::Cancelled)));
    assert_eq!(calls.get(), 3);
    assert_eq!(markup.notices.len(), 64, "cancellation is observed before visiting the remaining attributes");
    let control = tokenize(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(control.stopped, None);
    assert_eq!(control.records[0].notices.len(), 128);
    assert!(control.records[0].notices_truncated);
}

#[test]
fn an_explicit_markup_element_can_have_fragment_siblings_and_trailing_text() {
    for (raw, expected) in
        [("<markup>A</markup><i>B</i>", "AB"), ("<markup>A</markup> tail", "A tail"), (" before <markup>A</markup> after ", " before A after ")]
    {
        let scan = tokenize(raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None, "{raw}");
        let markup = &scan.records[0];
        assert_eq!(markup.text, expected);
        assert_eq!(markup.contexts[0].source, 0..0);
        if raw.ends_with("</i>") {
            let last = markup.runs.last().unwrap();
            assert_eq!(&markup.text[last.text.clone()], "B");
            assert_eq!(markup.inherited(last.context).map(|context| context.role).collect::<Vec<_>>(), [Role::Italic, Role::Markup]);
        }
    }
    let scan = tokenize("<markup><b>A</b></markup>", Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.records[0].contexts.len(), 2);
    assert_eq!(scan.records[0].contexts[0].source, 0..8, "single explicit wrappers keep their context identities");
}

#[test]
fn depth_64_inheritance_reaches_the_root_and_still_bounds_cycles() {
    for wrapper in [false, true] {
        let nested = if wrapper { 63 } else { 64 };
        let body = format!("{}x{}", "<b>".repeat(nested), "</b>".repeat(nested));
        let raw = if wrapper { format!("<markup>{body}</markup>") } else { body };
        let scan = tokenize(&raw, Limits { depth: 64, ..Limits::default() }, &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let mut markup = scan.records.into_iter().next().unwrap();
        let deepest = markup.runs[0].context;
        let chain = markup.inherited(deepest).collect::<Vec<_>>();
        assert_eq!(chain.len(), if wrapper { 64 } else { 65 });
        assert_eq!(chain.last().unwrap().role, Role::Markup);
        assert_eq!(chain.last().unwrap().source.is_empty(), !wrapper);
        markup.contexts[0].parent = Some(deepest);
        assert_eq!(markup.inherited(deepest).count(), 65, "public mutable contexts cannot create an unbounded walk");
    }
    let raw = format!("{}x{}", "<b>".repeat(65), "</b>".repeat(65));
    let scan = tokenize(&raw, Limits { depth: usize::MAX, ..Limits::default() }, &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped.unwrap().reason, StopReason::RecordLimit);
}

#[test]
fn consecutive_short_comments_poll_inside_the_actual_parser() {
    let raw = format!("{}</markup>", "<!---->".repeat(8192));
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 2
    };
    // Bypass source prevalidation. Without comment-loop polling this reaches
    // the malformed close before any later text/compaction cancellation check.
    assert!(matches!(parser(&raw, &mut Budget::default(), &keep).parse(), Err(Failure::Cancelled)));
    assert_eq!(calls.get(), 2);
    assert!(matches!(parser(&raw, &mut Budget::default(), &|| true).parse(), Err(Failure::Stop(StopReason::Malformed))));
    let raw = format!("{}<markup>A</markup>", "<!---->".repeat(8192));
    let scan = tokenize(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped, None);
    assert_eq!(scan.records[0].text, "A");
}

#[test]
fn wrapperless_fragments_keep_text_inheritance_and_original_ranges() {
    for (raw, expected) in [("<b>A</b> &amp; <i>B</i>", "A & B"), ("<b>A</b><i>B</i>", "AB"), ("plain &lt;text&gt;", "plain <text>"), ("", "")] {
        let scan = tokenize(raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None, "{raw}");
        let markup = &scan.records[0];
        assert_eq!(markup.text, expected);
        assert_eq!(markup.raw, raw);
        assert_eq!(markup.contexts[0].role, Role::Markup);
        assert_eq!(markup.contexts[0].source, 0..0);
        for run in &markup.runs {
            assert!(raw.get(run.source.clone()).is_some());
            let roles = markup.inherited(run.context).map(|context| context.role).collect::<Vec<_>>();
            assert_eq!(roles.last(), Some(&Role::Markup));
        }
        if raw.starts_with("<b>") {
            assert_eq!(markup.inherited(markup.runs[0].context).map(|context| context.role).collect::<Vec<_>>(), [Role::Bold, Role::Markup]);
            assert_eq!(&raw[markup.runs[0].source.clone()], "A");
            assert_eq!(markup.contexts[1].source, 0..3);
        }
    }
    let raw = "<!-- comment --><markup><b>A</b></markup>";
    let scan = tokenize(raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped, None);
    let markup = &scan.records[0];
    assert_ne!(markup.contexts[0].source, 0..0, "explicit wrappers keep their existing context identities");
    assert_eq!(markup.contexts.len(), 2);
    assert_eq!(markup.text, "A");
}

#[test]
fn fragments_keep_structural_resource_and_xml_dialect_guards() {
    for (raw, reason) in [
        ("<b>open", StopReason::Malformed),
        ("text</markup>", StopReason::Malformed),
        ("<b>A</i>", StopReason::Malformed),
        ("<!DOCTYPE b><b>A</b>", StopReason::Unsupported),
        ("<?process?><b>A</b>", StopReason::Unsupported),
        ("<x:b>A</x:b>", StopReason::Unsupported),
    ] {
        let mut budget = Budget::default();
        let retained = budget.retained_remaining();
        let scan = tokenize(raw, Limits::default(), &mut budget, &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, reason, "{raw}");
        assert_eq!(budget.retained_remaining(), retained);
    }
    let single = tokenize("<b>A</b>", Limits { depth: 1, ..Limits::default() }, &mut Budget::default(), &|| true).unwrap();
    assert_eq!(single.stopped, None);
    let nested = tokenize("<b><i>A</i></b>", Limits { depth: 1, ..Limits::default() }, &mut Budget::default(), &|| true).unwrap();
    assert_eq!(nested.stopped.unwrap().reason, StopReason::RecordLimit);
    let prefixed = format!("<!--{}--><markup/>", "x".repeat(4096));
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 2
    };
    assert!(matches!(parser(&prefixed, &mut Budget::default(), &keep).parse(), Err(Failure::Cancelled)));
    assert_eq!(calls.get(), 2);
}

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
        implicit_root: false,
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
    let mut decoder = parser(&raw, &mut budget, &keep);
    assert!(matches!(decoder.decoded(0..raw.len(), false, true), Err(Failure::Cancelled)));
    assert_eq!(calls.get(), 2);
    // The delimiter scan polls at 0..=4096 (17 calls); allow the first digit
    // poll too, then cancel at digit 256. Neither earlier walk can satisfy this.
    calls.set(0);
    let keep_digits = || {
        calls.set(calls.get() + 1);
        calls.get() < 19
    };
    let mut budget = Budget::default();
    let retained = budget.retained_remaining();
    assert!(matches!(parser(&raw, &mut budget, &keep_digits).decoded(0..raw.len(), false, true), Err(Failure::Cancelled)));
    assert_eq!(calls.get(), 19);
    assert_eq!(budget.retained_remaining(), retained, "cancel before producing the scalar");
    let raw = format!("<markup>{raw}</markup>");
    let scan = tokenize(&raw, Limits { bytes: raw.len() - 1, ..Limits::default() }, &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped.unwrap().reason, StopReason::PayloadLimit);
    let scan = tokenize(&raw, Limits::default(), &mut Budget::new(16 + 8 * raw.len() - 1, crate::MAX_METADATA_BYTES), &|| true).unwrap();
    assert_eq!(scan.stopped.unwrap().reason, StopReason::WorkLimit);
}

#[test]
fn long_comment_body_polls_before_reaching_its_terminator() {
    let raw = format!("<!--{}--><markup/>", "x".repeat(4096));
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 2
    };
    let mut budget = Budget::default();
    let mut reader = parser(&raw, &mut budget, &keep);
    reader.at = 4;
    reader.checked_at = 4;
    assert!(matches!(reader.through("-->"), Err(Failure::Cancelled)));
    assert_eq!(calls.get(), 2);
    assert!(reader.at < raw.find("-->").unwrap());
    assert_eq!(reader.at, 516);
    let mut budget = Budget::default();
    let mut reader = parser(&raw, &mut budget, &|| true);
    reader.at = 4;
    assert_eq!(reader.through("-->").unwrap_or_else(|_| panic!("valid comment")), 4..4100);
    assert_eq!(reader.at, 4103);
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
        assert_eq!(markup.contexts.len(), 2);
        assert_eq!(markup.contexts[1].name, "odd");
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
        implicit_root: false,
    };
    assert!(parser.opening().is_ok());
    assert_eq!(parser.contexts.len(), 1);
    assert!(parser.budget.retained_remaining() < before);
    assert!(matches!(parser.opening(), Err(Failure::Cancelled)));
    assert!(parser.at < raw.find("'>").unwrap(), "cancellation must precede attribute decoding");
    assert_eq!(parser.contexts.len(), 1);
    assert_eq!(calls.get(), 2);
}
