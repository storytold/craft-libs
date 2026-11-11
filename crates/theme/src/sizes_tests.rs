use serde_json::{Value, json};

use crate::format::{parse_sizes, write_sizes};

const COMPACT: &str = include_str!("../sizes/compact.json");
const COMFORTABLE: &str = include_str!("../sizes/comfortable.json");

#[test]
fn files_round_trip_and_keep_unknown_top_level_keys() {
    for text in [COMPACT, COMFORTABLE] {
        let file = parse_sizes(text.as_bytes()).unwrap();
        let written = write_sizes(&file).unwrap();
        assert_eq!(written, text, "the built-in files are in the written form");
        assert_eq!(parse_sizes(written.as_bytes()).unwrap().set, file.set);
    }
    let mut v: Value = serde_json::from_str(COMPACT).unwrap();
    v["note"] = json!({"kept": true});
    let written = write_sizes(&parse_sizes(v.to_string().as_bytes()).unwrap()).unwrap();
    assert!(written.contains("\"note\""), "{written}");
}
