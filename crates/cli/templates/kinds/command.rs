
    /// Runs when the user or a script calls the command. `call.params` is the JSON the caller sent.
    fn say(call: CommandCall) -> Result<Value, String> {
        let name = call.params.get("name").and_then(Value::as_str).unwrap_or("world");
        host::info(&format!("saying hello to {name}"));
        Ok(json!({ "greeting": format!("hello, {name}") }))
    }
