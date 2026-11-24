use super::*;
use crate::records::{self, Limits as RecordLimits};

fn property(kind: u32, data: &[u8]) -> Vec<u8> {
    [kind.to_be_bytes().as_slice(), (data.len() as u32).to_be_bytes().as_slice(), data].concat()
}
fn scan(bytes: &[u8]) -> Scan<Property<'_>> {
    records::properties(bytes, 0, RecordLimits::default(), &mut Budget::default(), &|| true).unwrap()
}
fn rgb(values: [f32; 3]) -> Vec<u8> {
    values.into_iter().flat_map(f32::to_be_bytes).collect()
}

#[test]
fn last_valid_channel_evidence_preserves_float_precedence_and_source_offsets() {
    for float_first in [false, true] {
        let byte = [property(16, &[255, 0, 0]), property(6, &127u32.to_be_bytes())].concat();
        let float = [property(38, &rgb([0.25, 0.5, 0.75])), property(33, &0.375f32.to_be_bytes())].concat();
        let bytes = [
            property(3, &[]),
            property(8, &0u32.to_be_bytes()),
            property(8, &7u32.to_be_bytes()),
            property(14, &0u32.to_be_bytes()),
            property(14, &1u32.to_be_bytes()),
            if float_first { [float, byte].concat() } else { [byte, float].concat() },
            property(0, &[]),
        ]
        .concat();
        let raw = scan(&bytes);
        let value = channel(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(value.complete());
        assert_eq!(value.selected.unwrap().offset, 0);
        assert_eq!(value.visible.unwrap().value, 7);
        assert_eq!(value.indicates.unwrap().value, ChannelColorIndicates::MaskedAreas);
        assert_eq!(value.byte_color.unwrap().value, [255, 0, 0]);
        assert_eq!(value.float_color.unwrap().value, [0.25, 0.5, 0.75]);
        assert_eq!(value.byte_opacity.unwrap().value, 127);
        assert_eq!(value.float_opacity.unwrap().value, 0.375);
        assert!(value.issues.is_empty());
        assert!(raw.records.iter().any(|record| record.offset == value.float_color.unwrap().offset && record.kind == 38));
    }
}

#[test]
fn invalid_channel_optionals_do_not_replace_valid_evidence_or_invent_defaults() {
    let bytes = [
        property(16, &[1, 2, 3]),
        property(16, &[1, 2]),
        property(38, &rgb([f32::NAN, 0., 0.])),
        property(33, &0.5f32.to_be_bytes()),
        property(33, &f32::INFINITY.to_be_bytes()),
        property(6, &256u32.to_be_bytes()),
        property(14, &2u32.to_be_bytes()),
        property(3, &[1]),
        property(8, &[1]),
        property(0, &[]),
    ]
    .concat();
    let raw = scan(&bytes);
    let value = channel(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(value.complete());
    assert_eq!(value.byte_color.unwrap().value, [1, 2, 3]);
    assert_eq!(value.float_opacity.unwrap().value, 0.5);
    assert!(value.float_color.is_none() && value.byte_opacity.is_none() && value.indicates.is_none());
    assert!(value.selected.is_none() && value.visible.is_none());
    assert_eq!(value.issues.len(), 7);
}

#[test]
fn incomplete_scans_limits_and_cancellation_remain_explicit() {
    let bytes = [property(8, &1u32.to_be_bytes()), property(3, &[]), property(0, &[])].concat();
    let limited = records::properties(&bytes, 0, RecordLimits { records: 1, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
    let value = channel(&limited, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(!value.complete());
    assert!(value.selected.is_none());
    assert_eq!(value.source_stopped.unwrap().reason, StopReason::RecordLimit);
    let raw = scan(&bytes);
    let value = channel(&raw, Limits::default(), &mut Budget::new(0, 4096), &|| true).unwrap();
    assert!(!value.complete());
    assert_eq!(value.stopped.unwrap().reason, StopReason::WorkLimit);
    assert!(matches!(channel(&raw, Limits::default(), &mut Budget::default(), &|| false), Err(Error::Cancelled)));
    let calls = std::cell::Cell::new(0);
    let keep_going = || {
        calls.set(calls.get() + 1);
        calls.get() < 4
    };
    assert!(matches!(channel(&raw, Limits::default(), &mut Budget::default(), &keep_going), Err(Error::Cancelled)));
}

#[test]
fn saved_channel_property_access_is_bounded_and_does_not_decode_pixels() {
    let mut image = crate::tests::TestImage::rgb8(1, 1);
    image.channels.push(("Overlay", vec![1], false));
    let bytes = image.write();
    let xcf = crate::Xcf::parse(&bytes).unwrap();
    let mut budget = Budget::default();
    let raw = xcf.channel_properties(0, RecordLimits::default(), &mut budget, &|| true).unwrap();
    assert!(raw.stopped.is_none());
    let value = channel(&raw, Limits::default(), &mut budget, &|| true).unwrap();
    assert!(value.complete());
    assert!(matches!(xcf.channel_properties(1, RecordLimits::default(), &mut budget, &|| true), Err(Error::Invalid(_))));
    assert!(matches!(xcf.channel_properties(usize::MAX, RecordLimits::default(), &mut budget, &|| false), Err(Error::Cancelled)));
    let raw = xcf.channel_properties(0, RecordLimits { string_bytes: 1, ..Default::default() }, &mut budget, &|| true).unwrap();
    assert_eq!(raw.stopped.unwrap().reason, StopReason::StringLimit);
    // A complete channel header does not need a hierarchy: this intentionally
    // invalid pixel pointer is left unread by the source metadata path.
    let mut source = crate::tests::W::new(26);
    source.out.clear();
    source.u32(1);
    source.u32(1);
    source.string("Saved");
    source.prop(3, &[]);
    source.end();
    source.ptr(usize::MAX);
    let raw = records::channel_properties(&source.out, 0, RecordLimits::default(), &mut budget, &|| true).unwrap();
    assert!(raw.stopped.is_none());
    assert!(channel(&raw, Limits::default(), &mut budget, &|| true).unwrap().selected.is_some());
}

#[test]
fn cancelled_channel_metadata_refunds_unreturned_issues_but_keeps_work() {
    let bytes = [property(16, &[1, 2]), property(0, &[])].concat();
    let raw = scan(&bytes);
    let calls = std::cell::Cell::new(0);
    let probe = channel(&raw, Limits::default(), &mut Budget::default(), &|| {
        calls.set(calls.get() + 1);
        true
    })
    .unwrap();
    assert_eq!(probe.issues.len(), 1);
    let final_check = calls.get();
    calls.set(0);
    let retained = 2 * std::mem::size_of::<Issue>();
    let mut budget = Budget::new(1024, retained);
    let result = channel(&raw, Limits::default(), &mut budget, &|| {
        calls.set(calls.get() + 1);
        calls.get() < final_check
    });
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(budget.retained_remaining(), retained);
    assert!(budget.work_remaining() < 1024);
    let work = budget.work_remaining();
    let retry = channel(&raw, Limits::default(), &mut budget, &|| true).unwrap();
    assert!(retry.complete());
    assert_eq!(retry.issues.len(), 1);
    assert_eq!(budget.retained_remaining(), 0);
    assert!(budget.work_remaining() < work);
    let full = channel(&raw, Limits::default(), &mut budget, &|| true).unwrap();
    assert_eq!(full.stopped.unwrap().reason, StopReason::RetainedLimit);
}

#[test]
fn borrowed_channel_properties_keep_source_stops_without_rescanning() {
    let bytes = [property(16, &[1, 2, 3]), property(0, &[])].concat();
    let mut raw = scan(&bytes);
    let source_stop = Diagnostic { offset: 99, ordinal: 4, reason: StopReason::Malformed };
    raw.stopped = Some(source_stop);
    let mut borrowed_budget = Budget::default();
    let mut wrapper_budget = Budget::default();
    let borrowed = channel_from_properties(&raw.records, raw.stopped, Limits::default(), &mut borrowed_budget, &|| true).unwrap();
    let wrapped = channel(&raw, Limits::default(), &mut wrapper_budget, &|| true).unwrap();
    assert!(!borrowed.complete());
    assert_eq!(borrowed.source_stopped, Some(source_stop));
    assert_eq!(borrowed.byte_color, wrapped.byte_color);
    assert_eq!(borrowed.byte_color.unwrap().value, [1, 2, 3]);
    assert_eq!(borrowed.stopped, wrapped.stopped);
    assert_eq!(borrowed.issues, wrapped.issues);
    assert_eq!(borrowed_budget.work_remaining(), wrapper_budget.work_remaining());
    assert_eq!(borrowed_budget.retained_remaining(), wrapper_budget.retained_remaining());
    assert!(matches!(channel_from_properties(&raw.records, raw.stopped, Limits::default(), &mut borrowed_budget, &|| false), Err(Error::Cancelled)));
}
