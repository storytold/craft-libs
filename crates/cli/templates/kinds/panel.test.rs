
#[cfg(test)]
mod panel_tests {
    use std::collections::BTreeMap;

    use craft_plugin_sdk::abi::Op;
    use craft_plugin_sdk::{Value, json, testing};

    use super::plugin::craft_testing;

    /// A host with a working `state.get` and `state.put`.
    fn state_host() -> impl FnMut(u32, &[u8], &mut [u8]) -> i64 {
        let mut store = BTreeMap::<String, Value>::new();
        move |op, input, out| {
            let request: Value = craft_plugin_sdk::serde_json::from_slice(input).unwrap_or_default();
            let key = request["key"].as_str().unwrap_or_default().to_owned();
            match Op::from_number(op) {
                Some(Op::StatePut) => {
                    store.insert(key, request["value"].clone());
                    testing::reply(out, &json!({}))
                }
                Some(Op::StateGet) => match store.get(&key) {
                    Some(v) => testing::reply(out, &json!({ "found": true, "value": v })),
                    None => testing::reply(out, &json!({ "found": false })),
                },
                _ => testing::reply(out, &json!({})),
            }
        }
    }

    #[test]
    fn each_click_adds_one_and_the_count_is_kept() {
        testing::with_host(state_host(), || {
            let label = |event: Value| {
                let response = craft_testing::invoke("counter", event).expect("panel call").expect("panel answers");
                response["tree"]["children"][0]["text"].as_str().unwrap_or_default().to_owned()
            };
            assert_eq!(label(json!({ "event": "open" })), "Clicked 0 times");
            assert_eq!(label(json!({ "event": "action", "widget": "bump" })), "Clicked 1 times");
            assert_eq!(label(json!({ "event": "action", "widget": "bump" })), "Clicked 2 times");
            assert_eq!(label(json!({ "event": "open" })), "Clicked 2 times");
        });
    }
}
