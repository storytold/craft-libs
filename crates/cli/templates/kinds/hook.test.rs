
#[cfg(test)]
mod hook_tests {
    use craft_plugin_sdk::{json, testing};

    use super::plugin::craft_testing;

    #[test]
    fn log_commands_reports_the_command_id_and_outcome() {
        let logged = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let sink = logged.clone();
        let host = move |_op: u32, input: &[u8], out: &mut [u8]| {
            let request: craft_plugin_sdk::Value = craft_plugin_sdk::serde_json::from_slice(input).unwrap_or_default();
            sink.borrow_mut().push(request["msg"].as_str().unwrap_or_default().to_owned());
            testing::reply(out, &json!({}))
        };
        let request = json!({ "call": { "id": "clip.add", "params": {}, "depth": 0, "origin": "ui", "seq": 1 }, "outcome": "ok" });
        let response = testing::with_host(host, || craft_testing::invoke("log-commands", request));
        assert_eq!(response, Ok(Some(json!({}))));
        assert_eq!(*logged.borrow(), vec!["clip.add finished: ok".to_owned()]);
    }
}
