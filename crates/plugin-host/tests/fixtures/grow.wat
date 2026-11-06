;; craft_invoke(pages, ...) returns memory.grow(pages): the old size in pages, or -1.
(module
  (memory (export "memory") 1 16)
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param i32 i32 i32 i32 i32) (result i64)
    (i64.extend_i32_s (memory.grow (local.get 0)))))
