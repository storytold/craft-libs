;; Conformance plugin `oom`. craft_invoke(entry, pages, ...) returns memory.grow(pages): the old
;; size in pages, or -1. The declared maximum is far above any test cap, so a host cap decides.
(module
  (memory (export "memory") 1 1024)
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param $entry i32) (param $pages i32) (param i32 i32 i32) (result i64)
    (i64.extend_i32_s (memory.grow (local.get $pages)))))
