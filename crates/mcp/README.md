# craft-mcp

Transport-independent MCP 2025-03-26 server initialization and result helpers for tools,
resources, prompts and completion.
Use one `Session` per client connection with `craft-jsonrpc` for message parsing and
batch dispatch. The caller owns transport, tool definitions and execution, response
size limits, and shutdown. Pass the outer envelope's batch status to `Session::request`.

`Policy::Compatible` retains existing permissive clients: tool calls can precede
initialization, initialization can occur in a batch, and client information needs only
to be an object. `Policy::Strict` requires initialization first (except ping),
rejects batched or repeated initialization, and requires client name and version.
Both respond with the sole supported version, 2025-03-26, even if the client offered
a different version. An initialized notification moves the session to `Ready`.
This crate does not enforce every MCP lifecycle or capability rule; applications
must validate any additional requirements they depend on.

See the [MCP lifecycle specification](https://modelcontextprotocol.io/specification/2025-03-26/basic/lifecycle).

The crate also checks the parameters of the resource, prompt and completion methods and
builds their results, for a server that routes those methods itself:

- `page` takes one page of a list from the request's `cursor`, and `resource_list`,
  `resource_template_list` and `prompt_list` wrap the page with its `nextCursor`. A
  cursor that `page` did not hand out is rejected with -32602.
- `resource_read` returns the requested URI. `text_contents`, `blob_contents` and
  `read_result` build the `resources/read` result, and `resource_not_found` is the
  -32002 error with the URI in its data.
- `prompt_get` returns the prompt name and its string arguments. `prompt_result` and
  `text_message` build the `prompts/get` result.
- `completion_request` returns the prompt or resource reference and the argument being
  completed. `completion_result` keeps the first 100 values and sets `total` and `hasMore`.

See the [pagination](https://modelcontextprotocol.io/specification/2025-03-26/server/utilities/pagination),
[resources](https://modelcontextprotocol.io/specification/2025-03-26/server/resources),
[prompts](https://modelcontextprotocol.io/specification/2025-03-26/server/prompts) and
[completion](https://modelcontextprotocol.io/specification/2025-03-26/server/utilities/completion)
specifications.
