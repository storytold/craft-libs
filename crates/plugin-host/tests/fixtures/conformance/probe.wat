;; Conformance plugin `probe`. craft_invoke(entry, ...) calls the host op at table slot `entry`
;; with the request "{}" and returns the packed host answer (low 32 bits status, high 32 bits
;; byte count). Slots 0 to 18 are every op of this release in number order. Slot 19 is the
;; reserved op 0x00A0, which no release serves. Any other slot returns -1 without a host call.
(module
  (import "craft_host" "call" (func $host (param i32 i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1 16)
  (data (i32.const 64) "{}")
  (data (i32.const 128)
    "\01\00\00\00" "\02\00\00\00" "\03\00\00\00" "\04\00\00\00"
    "\10\00\00\00" "\11\00\00\00" "\20\00\00\00" "\21\00\00\00"
    "\30\00\00\00" "\40\00\00\00" "\41\00\00\00" "\50\00\00\00"
    "\51\00\00\00" "\60\00\00\00" "\61\00\00\00" "\62\00\00\00"
    "\70\00\00\00" "\80\00\00\00" "\90\00\00\00" "\a0\00\00\00")
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param $slot i32) (param i32 i32 i32 i32) (result i64)
    (if (i32.ge_u (local.get $slot) (i32.const 20)) (then (return (i64.const -1))))
    (call $host
      (i32.load offset=128 (i32.shl (local.get $slot) (i32.const 2)))
      (i32.const 64) (i32.const 2) (i32.const 1024) (i32.const 8192))))
