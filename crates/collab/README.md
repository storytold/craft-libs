# craft-collab

`craft-collab` is the co-editing wire contract the Craft apps share (add-in protocol 2). Apps that
edit one document together exchange document updates, state vectors and awareness through a
transport add-in. Two apps meet in one room only if they agree on:

- the size and encoding of a payload: at most 16 MiB an update and 64 KiB an awareness update
  before base64 (`MAX_UPDATE`, `MAX_AWARENESS`), standard base64 with optional padding;
- `crypto`: AES-256-GCM with a 32-byte room key. A payload is `nonce (12 random bytes) ||
  ciphertext || tag (16 bytes)`; the associated data is `room + "\n" + method`, so a message
  can't be replayed into another room or as another kind. Every failure to open a payload is the
  one error `CollabError::Decrypt`;
- `awareness`: the y-protocols encoding of who is in the room. `Awareness` keeps the peers,
  ignores replays and older clocks, and forgets a client that hasn't renewed its state for 30 s.
  What a state says (user, cursor, selection) is the app's.

The shared document itself is each app's schema and stays with the app, as do the transports.
This crate has no CRDT dependency.
