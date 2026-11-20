
#[cfg(test)]
mod command_tests {
    use craft_plugin_sdk::{json, testing};

    use super::plugin::craft_testing;

    #[test]
    fn say_greets_by_name_and_logs() {
        let logged = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let sink = logged.clone();
        let host = move |_op: u32, input: &[u8], out: &mut [u8]| {
            let request: craft_plugin_sdk::Value = craft_plugin_sdk::serde_json::from_slice(input).unwrap_or_default();
            sink.borrow_mut().push(request["msg"].as_str().unwrap_or_default().to_owned());
            testing::reply(out, &json!({}))
        };
        let response = testing::with_host(host, || craft_testing::invoke("say", json!({ "params": { "name": "Ada" }, "origin": "ui" })));
        assert_eq!(response, Ok(Some(json!({ "result": { "greeting": "hello, Ada" } }))));
        assert_eq!(*logged.borrow(), vec!["saying hello to Ada".to_owned()]);
    }
}
