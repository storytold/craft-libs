# craft-crdt

Conflict-free replicated types for Craft documents. Fractional sibling keys, RGA text,
a replica, and a memory-link simulator that proves concurrent edits converge.

This crate does not open a socket. CollabCraft is the live relay: rooms, tokens,
presence, and lock arbitration for operations that do not commute. Apps depend on
both. They send `crdt.text` commands with no lock scope, and they apply the payload
through [`TextDoc`](src/text.rs).

## What it holds

| module | role |
|--------|------|
| `frac` | fractional indexing: sibling order as sortable base-62 keys, never renumbering |
| `ids` | client-scoped 64-bit ids allocated without coordination |
| `change` | the change algebra: Create/Set/Delete/SetDoc/Put/Remove |
| `replica` | the synchronized state; strict apply and lenient replay |
| `diff` | the edit-sized diff; `apply(diff(a, b), a) == b` is a tested property |
| `server` | in-crate test oracle: validate whole batches, accept with a seq or refuse. Not the live relay |
| `text` | the RGA text property for fine-grained concurrent writing |
| `merge` | three-way merge and the per-list merge strategies |
| `sim` | the N-client convergence simulator: random delays, drops, reconnects, per seed |

## Using it

```toml
craft-crdt = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

WordCraft is the first app: each paragraph is a `TextDoc`. SolidCraft feature trees
and FilmCraft renders keep CollabCraft's region locks.
