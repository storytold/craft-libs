;; Bump allocator. craft_free records the last freed block; craft_invoke returns it as len<<32|ptr.
(module
  (memory (export "memory") 1 16)
  (global $top (mut i32) (i32.const 1024))
  (global $fp (mut i32) (i32.const 0))
  (global $fl (mut i32) (i32.const 0))
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param $len i32) (param $align i32) (result i32)
    (local $p i32)
    (local.set $p
      (i32.and
        (i32.add (global.get $top) (i32.sub (local.get $align) (i32.const 1)))
        (i32.xor (i32.sub (local.get $align) (i32.const 1)) (i32.const -1))))
    (if (i64.gt_u
          (i64.add (i64.extend_i32_u (local.get $p)) (i64.extend_i32_u (local.get $len)))
          (i64.extend_i32_u (i32.shl (memory.size) (i32.const 16))))
      (then (return (i32.const 0))))
    (global.set $top (i32.add (local.get $p) (local.get $len)))
    (local.get $p))
  (func (export "craft_free") (param i32 i32 i32)
    (global.set $fp (local.get 0))
    (global.set $fl (local.get 1)))
  (func (export "craft_invoke") (param i32 i32 i32 i32 i32) (result i64)
    (i64.or (i64.shl (i64.extend_i32_u (global.get $fl)) (i64.const 32)) (i64.extend_i32_u (global.get $fp)))))
