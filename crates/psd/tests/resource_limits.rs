use craft_psd::descriptor::{Descriptor, ObjectArray, ParseLimits, Value};
use craft_psd::float::FloatImage;
use craft_psd::smart::SmartObject;

#[test]
fn descriptor_limits_apply_at_boundary_and_through_every_nested_container() {
    let limits = ParseLimits { max_descriptor_items: 1, max_list_items: 1 };
    let one = Descriptor::new("test").with("one", Value::Boolean(true));
    let two = one.clone().with("two", Value::Boolean(false));
    assert!(Descriptor::parse_prefix_with_limits(&one.to_bytes(), limits).is_ok());
    assert!(Descriptor::parse_prefix_with_limits(&two.to_bytes(), limits).is_err());
    assert!(Descriptor::parse_prefix(&two.to_bytes()).is_ok());
    for nested in [
        Value::Descriptor(two.clone()),
        Value::GlobalObject(two.clone()),
        Value::ObjectArray(ObjectArray { prefix: 0, body: two.clone() }),
        Value::List(vec![Value::Descriptor(two.clone())]),
    ] {
        let bytes = Descriptor::new("test").with("nest", nested).to_bytes();
        assert!(Descriptor::parse_prefix_with_limits(&bytes, limits).is_err());
        assert!(Descriptor::parse_prefix(&bytes).is_ok());
    }
    for n in [0, 1, 2] {
        let d = Descriptor::new("test").with("list", Value::List(vec![Value::Boolean(true); n]));
        assert_eq!(Descriptor::parse_prefix_with_limits(&d.to_bytes(), limits).is_ok(), n <= 1);
    }
    let nested_list = Descriptor::new("test").with("list", Value::List(vec![Value::List(vec![Value::Boolean(true); 2])]));
    assert!(Descriptor::parse_prefix_with_limits(&nested_list.to_bytes(), limits).is_err());
    let mut bytes = one.to_bytes();
    let used = bytes.len();
    bytes.extend_from_slice(b"tail");
    assert_eq!(Descriptor::parse_prefix_with_limits(&bytes, limits).unwrap().1, used);
    let zero = ParseLimits { max_descriptor_items: 0, max_list_items: 0 };
    assert!(Descriptor::parse_prefix_with_limits(&Descriptor::new("test").to_bytes(), zero).is_ok());
    assert!(Descriptor::parse_prefix_with_limits(&one.to_bytes(), zero).is_err());
}

#[test]
fn count_limits_reject_before_reading_declared_elements() {
    let limits = ParseLimits { max_descriptor_items: 1, max_list_items: 1 };
    let mut bytes = Descriptor::new("test").to_bytes();
    let end = bytes.len();
    bytes[end - 4..].copy_from_slice(&2u32.to_be_bytes());
    assert!(matches!(Descriptor::parse_prefix_with_limits(&bytes, limits), Err(craft_psd::PsdError::LimitExceeded(_))));
    let mut bytes = Descriptor::new("test").with("list", Value::List(vec![])).to_bytes();
    let end = bytes.len();
    bytes[end - 4..].copy_from_slice(&2u32.to_be_bytes());
    assert!(matches!(Descriptor::parse_prefix_with_limits(&bytes, limits), Err(craft_psd::PsdError::LimitExceeded(_))));
}

#[test]
fn placed_layer_data_honors_descriptor_limits() {
    let d = Descriptor::new("test").with("one", Value::Boolean(true)).with("two", Value::Boolean(false));
    let mut bytes = b"soLD".to_vec();
    bytes.extend_from_slice(&4u32.to_be_bytes());
    bytes.extend_from_slice(&16u32.to_be_bytes());
    bytes.extend_from_slice(&d.to_bytes());
    assert!(SmartObject::from_placed_layer_data(&bytes).is_ok());
    assert!(SmartObject::from_placed_layer_data_with_limits(&bytes, ParseLimits { max_descriptor_items: 1, max_list_items: 1 }).is_err());
}

#[test]
fn borrowed_pixels_match_owned_render_and_preserve_short_slice_behavior() {
    let smart = SmartObject { uuid: String::new(), quad: [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]], size: None, warp: None };
    for data in [vec![], vec![[1.0, 0.5, 0.25, 0.5]], vec![[1.0, 0.5, 0.25, 0.5]; 4]] {
        let image = FloatImage { width: 2, height: 2, data };
        let owned = smart.render_placed(&image, 1.0, Some((2, 2)));
        let borrowed = smart.render_placed_pixels(2, 2, &image.data, 1.0, Some((2, 2)));
        assert!(owned.is_some());
        assert_eq!(owned, borrowed);
    }
    assert!(smart.render_placed_pixels(0, 2, &[], 1.0, None).is_none());
    assert!(smart.render_placed_pixels(u32::MAX, u32::MAX, &[], 1.0, Some((u32::MAX, u32::MAX))).is_none());
    // A short source remains safely transparent even with enormous declared dimensions.
    assert!(smart.render_placed_pixels(u32::MAX, u32::MAX, &[], 1.0, Some((1, 1))).is_some());
}
