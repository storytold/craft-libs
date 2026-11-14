# u2 design: a declarative adapter for providers that are not OpenAI-shaped

Written 12/10/2026. **Built 13/10/2026** (see "As built" at the end); the rest of this note is the original design.
Row: `u2` in `ai-framework-handoff/handoff_tasks.py`. Depends on u1 (built), p1 and p3.

## What it is

One generic adapter, `craft-ai-declarative`, that talks to a provider whose API is described by a
small data file. The file says which calls to make, how to authenticate and where in the replies to
find the result. The adapter implements the existing `Provider` trait, so apps, consent, history and
the conformance suite see nothing new.

## What it is not

- It runs no code from the file or from anywhere else. There are no scripts, no expressions, no
  loops and no conditions in a spec. A spec is data, checked once when it is loaded.
- It does not read local files, environment variables or other hosts.
- It does not cover APIs that need signing, token refresh, multipart uploads, streaming or webhooks
  (see "Reported as unsupported"). Those keep needing a hand-written adapter.
- It does not share specs between people. Where a spec comes from is a person's own choice for now.

## The spec (version 1, JSON)

JSON, because the model crate already speaks serde JSON, the store is JSON, and a pure validator can
live in the wasm-safe model crate so the launcher can check a spec as it is pasted.

```json
{
  "spec_version": 1,
  "id": "example-images",
  "label": "Example Images",
  "base": "https://api.example.com/v2",
  "auth": { "kind": "header", "name": "x-api-key" },
  "probe": { "method": "GET", "path": "/account" },
  "models": [
    { "id": "img-1", "label": "Image 1", "capability": "image.generate",
      "accepts": { "prompt": 1 }, "max_outputs": 1 }
  ],
  "flow": { "kind": "request_response" | "queue", "...": "see below" },
  "limits": { "poll_interval_ms": 1000, "max_polls": 600 }
}
```

- `base`: https, or http for a loopback address, by the same `Endpoint` rule as every connection.
  Every call is `base` plus a path. A path starts with `/`, has no `..`, no `//`, no scheme and no
  host. An absolute URL in a path is an error.
- `auth`: `bearer` (default), `header` with a name (the `check_key_header` rules from u1), or `none`.
  The key is never part of a spec. It comes from the connection's secret reference, exactly as for
  every other connection, and is only ever placed in the one header `auth` names.
- `models`: at most 16. Each declares its capability and the inputs it takes. Nothing is offered
  that the spec does not declare, the same rule as a u1 connection.
- `probe`: an optional free request for the Test button. Without one, Test reports that nothing
  could be confirmed.

### `request_response` flow

```json
{ "kind": "request_response",
  "call": { "method": "POST", "path": "/images",
            "body": { "model": "{model}", "prompt": "{prompt}", "n": 1 } },
  "result": { "at": "/data/0/b64_json", "as": "base64", "media_type": "image/png" } }
```

### `queue` flow

```json
{ "kind": "queue",
  "submit": { "method": "POST", "path": "/flux-3-image", "body": { "prompt": "{prompt}" },
              "job_id_at": "/id" },
  "poll": { "method": "GET", "path": "/get_result", "query": { "id": "{job_id}" },
            "status_at": "/status",
            "states": { "Pending": "queued", "Generating": "running", "Ready": "succeeded",
                        "Error": "failed", "Request Moderated": "failed" },
            "result": { "at": "/result/sample", "as": "url", "media_type": "image/png" } },
  "cancel": null }
```

### Templates and extraction

- A template is a JSON value in the spec. The only substitution is `{name}` inside a string leaf,
  from a closed set: `prompt`, `model`, `job_id`, `seed`, and `param.<name>` for parameters the model
  declares. The structure of a request always comes from the spec, never from the user's text, so a
  prompt cannot add a field or change a key. Inserted text is escaped as JSON.
- Extraction is a JSON pointer (RFC 6901) plus one of `base64`, `url` or `text`. Nothing else.
- Status words map to the four states the trait has: queued, running, succeeded, failed. A status
  word the table does not list is an error, never a guess.

## Safety and bounds, checked when the spec is loaded

- Spec file at most 32 KiB; at most 16 models, 8 parameters per model, templates at most 4 KiB,
  pointers at most 8 levels deep, status table at most 16 entries.
- Unknown fields are refused (`deny_unknown_fields`). A feature this version does not have fails
  with its name, for example "unsupported: flow.webhook", instead of being ignored.
- Static headers a spec may add: `accept` and names that pass the u1 header rules, values at most
  256 printable ASCII characters. No header can be `authorization`, `host`, `cookie` or anything
  the transport owns.
- At run time: no redirects followed, replies capped (32 MiB for JSON, with the existing 128 MiB
  ceiling for inline base64 images), the number of polls and the total job time bounded by
  `limits`, a job id must be a short printable string, at most 8 jobs at once per connection.
