# craft-evm

Read-only EVM bytecode inspection: strict raw/hex admission, opcode listing, dispatcher selector and jump-pattern facts, and Solidity/Vyper CBOR metadata. No VM execution, RPC, wallet, signing or transaction support.

Preserves Craft Anything's parser behavior and limits: 16 MiB decoded bytecode, 4,096 reported functions and 400 listing entries, plus existing bounded CBOR recursion/collection parsing. These are not a total allocator-byte guarantee. Dispatcher facts are heuristics, not a full control-flow or semantic proof. EOF containers remain unsupported.

Application file I/O, evidence policy and selector naming remain outside this crate.
