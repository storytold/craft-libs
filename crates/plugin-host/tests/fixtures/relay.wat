;; Hands its request to a host op and returns the host's answer. Entry 0 relays `link.call`
;; (0x40), entry 1 relays `link.apps` (0x41). It returns `{"host": <the host's answer>}`, or
;; `{"status":-N}` when the host call fails. The wrapper keeps a host `{"error"}` answer from being
;; read as the plugin's own error response.
(module
  (import "craft_host" "call" (func $host (param i32 i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1 16)
  (global $heap (mut i32) (i32.const 4096))
  (data (i32.const 64) "{\"status\":-0}")
  (data (i32.const 32768) "{\"host\":")

  (func $alloc (param $len i32) (param $align i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (if (i32.gt_u (global.get $heap) (i32.const 30000)) (then (global.set $heap (local.get $p)) (return (i32.const 0))))
    (local.get $p))

  (func (export "craft_abi") (result i32) i32.const 1)
  (export "craft_alloc" (func $alloc))
  (func (export "craft_free") (param $p i32) (param $len i32) (param $align i32))

  (func (export "craft_invoke") (param $e i32) (param $rp i32) (param $rl i32) (param $bp i32) (param $bl i32) (result i64)
    (local $r i64) (local $st i32) (local $out i32)
    (local.set $r (call $host
      (if (result i32) (i32.eq (local.get $e) (i32.const 0)) (then (i32.const 64)) (else (i32.const 65)))
      (local.get $rp) (local.get $rl) (i32.const 32776) (i32.const 16000)))
    (local.set $st (i32.wrap_i64 (local.get $r)))
    (if (i32.ne (local.get $st) (i32.const 0)) (then
      (local.set $out (call $alloc (i32.const 13) (i32.const 1)))
      (memory.copy (local.get $out) (i32.const 64) (i32.const 13))
      (i32.store8 (i32.add (local.get $out) (i32.const 11)) (i32.add (i32.const 48) (i32.sub (i32.const 0) (local.get $st))))
      (return (i64.or (i64.shl (i64.const 13) (i64.const 32)) (i64.extend_i32_u (local.get $out))))))
    (local.set $st (i32.wrap_i64 (i64.shr_u (local.get $r) (i64.const 32))))
    (i32.store8 (i32.add (i32.const 32776) (local.get $st)) (i32.const 125))
    (i64.or (i64.shl (i64.extend_i32_u (i32.add (local.get $st) (i32.const 9))) (i64.const 32)) (i64.const 32768)))
)
