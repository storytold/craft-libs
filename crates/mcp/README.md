# craft-mcp

Transport-independent MCP 2025-03-26 server initialization and tool result helpers.
Use one `Session` per client connection with `craft-jsonrpc` for message parsing and
batch dispatch. The caller owns transport, tool definitions and execution, response
size limits, and shutdown. Pass the outer envelope's batch status to `Session::request`.
Parse each message with `craft_jsonrpc::parse_json(bytes, jsonrpc_limits())`, which
rejects a message larger than `max_message_bytes`, and pass the parsed value to
`craft_jsonrpc::process` with the same limits, which rejects the null and fractional
request IDs that MCP forbids. `process` does not check the message size.

`Policy::Compatible` retains existing permissive clients: tool calls can precede
initialization, initialization can occur in a batch, and client information needs only
to be an object. `Policy::Strict` requires initialization first (except ping),
rejects batched or repeated initialization, and requires client name and version.
Both respond with the sole supported version, 2025-03-26, even if the client offered
a different version. Instructions set with `Session::set_instructions` are returned by
every initialize response, in either policy and with or without legacy initialization.
An initialized notification moves the session to `Ready`.
This crate does not enforce every MCP lifecycle or capability rule; applications
must validate any additional requirements they depend on.

See the [MCP lifecycle specification](https://modelcontextprotocol.io/specification/2025-03-26/basic/lifecycle).
