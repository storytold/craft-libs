;; Conformance plugin `loop`. craft_invoke(entry, n, ...):
;;   entry 0: infinite loop of cheap instructions, never returns.
;;   entry 1: counts to n and returns n. Control: a finite loop finishes inside a big enough budget.
(module
  (memory (export "memory") 1 16)
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param $entry i32) (param $n i32) (param i32 i32 i32) (result i64)
    (local $i i32)
    (if (i32.eqz (local.get $entry)) (then (loop $forever (br $forever))))
    (loop $count
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $count (i32.lt_u (local.get $i) (local.get $n))))
    (i64.extend_i32_u (local.get $i))))
