# craft-jsonrpc

`craft-jsonrpc` provides bounded JSON-RPC 2.0 server envelopes independently of a transport or an
application's method router. It can read a bounded newline-delimited message, parse it from bytes,
check request IDs and batch sizes, dispatch requests and notifications separately, and build
response envelopes. It does not start a server, choose an MCP protocol version, or interpret tool
names and arguments.

`Limits` defaults to 64 MiB per message and 4096 batch entries. `parse_json` checks the byte limit
before deserialization, and `process` rejects an oversized batch before calling any handler.
For newline-delimited transports, `read_line` bounds memory while reading; after an over-limit
line, the caller must close the stream or call `skip_line`, which discards the rest of that line
without buffering it, before reading the next message.
`serde_json`'s nesting limit remains in effect. Response sizes are the caller's responsibility.

A valid request receives the handler's result or error with its original string, numeric, or null
ID. The default accepts numeric IDs as required by [JSON-RPC 2.0](https://www.jsonrpc.org/specification#request_object),
which discourages fractional values. Set `integer_ids_only` and clear `allow_null_ids` for
protocols such as MCP that require integer or string IDs. A legacy server can retain its previous
null-ID behavior independently. An omitted ID is a notification: it calls the separate notification
handler and gets no reply.
`process_with` and `process_legacy_with` take one value that implements `Handler` in place of the
two closures. Its `request` and `notification` methods both take `&mut self`, so a server can keep
one session that requests and notifications both update, in message order, with each
notification's `params`.
Malformed batch entries receive an invalid-request error; an empty batch gets one error response;
a batch containing only notifications has no response. Invalid parameter shapes receive an
invalid-params error for requests and no reply for notifications. The handler decides whether
object or array parameters are valid for each method.

The caller may inspect the bounded parsed value before `process` to support an existing legacy
protocol. Parse failures and limit failures are returned explicitly so each transport can choose
its error and connection policy; `parse_error_response` builds the JSON-RPC parse error.
`ParseError::error_response` builds a response for any of these failures: `-32700` for invalid
JSON and `-32600` for a message or batch over its limit, each with a null ID.

The envelope behavior is extracted from StencilCraft's JSON-lines MCP/control server. App-specific
method routing, legacy control requests, and MCP lifecycle policy stay with the app.
