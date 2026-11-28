//! A list reads and writes as a plain sequence.

use craft_shared_vec::SharedVec;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Item {
    id: u32,
    name: String,
    children: SharedVec<Item>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PlainItem {
    id: u32,
    name: String,
    children: Vec<PlainItem>,
}

#[test]
fn a_json_array_round_trips_byte_for_byte() {
    let numbers: Vec<u32> = (0..5_000).map(|n| n * 7 % 1_013).collect();
    let text = serde_json::to_string(&numbers).unwrap_or_default();
    let list: SharedVec<u32> = serde_json::from_str(&text).unwrap_or_default();
    assert!(list.iter().eq(numbers.iter()));
    assert_eq!(serde_json::to_string(&list).ok(), Some(text));

    for text in ["[]", "[[1]]", "[[1,2],[],[3]]"] {
        let nested: SharedVec<SharedVec<u8>> = serde_json::from_str(text).unwrap_or_default();
        assert_eq!(serde_json::to_string(&nested).ok().as_deref(), Some(text));
    }
}

#[test]
fn a_tree_reads_and_writes_the_same_bytes_as_one_built_on_vec() {
    let leaf = |id: u32| PlainItem { id, name: format!("item {id}"), children: Vec::new() };
    let plain =
        PlainItem { id: 0, name: "root".into(), children: (1..300).map(|id| PlainItem { children: vec![leaf(id * 1_000)], ..leaf(id) }).collect() };
    for text in [serde_json::to_string(&plain).unwrap_or_default(), serde_json::to_string_pretty(&plain).unwrap_or_default()] {
        let shared: Item = serde_json::from_str(&text).unwrap_or_else(|_| Item { id: u32::MAX, name: String::new(), children: SharedVec::new() });
        assert_eq!(shared.children.len(), 299);
        let again = if text.contains('\n') { serde_json::to_string_pretty(&shared) } else { serde_json::to_string(&shared) };
        assert_eq!(again.ok(), Some(text));
    }
}

#[test]
fn input_that_is_not_a_sequence_is_an_error() {
    for text in ["{\"a\":1}", "7", "\"[1]\"", "[1,", "[1,\"two\"]"] {
        assert!(serde_json::from_str::<SharedVec<u32>>(text).is_err(), "{text}");
    }
}
