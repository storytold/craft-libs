# craft-mcp

Transport-independent MCP server initialization and tool result helpers.
Use one `Session` per client connection with `craft-jsonrpc` for message parsing and
batch dispatch. The caller owns transport, tool definitions and execution, response
size limits, and shutdown. Pass the outer envelope's batch status to `Session::request`.

`Policy::Compatible` retains existing permissive clients: tool calls can precede
initialization, initialization can occur in a batch, and client information needs only
to be an object. `Policy::Strict` requires initialization first (except ping),
rejects batched or repeated initialization, and requires client name and version.
By default both respond with 2025-03-26, even if the client offered a different version.
`Session::with_versions` sets the accepted versions, in order of preference, from
`SUPPORTED_PROTOCOL_VERSIONS` (2025-06-18, 2025-03-26 and 2024-11-05). A client that
asks for one of them gets it back, and any other request gets the first.
`Session::negotiated_version` returns the version the session answered with, for
features that depend on it, such as the `MCP-Protocol-Version` HTTP header. Once
2025-06-18 is negotiated, `Policy::Strict` rejects every batched request and
`Session::accepts_batches` returns false. Reject a batch envelope with -32600 before
dispatching it when `accepts_batches` is false, because `Session::notification` cannot
tell a batched notification from a single one.
An initialized notification moves the session to `Ready`.
This crate does not enforce every MCP lifecycle or capability rule; applications
must validate any additional requirements they depend on.

See the [MCP lifecycle specification](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle)
and the [2025-06-18 changes](https://modelcontextprotocol.io/specification/2025-06-18/changelog).
