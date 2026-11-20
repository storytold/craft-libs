;; Conformance plugin `alloc-bomb`. craft_invoke(entry, ...) grows memory one page at a time until
;; a grow fails, then returns the number of pages it gained. The module's own maximum is 64 pages.
;; A host cap below 64 pages must stop it with an out of memory trap. A host cap above 64 pages
;; leaves only the module maximum, so the grow returns -1 and the call returns 63.
(module
  (memory (export "memory") 1 64)
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param i32 i32 i32 i32 i32) (result i64)
    (local $gained i64)
    (loop $storm
      (if (i32.ne (memory.grow (i32.const 1)) (i32.const -1))
        (then (local.set $gained (i64.add (local.get $gained) (i64.const 1))) (br $storm))))
    (local.get $gained)))
