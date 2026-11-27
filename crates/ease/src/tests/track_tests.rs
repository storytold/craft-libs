use crate::*;

fn key<T>(time: f64, value: T, ease: Easing) -> Key<T> {
    Key { time, value, ease }
}

#[test]
fn a_track_eases_between_its_keys_and_holds_outside_them() {
    let t = Track::from_keys(vec![
        key(2.0, 10.0f64, Easing::QuadIn),
        key(0.0, 0.0, Easing::Linear),
        key(4.0, 0.0, Easing::Hold),
        key(6.0, 50.0, Easing::Linear),
    ])
    .unwrap();
    assert_eq!(t.keys().iter().map(|k| k.time).collect::<Vec<_>>(), [0.0, 2.0, 4.0, 6.0]);
    assert_eq!(t.sample(-1.0), Ok(0.0));
    assert_eq!(t.sample(1.0), Ok(5.0));
    assert_eq!(t.sample(2.0), Ok(10.0));
    // Quad in: a quarter of the way at half time.
    assert_eq!(t.sample(3.0), Ok(7.5));
    // Hold keeps the value until the next key.
    assert_eq!(t.sample(4.0), Ok(0.0));
    assert_eq!(t.sample(5.999), Ok(0.0));
    assert_eq!(t.sample(6.0), Ok(50.0));
    assert_eq!(t.sample(1e300), Ok(50.0));
}

#[test]
fn an_overshooting_ease_carries_the_value_past_its_keys() {
    let t = Track::from_keys(vec![key(0.0, 0.0f64, Easing::BackOut), key(1.0, 100.0, Easing::Linear)]).unwrap();
    let peak = (1..100).map(|i| t.sample(i as f64 / 100.0).unwrap()).fold(f64::MIN, f64::max);
    assert!(peak > 105.0 && peak < 115.0, "{peak}");
}

#[test]
fn tracks_hold_any_animatable_value() {
    let p = Track::from_keys(vec![key(0.0, Vec2::new(0.0, 0.0), Easing::Linear), key(1.0, Vec2::new(10.0, -20.0), Easing::Linear)]).unwrap();
    assert_eq!(p.sample(0.5), Ok(Vec2::new(5.0, -10.0)));
    let c = Track::from_keys(vec![key(0.0, [0.0f32, 1.0, 0.5, 1.0], Easing::Linear), key(2.0, [1.0, 0.0, 0.5, 0.0], Easing::Linear)]).unwrap();
    assert_eq!(c.sample(1.0), Ok([0.5, 0.5, 0.5, 0.5]));
    let s = Track::from_keys(vec![key(0.0, 1.0f32, Easing::Linear), key(1.0, 3.0, Easing::Linear)]).unwrap();
    assert_eq!(s.sample(0.25), Ok(1.5));
    let d = Track::from_keys(vec![key(0.0, [0.0f64; 3], Easing::Linear), key(1.0, [3.0, 6.0, 9.0], Easing::Linear)]).unwrap();
    assert_eq!(d.sample(1.0 / 3.0).map(|v| v.map(|x| (x * 1e9).round() / 1e9)), Ok([1.0, 2.0, 3.0]));
}