- A result that is a URL is returned as a handle and is never fetched by this adapter. The existing
  `craft-ai-media` wrapper copies it under its own rules (https, no private addresses, no
  credentials, redirects re-checked). The key is never sent to a result host.
- The poll address is always built from the spec and `base`. A polling URL supplied by the server
  is ignored, as in the BFL adapter.
- Consent: the disclosure names the `base` origin, so changing the host asks again, as for any
  connection. The launcher shows a plain-language summary of a spec before saving it ("sends your
  prompt to api.example.com, then asks for the result every second").

## Where it sits

- `ai-core`: `spec.rs`, the types and the pure validator (wasm-safe, so the launcher can validate
  live). No I/O.
- New native crate `craft-ai-declarative`: the runner that implements `Provider`, with the HTTP
  layer copied in shape from `craft-ai-bfl` (no redirects, redacted errors, capped bodies).
- Storage: a spec is a file beside the connections store (`specs/<id>.json`). A connection uses it
  through `provider: "declarative"` and a new optional `spec` field holding the spec id. The u1
  `key_header` field is not used for such a connection: `auth` in the spec replaces it, so there is
  one place to look.
- `ProviderFactory` registers `declarative`; nothing in `AiClient` changes.

## How it would be proved (acceptance from the row)

Two scripted localhost servers, no live service:

1. A queue-style media API, modelled on the documented BFL flow shown above.
2. A plain request/response API, modelled on an OpenAI-style image call.

Both pass the shared conformance suite (`run_lifecycle_paced`, `run_cancel_paced`, `run_rejects`,
`run_unknown_job`). Then one test per rule above: an absolute URL or `..` in a path, an unknown
field, a reserved header, a key placeholder in a body, a redirect, an oversized reply, a status that
never becomes final (poll budget), a job id with control characters, a status word that is not in
the table, a prompt that tries to inject a field, and every size limit at and just past its edge.
Then the deliberate-bug check in `docs/research/phase2d/mutation_check.py` with one case per rule.

Size estimate: about 900 lines in four files (types and validator, templates and extraction, runner,
tests), none above 500.

## Reported as unsupported (by design, with the reason)

| Needed by | Why it is not data |
|---|---|
| Multipart image upload (edit, inpaint) | Needs input handles first (p4, p10); a later spec version |
| Streaming text (server-sent events) | A different job shape; keep the hand-written adapter |
| Request signing (AWS SigV4), OAuth refresh, service-account JWT | Needs code and secrets handling beyond one header |
| Webhooks, websockets | Needs a listener, which this layer does not open |
| Pagination, conditionals | Would turn the spec into a program |

## Open questions

1. JSON as the format (recommended) or TOML. JSON keeps one parser and lets the model crate validate.
2. Specs stay per user, added by hand or pasted, with no download of specs from anywhere. Agreed?
3. The pure validator in the wasm-safe model crate (recommended), so the launcher checks as you
   paste, with the runner in a native crate.
4. Spec sharing, signing and provenance are a later question and are not designed here.

## As built (13/10/2026)

Code: `crates/ai-core/src/spec.rs` and `spec/` (types, checker, substitutions; wasm-safe),
`crates/ai-declarative` (the runner), `crates/ai-client/src/declarative.rs` (the factory entry).
Tests: `ai-core/tests/spec_*.rs`, `ai-declarative/tests/{queue,direct}.rs`,
`ai-client/tests/declarative.rs`. Deliberate-bug cases: group `u2` in
`docs/research/phase2d/mutation_check.py` (cases in `mutation_u2.py`).

Where it differs from the design above:

- **No `spec` field on `Connection`.** The spec file is `specs/<connection id>.json` beside the
  connections store, so the shared contract is unchanged. Sharing one spec between connections is
  left for later.
- **The connection's address must equal the spec's `base`.** Consent and "on this computer" are
  decided from the connection's address, so a spec must not be able to send somewhere else.
- **Prompt only.** Models accept exactly one `prompt`; `{seed}`, `param.<name>`, uploads and any
  other input are refused by name. Requests with options are refused as Unsupported.
- **Placeholders:** `{prompt}`, `{model}`, `{job_id}` only; `{job_id}` is the only one allowed in
  a path; `{{` and `}}` write a brace.
- **No static headers** and no `poll_interval_ms`. Limits are `max_polls` and `max_seconds`.
- **Request/response jobs call once, on the first poll,** and a failure ends the job rather than
  resending a billable call. Queue status checks may be retried.
- **Cancel** is an optional `DELETE`/`POST` call that must use `{job_id}`; without one a running
  queue job reports Unsupported.
- **Finished jobs are forgotten (oldest first) when 128 are tracked;** all-running refuses with
  Quota before anything paid is sent.

Not run: any live or hosted service, TLS, other operating systems. The runner has only met
scripted localhost servers.
