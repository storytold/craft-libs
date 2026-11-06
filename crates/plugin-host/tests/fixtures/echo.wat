;; A small real plugin. Entries: 0 echo, 1 trap, 2 spin, 3 cmd.execute then trap, 4 {"error"},
;; 5 host.log then result, 6 session counter, 7 version digit, 8 not JSON.
;; Other entries return status -1.
;; Tests replace the hook marker lines in the two state functions and the version global to build variants.
(module
  (import "craft_host" "call" (func $host (param i32 i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1 16)
  (global $heap (mut i32) (i32.const 4096))
  (global $n (mut i32) (i32.const 0))
  (global $ver i32 (i32.const 49))
  (data (i32.const 64) "{\"result\":")
  (data (i32.const 128) "{\"error\":\"nope\"}")
  (data (i32.const 192) "{\"id\":\"app.set\",\"params\":{}}")
  (data (i32.const 256) "{\"msg\":\"hello\"}")
  (data (i32.const 320) "{\"result\":1}")
  (data (i32.const 352) "not json")

  (func $alloc (param $len i32) (param $align i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (if (i32.gt_u (global.get $heap) (i32.const 60000)) (then (global.set $heap (local.get $p)) (return (i32.const 0))))
    (local.get $p))

  (func $pack (param $out i32) (param $len i32) (result i64)
    (i64.or (i64.shl (i64.extend_i32_u (local.get $len)) (i64.const 32)) (i64.extend_i32_u (local.get $out))))

  ;; copy `len` bytes from the data segment at `src` into a fresh buffer and pack it
  (func $lit (param $src i32) (param $len i32) (result i64)
    (local $out i32)
    (local.set $out (call $alloc (local.get $len) (i32.const 1)))
    (memory.copy (local.get $out) (local.get $src) (local.get $len))
    (call $pack (local.get $out) (local.get $len)))

  (func (export "craft_abi") (result i32) i32.const 1)
  (export "craft_alloc" (func $alloc))
  (func (export "craft_free") (param $p i32) (param $len i32) (param $align i32)
    (if (i32.eq (i32.add (local.get $p) (local.get $len)) (global.get $heap)) (then (global.set $heap (local.get $p)))))

  (func (export "craft_invoke") (param $e i32) (param $rp i32) (param $rl i32) (param $bp i32) (param $bl i32) (result i64)
    (local $out i32) (local $len i32)
    (if (i32.eq (local.get $e) (i32.const 0)) (then
      (local.set $len (i32.add (local.get $rl) (i32.const 11)))
      (local.set $out (call $alloc (local.get $len) (i32.const 1)))
      (memory.copy (local.get $out) (i32.const 64) (i32.const 10))
      (memory.copy (i32.add (local.get $out) (i32.const 10)) (local.get $rp) (local.get $rl))
      (i32.store8 (i32.add (local.get $out) (i32.add (i32.const 10) (local.get $rl))) (i32.const 125))
      (return (call $pack (local.get $out) (local.get $len)))))
    (if (i32.eq (local.get $e) (i32.const 1)) (then unreachable))
    (if (i32.eq (local.get $e) (i32.const 2)) (then (loop $l (br $l))))
    (if (i32.eq (local.get $e) (i32.const 3)) (then
      (drop (call $host (i32.const 0x11) (i32.const 192) (i32.const 28) (i32.const 2048) (i32.const 256)))
      unreachable))
    (if (i32.eq (local.get $e) (i32.const 4)) (then (return (call $lit (i32.const 128) (i32.const 16)))))
    (if (i32.eq (local.get $e) (i32.const 5)) (then
      (drop (call $host (i32.const 0x1) (i32.const 256) (i32.const 15) (i32.const 2048) (i32.const 64)))
      (return (call $lit (i32.const 320) (i32.const 12)))))
    (if (i32.eq (local.get $e) (i32.const 6)) (then
      (global.set $n (i32.add (global.get $n) (i32.const 1)))
      (local.set $out (call $alloc (i32.const 12) (i32.const 1)))
      (memory.copy (local.get $out) (i32.const 320) (i32.const 12))
      (i32.store8 (i32.add (local.get $out) (i32.const 10)) (i32.add (i32.const 48) (global.get $n)))
      (return (call $pack (local.get $out) (i32.const 12)))))
    (if (i32.eq (local.get $e) (i32.const 7)) (then
      (local.set $out (call $alloc (i32.const 12) (i32.const 1)))
      (memory.copy (local.get $out) (i32.const 320) (i32.const 12))
      (i32.store8 (i32.add (local.get $out) (i32.const 10)) (global.get $ver))
      (return (call $pack (local.get $out) (i32.const 12)))))
    (if (i32.eq (local.get $e) (i32.const 8)) (then (return (call $lit (i32.const 352) (i32.const 8)))))
    (i64.const -1))

  (func (export "craft_state_save") (param $outpp i32) (result i32)
    (local $out i32)
    ;;SAVE_HOOK
    (local.set $out (call $alloc (i32.const 1) (i32.const 1)))
    (i32.store8 (local.get $out) (global.get $n))
    (i32.store (local.get $outpp) (local.get $out))
    (i32.const 1))

  (func (export "craft_state_load") (param $p i32) (param $len i32) (param $vp i32) (param $vl i32) (result i32)
    ;;LOAD_HOOK
    (global.set $n (i32.load8_u (local.get $p)))
    (i32.const 0))
)