#[test]
fn bad_keys_and_times_are_refused() {
    assert_eq!(Track::<f64>::new().sample(0.0), Err(EaseError::Empty));
    let t = Track::from_keys(vec![key(0.0, 1.0f64, Easing::Linear)]).unwrap();
    assert_eq!(t.sample(f64::NAN), Err(EaseError::NotFinite("time")));
    assert_eq!(t.sample(f64::INFINITY), Err(EaseError::NotFinite("time")));
    assert_eq!(t.sample(5.0), Ok(1.0));
    assert_eq!(Track::from_keys(vec![key(f64::NAN, 1.0f64, Easing::Linear)]), Err(EaseError::NotFinite("time")));
    assert_eq!(Track::from_keys(vec![key(0.0, f64::NAN, Easing::Linear)]), Err(EaseError::NotFinite("value")));
    assert_eq!(Track::from_keys(vec![key(0.0, Vec2::new(0.0, f64::INFINITY), Easing::Linear)]), Err(EaseError::NotFinite("value")));
    assert_eq!(Track::from_keys(vec![key(1.0, 1.0f64, Easing::Linear), key(1.0, 2.0, Easing::Linear)]), Err(EaseError::SameTime(1.0)));
    // −0 and +0 are one time.
    assert_eq!(Track::from_keys(vec![key(-0.0, 1.0f64, Easing::Linear), key(0.0, 2.0, Easing::Linear)]), Err(EaseError::SameTime(0.0)));
    let mut z = Track::from_keys(vec![key(-0.0, 1.0f64, Easing::Linear), key(1.0, 2.0, Easing::Linear)]).unwrap();
    assert_eq!(z.key_at(0.0), Some(0));
    assert_eq!(z.key_at(-0.0), Some(0));
    assert_eq!(z.insert(0.0, 5.0), Ok(0));
    assert_eq!(z.len(), 2);
    let too_many: Vec<Key<f32>> = (0..=MAX_TRACK_KEYS).map(|i| key(i as f64, 0.0, Easing::Linear)).collect();
    assert_eq!(Track::from_keys(too_many), Err(EaseError::Full(MAX_TRACK_KEYS)));
}

#[test]
fn inserting_splits_a_segment_with_its_ease() {
    let mut t = Track::from_keys(vec![key(0.0, 0.0f64, Easing::CubicIn), key(2.0, 8.0, Easing::Linear)]).unwrap();
    assert_eq!(t.insert(1.0, 1.0), Ok(1));
    assert_eq!(t.keys()[1].ease, Easing::CubicIn);
    // Setting an existing key keeps its ease.
    t.set_ease(1, Easing::Hold).unwrap();
    assert_eq!(t.insert(1.0, 3.0), Ok(1));
    assert_eq!((t.keys()[1].value, &t.keys()[1].ease), (3.0, &Easing::Hold));
    assert_eq!(t.insert(-0.0, 7.0), Ok(0));
    assert_eq!(t.len(), 3);
    assert_eq!(t.key_at(0.0), Some(0));
    assert_eq!(t.insert(5.0, f64::NAN), Err(EaseError::NotFinite("value")));
    assert_eq!(t.insert(f64::NEG_INFINITY, 1.0), Err(EaseError::NotFinite("time")));
    assert_eq!(t.remove(1).map(|k| k.value), Some(3.0));
    assert!(t.remove(9).is_none() && t.set_ease(9, Easing::Linear).is_none());
    // Before the first key, a new key eases linearly.
    assert_eq!(t.insert(-1.0, 0.0), Ok(0));
    assert_eq!(t.keys()[0].ease, Easing::Linear);
}

#[test]
fn keys_at_the_ends_of_the_number_range_sample_without_overflow() {
    let t = Track::from_keys(vec![key(-f64::MAX, -1.0f64, Easing::Linear), key(f64::MAX, 1.0, Easing::Linear)]).unwrap();
    assert_eq!(t.sample(0.0), Ok(0.0));
    let v = t.sample(f64::MAX / 2.0).unwrap();
    assert!((v - 0.5).abs() < 1e-12, "{v}");
    // Keys a hair apart.
    let near = Track::from_keys(vec![key(1.0, 0.0f64, Easing::Linear), key(1.0 + f64::EPSILON, 1.0, Easing::Linear)]).unwrap();
    assert!(near.sample(1.0 + f64::EPSILON / 2.0).unwrap().is_finite());
    // Values at opposite ends of the range interpolate without overflowing their difference.
    let wide = Track::from_keys(vec![key(0.0, f64::MAX, Easing::Linear), key(1.0, -f64::MAX, Easing::Linear)]).unwrap();
    assert_eq!(wide.sample(0.5), Ok(0.0));
    let v =
        Track::from_keys(vec![key(0.0, Vec2::new(-f64::MAX, 1e308), Easing::Linear), key(1.0, Vec2::new(f64::MAX, -1e308), Easing::Linear)]).unwrap();
    assert_eq!(v.sample(0.5), Ok(Vec2::ZERO));
    let a = Track::from_keys(vec![key(0.0, [f64::MAX; 2], Easing::Linear), key(1.0, [-f64::MAX; 2], Easing::Linear)]).unwrap();
    assert!(a.sample(0.25).unwrap().iter().all(|x| x.is_finite()));
    // Keys a subnormal step either side of 0 still interpolate between them.
    let tiny = Track::from_keys(vec![key(-f64::from_bits(1), 0.0f64, Easing::Linear), key(f64::from_bits(1), 1.0, Easing::Linear)]).unwrap();
    assert_eq!(tiny.sample(0.0), Ok(0.5));
    // An overshoot past the largest f32 is refused, not returned as infinity.
    let over = Track::from_keys(vec![key(0.0, -f32::MAX, Easing::BackOut), key(1.0, f32::MAX, Easing::Linear)]).unwrap();
    assert_eq!(over.sample(0.6), Err(EaseError::Overflow));
    // An overshoot whose step past the far key is too large to represent, with a result that
    // is not.
    let r = crate::lerp(-1e308, 5e307, 1.25);
    assert!((r / 8.75e307 - 1.0).abs() < 1e-12, "{r}");
    let back = Track::from_keys(vec![key(0.0, -f64::MAX, Easing::BackOut), key(1.0, -1e300, Easing::Linear)]).unwrap();
    assert!(back.sample(0.6).unwrap() > 0.0);
}

