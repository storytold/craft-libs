# craft-runtime (web)

Browser Craft apps use **craft-tier-store** (hot mirror + warm/cold tiers), **craft-worker-host**
(SharedWorker RPC), and **craft-gpu** providers dispatched through **craft-runtime**.

Tracking: [storytold/craft-libs#132](https://github.com/storytold/craft-libs/issues/132),
lab tasks wdunn001/craft-libs#4–#10.

## Repin order

1. craft-libs (this repo)
2. collab-craft / CollabCraft service
3. Application `-web` crates

## Static assets

Ship `assets/craft-worker-host.js` (+ brotli sibling) from CollabCraft static host.
