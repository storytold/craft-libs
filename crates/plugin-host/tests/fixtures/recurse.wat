;; craft_invoke(depth, ...) recurses depth times and returns depth.
(module
  (memory (export "memory") 1 16)
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func $rec (param i32) (result i32)
    (if (result i32) (i32.eqz (local.get 0))
      (then (i32.const 0))
      (else (i32.add (i32.const 1) (call $rec (i32.sub (local.get 0) (i32.const 1)))))))
  (func (export "craft_invoke") (param i32 i32 i32 i32 i32) (result i64)
    (i64.extend_i32_s (call $rec (local.get 0)))))