#[cfg(feature = "serde")]
#[test]
fn tracks_round_trip_through_files_and_bad_files_are_refused() {
    let t = Track::from_keys(vec![key(0.0, 1.0f64, Easing::Linear), key(1.5, 2.0, Easing::Hold), key(3.0, 0.5, Easing::QuadOut)]).unwrap();
    let text = serde_json::to_string(&t).unwrap();
    assert_eq!(
        text,
        r#"{"keys":[{"time":0.0,"value":1.0,"ease":"Linear"},{"time":1.5,"value":2.0,"ease":"Hold"},{"time":3.0,"value":0.5,"ease":"QuadOut"}]}"#
    );
    assert_eq!(serde_json::from_str::<Track<f64>>(&text).unwrap(), t);
    // Keys out of order are sorted; a missing ease is linear.
    let r: Track<f64> = serde_json::from_str(r#"{"keys":[{"time":2,"value":1},{"time":1,"value":0}]}"#).unwrap();
    assert_eq!((r.keys()[0].time, &r.keys()[1].ease), (1.0, &Easing::Linear));
    // Two keys at one time, or a time that is not a number, are a broken file.
    assert!(serde_json::from_str::<Track<f64>>(r#"{"keys":[{"time":1,"value":1},{"time":1,"value":0}]}"#).is_err());
    assert!(serde_json::from_str::<Track<f64>>(r#"{"keys":[{"time":"x","value":1}]}"#).is_err());
    // More keys than a track holds are refused while reading, before the rest are read.
    let mut big = String::from(r#"{"keys":["#);
    for i in 0..=MAX_TRACK_KEYS {
        if i > 0 {
            big.push(',');
        }
        big.push_str(&format!(r#"{{"time":{i},"value":0}}"#));
    }
    big.push_str("]}");
    let err = serde_json::from_str::<Track<f32>>(&big).unwrap_err().to_string();
    assert!(err.contains("more than 100000 keys"), "{err}");
}

/// Formats that write a struct as its fields in order (rather than by name) read a track too,
/// through the same checks.
#[cfg(feature = "serde")]
#[test]
fn tracks_read_from_formats_that_write_fields_in_order() {
    let t: Track<f64> = serde_json::from_str(r#"[[{"time":2,"value":1},{"time":1,"value":0,"ease":"QuadIn"}]]"#).unwrap();
    assert_eq!(t.keys().iter().map(|k| (k.time, k.value)).collect::<Vec<_>>(), [(1.0, 0.0), (2.0, 1.0)]);
    assert_eq!(t.keys()[0].ease, Easing::QuadIn);
    assert!(serde_json::from_str::<Track<f64>>(r#"[[{"time":1,"value":1},{"time":1,"value":0}]]"#).is_err());
    assert!(serde_json::from_str::<Track<f64>>("[]").is_err());
    assert!(serde_json::from_str::<Track<f64>>("[[], 1]").is_err());
}
