# craft-mcp

Transport-independent MCP server initialization and tool result helpers.
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
By default both respond with 2025-03-26, even if the client offered a different version.
`Session::with_versions` sets the accepted versions, in order of preference, from
`SUPPORTED_PROTOCOL_VERSIONS` (2025-06-18, 2025-03-26 and 2024-11-05). A client that
asks for one of them gets it back, and any other request gets the first.
`Session::negotiated_version` returns the version the session answered with, for
features that depend on it, such as the `MCP-Protocol-Version` HTTP header. Once
2025-06-18 is negotiated, `Policy::Strict` rejects every batched request and
`Session::accepts_batches` returns false. Reject a batch envelope with -32600 before
dispatching it when `accepts_batches` is false, because `Session::notification` cannot
tell a batched notification from a single one. Instructions set with
`Session::set_instructions` are returned by every initialize response, in either policy
and with or without legacy initialization.
An initialized notification moves the session to `Ready`.
This crate does not enforce every MCP lifecycle or capability rule; applications
must validate any additional requirements they depend on.

`roots::Roots` confines the file paths in tool arguments to read and write roots.
`Roots::check` first compares the text of the path, made absolute with its `.` and
`..` names removed, with the roots as given and as resolved, before any file system
call, so every path outside the roots gets the same `Outside` error whatever exists
there. It then resolves the path, following links, and returns it only when it lies
inside a root of the requested kind. On Windows it also refuses, in the path as
given and in the path its links lead to, device names, alternate data streams,
names that end in a dot or a space, `\\.\` and `\??\` paths, and `\\?\` paths
other than plain drive and UNC paths such as those `check` returns. It also
refuses a path on a network share unless a root of the requested kind is on the
same server and share. These Windows checks read only the text of the path and
run before any file system call, so a path whose text names another share does
not open a connection to that computer. A link or junction inside a root that
leads to another share is followed when the path is resolved, which can connect
to that share before the path is refused. `Roots::required` refuses to build
roots without both kinds. A path is checked before it is opened, so another
program that replaces a folder with a link between the two steps can still reach
outside the roots.

See the [MCP lifecycle specification](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle)
and the [2025-06-18 changes](https://modelcontextprotocol.io/specification/2025-06-18/changelog).
