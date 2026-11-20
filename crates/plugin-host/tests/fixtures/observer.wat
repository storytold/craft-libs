;; `after` and `change` hook guests that record what they are given in `state.local`.
;; Entries:
;;   0 after: appends the request under key e0, e1, ... (the log-edits sample plugin)
;;   1 change: appends the request under key c0, c1, ...
;;   2 after: traps
;;   4 before: passes with {}
;;   3 after: calls host.log, then stores the request under "lost" when it is an events.lost
;;     marker and under "last" otherwise
;; The state key counters count per entry and wrap at 10.
(module
  (import "craft_host" "call" (func $host (param i32 i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1 16)
  (global $heap (mut i32) (i32.const 4096))
  (global $ne (mut i32) (i32.const 0))
  (global $nc (mut i32) (i32.const 0))
  ;; `{"key":"X0","value":` with the letter at offset 8 and the digit at offset 9
  (data (i32.const 128) "{\"key\":\"X0\",\"value\":")
  (data (i32.const 192) "{\"key\":\"last\",\"value\":")
  (data (i32.const 256) "{\"key\":\"lost\",\"value\":")
  (data (i32.const 320) "{\"msg\":\"seen\"}")
  (data (i32.const 384) "{}")

  (func $alloc (param $len i32) (param $align i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (if (i32.gt_u (global.get $heap) (i32.const 30000)) (then (global.set $heap (local.get $p)) (return (i32.const 0))))
    (local.get $p))

  ;; put `{"key":"..","value":` (template at $tpl, $tl bytes) + request + `}`
  (func $put (param $tpl i32) (param $tl i32) (param $rp i32) (param $rl i32) (result i32)
    (local $len i32) (local $out i32)
    (local.set $len (i32.add (i32.add (local.get $tl) (local.get $rl)) (i32.const 1)))
    (local.set $out (call $alloc (local.get $len) (i32.const 1)))
    (memory.copy (local.get $out) (local.get $tpl) (local.get $tl))
    (memory.copy (i32.add (local.get $out) (local.get $tl)) (local.get $rp) (local.get $rl))
    (i32.store8 (i32.add (local.get $out) (i32.sub (local.get $len) (i32.const 1))) (i32.const 125))
    (drop (call $host (i32.const 0x51) (local.get $out) (local.get $len) (i32.const 32768) (i32.const 256)))
    (local.get $out))

  (func (export "craft_abi") (result i32) i32.const 1)
  (export "craft_alloc" (func $alloc))
  (func (export "craft_free") (param $p i32) (param $len i32) (param $align i32)
    (if (i32.eq (i32.add (local.get $p) (local.get $len)) (global.get $heap)) (then (global.set $heap (local.get $p)))))

  (func (export "craft_invoke") (param $e i32) (param $rp i32) (param $rl i32) (param $bp i32) (param $bl i32) (result i64)
    (local $out i32)
    (if (i32.eq (local.get $e) (i32.const 0)) (then
      (i32.store8 (i32.const 136) (i32.const 101))
      (i32.store8 (i32.const 137) (i32.add (i32.const 48) (i32.rem_u (global.get $ne) (i32.const 10))))
      (global.set $ne (i32.add (global.get $ne) (i32.const 1)))
      (drop (call $put (i32.const 128) (i32.const 20) (local.get $rp) (local.get $rl)))
      (return (i64.const 0))))
    (if (i32.eq (local.get $e) (i32.const 1)) (then
      (i32.store8 (i32.const 136) (i32.const 99))
      (i32.store8 (i32.const 137) (i32.add (i32.const 48) (i32.rem_u (global.get $nc) (i32.const 10))))
      (global.set $nc (i32.add (global.get $nc) (i32.const 1)))
      (drop (call $put (i32.const 128) (i32.const 20) (local.get $rp) (local.get $rl)))
      (return (i64.const 0))))
    (if (i32.eq (local.get $e) (i32.const 2)) (then unreachable))
    (if (i32.eq (local.get $e) (i32.const 3)) (then
      (drop (call $host (i32.const 0x1) (i32.const 320) (i32.const 14) (i32.const 32768) (i32.const 64)))
      ;; the request is `{"events.lost":n}` or `{"call":...`: the byte at offset 2 is `e` or `c`
      (if (i32.eq (i32.load8_u (i32.add (local.get $rp) (i32.const 2))) (i32.const 101))
        (then (drop (call $put (i32.const 256) (i32.const 22) (local.get $rp) (local.get $rl))))
        (else (drop (call $put (i32.const 192) (i32.const 22) (local.get $rp) (local.get $rl)))))
      (return (i64.const 0))))
    (if (i32.eq (local.get $e) (i32.const 4)) (then
      (local.set $out (call $alloc (i32.const 2) (i32.const 1)))
      (memory.copy (local.get $out) (i32.const 384) (i32.const 2))
      (return (i64.or (i64.shl (i64.const 2) (i64.const 32)) (i64.extend_i32_u (local.get $out))))))
    (i64.const -1))
)
