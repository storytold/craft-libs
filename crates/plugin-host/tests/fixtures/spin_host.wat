;; craft_invoke polls host.cancelled (op 3) forever.
(module
  (import "craft_host" "call" (func $host (param i32 i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1 16)
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param i32 i32 i32 i32 i32) (result i64)
    (loop $forever
      (drop (call $host (i32.const 3) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)))
      (br $forever))
    i64.const 0))
