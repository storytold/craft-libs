;; Conformance plugin `start-loop`. The start function never returns. Instantiation must end by
;; fuel, not hang the host.
(module
  (memory (export "memory") 1 16)
  (func $spin (loop $forever (br $forever)))
  (start $spin)
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param i32 i32 i32 i32 i32) (result i64) i64.const 0))
