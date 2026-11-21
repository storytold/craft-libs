;; `before` hook guests. Entries:
;;   0 veto with a fixed reason   (the no-delete sample plugin)
;;   1 rewrite {"gain":2}
;;   2 pass {}
;;   3 trap
;;   4 spin
;;   5 veto with a number: malformed
;;   6 not JSON
;;   7 calls cmd.execute app.set, then passes (a hook that runs a command)
;;   8 veto and rewrite together
;;   9 veto with a blank reason: malformed

(module
  (import "craft_host" "call" (func $host (param i32 i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1 16)
  (global $heap (mut i32) (i32.const 4096))
  (data (i32.const 64) "{\"veto\":\"clips cannot be deleted\"}")
  (data (i32.const 128) "{\"rewrite\":{\"gain\":2}}")
  (data (i32.const 192) "{}")
  (data (i32.const 224) "{\"veto\":5}")
  (data (i32.const 256) "not json")
  (data (i32.const 288) "{\"id\":\"app.set\",\"params\":{}}")
  (data (i32.const 352) "{\"veto\":\"no\",\"rewrite\":{\"gain\":9}}")
  (data (i32.const 416) "{\"veto\":\"   \"}")

  (func $alloc (param $len i32) (param $align i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (if (i32.gt_u (global.get $heap) (i32.const 30000)) (then (global.set $heap (local.get $p)) (return (i32.const 0))))
    (local.get $p))

  (func $lit (param $src i32) (param $len i32) (result i64)
    (local $out i32)
    (local.set $out (call $alloc (local.get $len) (i32.const 1)))
    (memory.copy (local.get $out) (local.get $src) (local.get $len))
    (i64.or (i64.shl (i64.extend_i32_u (local.get $len)) (i64.const 32)) (i64.extend_i32_u (local.get $out))))

  (func (export "craft_abi") (result i32) i32.const 1)
  (export "craft_alloc" (func $alloc))
  (func (export "craft_free") (param $p i32) (param $len i32) (param $align i32)
    (if (i32.eq (i32.add (local.get $p) (local.get $len)) (global.get $heap)) (then (global.set $heap (local.get $p)))))

  (func (export "craft_invoke") (param $e i32) (param $rp i32) (param $rl i32) (param $bp i32) (param $bl i32) (result i64)
    (if (i32.eq (local.get $e) (i32.const 0)) (then (return (call $lit (i32.const 64) (i32.const 34)))))
    (if (i32.eq (local.get $e) (i32.const 1)) (then (return (call $lit (i32.const 128) (i32.const 22)))))
    (if (i32.eq (local.get $e) (i32.const 2)) (then (return (call $lit (i32.const 192) (i32.const 2)))))
    (if (i32.eq (local.get $e) (i32.const 3)) (then unreachable))
    (if (i32.eq (local.get $e) (i32.const 4)) (then (loop $l (br $l))))
    (if (i32.eq (local.get $e) (i32.const 5)) (then (return (call $lit (i32.const 224) (i32.const 10)))))
    (if (i32.eq (local.get $e) (i32.const 6)) (then (return (call $lit (i32.const 256) (i32.const 8)))))
    (if (i32.eq (local.get $e) (i32.const 7)) (then
      (drop (call $host (i32.const 0x11) (i32.const 288) (i32.const 28) (i32.const 2048) (i32.const 256)))
      (return (call $lit (i32.const 192) (i32.const 2)))))
    (if (i32.eq (local.get $e) (i32.const 8)) (then (return (call $lit (i32.const 352) (i32.const 34)))))
    (if (i32.eq (local.get $e) (i32.const 9)) (then (return (call $lit (i32.const 416) (i32.const 14)))))
    (i64.const -1))
)
