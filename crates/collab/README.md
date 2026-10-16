# craft-collab

The shared collaboration core for the Crafting Apps family, extracted from the proven
`figcraft-collab` and generalized (the CraftHub specification §2.15/§3.4) — the one
sanctioned exception to the family rule that repos share standards, not code.

## What it holds

| module | role |
|--------|------|
| `frac` | fractional indexing: sibling order as sortable base-62 keys, never renumbering |
| `ids` | client-scoped 64-bit ids allocated without coordination |
| `change` | the change algebra: Create/Set/Delete/SetDoc/Put/Remove, one batch = one undo step |
| `replica` | the synchronized state; strict apply (server) and lenient (client replay) |
| `diff` | the edit-sized diff; `apply(diff(a, b), a) == b` is a tested property |
| `server` | the pure server: validate whole batches, accept with a seq or refuse with fixes |
| `client` | optimistic application, reconciliation, per-user undo by server order |
| `journal` | append-only persistence: no acknowledgment before the write |
| `code` | join codes: Crockford base-32, 10,000-round stretching, constant-time compares |
| `text` | the RGA text property for fine-grained concurrent writing |
| `merge` | three-way merge and the per-list merge strategies |
| `sim` | the N-client convergence simulator: random delays, drops, reconnects — per seed |
| `adapter` | `DocSchema` + `HubAdapter`: how an app maps its document onto the core |

Pure: no sockets, no files, no clock, no randomness source — all I/O behind traits so the
whole crate is testable in memory.

## Using it

```toml
craft-collab = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

An app integrates by implementing `HubAdapter` (a property table, a view/apply/diff bridge
and a presence projection) and proving itself with the simulator — the figcraft and
wordcraft reference adapters in the CraftHub repository show the shape.
