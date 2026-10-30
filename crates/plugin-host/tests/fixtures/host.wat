;; craft_invoke(op, in_ptr, in_len, out_ptr, out_cap) forwards to craft_host.call and returns its i64.
(module
  (import "craft_host" "call" (func $host (param i32 i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1 16)
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param i32 i32 i32 i32 i32) (result i64)
    (call $host (local.get 0) (local.get 1) (local.get 2) (local.get 3) (local.get 4))))
