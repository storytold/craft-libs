# Suite-wide AI contract for app maintainers

**Status:** proposed for review, 13/10/2026. This repo receives `craft-ai-core` (the wasm-safe data model and contract) first; the store, client, adapters and widgets named below arrive in later pull requests and are described here so the whole contract can be reviewed at once. No app mounts it end to end yet.

## Ownership boundary

| Shared crates | Each app or launcher |
| --- | --- |
| Versioned connection/model/tool declarations, suite defaults and per-app overrides in one shared store | App-specific tool UI, request construction, document changes, display of results |
| Model compatibility, binding resolution, consent and job contract | A suitable registered execution adapter, if the chosen backend is actually supported in that app |
| Shared picker, connection panel and optional reusable widgets | Where/how those widgets appear, and app-specific setup |

A registered tool is a **declaration**, not installed code. A compatible binding is not evidence that a provider can execute the request. Apps must fail explicitly when the selected adapter is absent, not silently substitute another engine. A separate database should be added only for an identified indexing or coordination need; the current versioned store remains authoritative for these settings.

## Declaring and adopting a tool

1. Define a `craft_ai_core::Role` with a stable `RoleId`, a display label, one `CapabilityId`, and a nonempty set of `InputRole`s. Choose a suite-wide ID with the other maintainers. `ToolDesk::register_tool(role)` writes it once to the shared store; `ToolDesk::custom_tools()` reads registered declarations in any process using that same store. Built-in IDs cannot be replaced. A declaration does not add it to the seven built-in catalogue.
2. Add or discover a connection and a model advertising that capability and the required inputs. `ToolDesk::choose(&role, &connection_id, &model_id)` sets the suite default. The shared update verifies the declared tool contract and model compatibility before persisting the binding. An app can use `AiClient::bind` for an explicit per-app override. Removal with `ToolDesk::remove_tool(&role_id)` deletes the declaration and its suite/app bindings together.
3. An app reads the same store and resolves the role with `AiClient::status`, `choices`, `picker`, `plan` and `start` as appropriate. It may localise the label, but **must not reinterpret the same ID's capability or inputs**. Mismatched contracts are rejected during resolution, model listing, picker creation and binding. Apps still own their UI, input conversion and actual execution route.

Concrete API examples and regressions: `crates/ai-client/tests/custom_tools.rs`, `crates/ai-core/tests/custom_tools.rs` and `crates/ai-native/tests/shared_store.rs`. The shared store is `craft_ai_native::shared::SharedStore`; it uses locked atomic updates for registration, binding and removal. Old version-1 JSON files without `custom_tools` still load with an empty list. Secret values belong in the secret store, never in the shared JSON or a tool declaration.

## Deliberate limits and next gate

The current surface does **not** ship a dynamic adapter registry for arbitrary user code, a general tool executor, app-specific menus, automatic model installation or a complete provider route. Existing native/hosted adapters have separate capability and real-service gates. Before calling this platform complete, maintainers need a small executable adoption example and conformance tests proving that a declared tool, a bound connection and a registered adapter run one request across two independent app clients, including an unavailable-adapter error, consent, cancellation and a legacy-store migration. The apps themselves remain outside this shared-platform task.
