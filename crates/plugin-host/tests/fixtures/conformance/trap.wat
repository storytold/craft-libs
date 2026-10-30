;; Conformance plugin `trap`. craft_invoke(entry, arg, ...) runs one guest fault per entry:
;;   0 unreachable        1 divide by zero         2 load past the end of memory
;;   3 i32.MIN / -1       4 call_indirect with the wrong signature
;;   5 call_indirect to an empty table slot        6 returns 7 (control: no fault)
(module
  (type $unary (func (param i32)))
  (type $other (func (result i32)))
  (memory (export "memory") 1 16)
  (table 2 funcref)
  (elem (i32.const 0) $f0)
  (func $f0 (param i32))
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param $entry i32) (param $arg i32) (param i32 i32 i32) (result i64)
    (if (i32.eq (local.get $entry) (i32.const 0)) (then unreachable))
    (if (i32.eq (local.get $entry) (i32.const 1)) (then (return (i64.extend_i32_u (i32.div_u (i32.const 10) (local.get $arg))))))
    (if (i32.eq (local.get $entry) (i32.const 2)) (then (return (i64.extend_i32_u (i32.load (i32.const -16))))))
    (if (i32.eq (local.get $entry) (i32.const 3)) (then (return (i64.extend_i32_s (i32.div_s (i32.const 0x80000000) (i32.const -1))))))
    (if (i32.eq (local.get $entry) (i32.const 4)) (then (return (i64.extend_i32_s (call_indirect (type $other) (i32.const 0))))))
    (if (i32.eq (local.get $entry) (i32.const 5)) (then (call_indirect (type $unary) (i32.const 0) (i32.const 1))))
    (i64.const 7)))
