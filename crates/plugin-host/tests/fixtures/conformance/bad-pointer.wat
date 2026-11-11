;; Conformance plugin `bad-pointer`. craft_invoke(entry, ...) makes one host call (host.log, 0x1)
;; with pointers the guest has no right to use, and returns the packed host answer:
;;   0 request range starts at 0xFFFFFFF0          1 output range starts at 0xFFFFFFF0
;;   2 request at 0xFFFFFFFF with length 2 (the end wraps past 32 bits)
;;   3 output capacity of 0xFFFFFFFF               4 control: valid ranges, request "{}"
;;   5 returns the pointer pair len<<32|ptr for a block outside memory, without a host call
;; Entries 0 to 3 get E_BAD_ARG from the host call body. The guest keeps running.
(module
  (import "craft_host" "call" (func $host (param i32 i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1 16)
  (data (i32.const 64) "{}")
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param $entry i32) (param i32 i32 i32 i32) (result i64)
    (if (i32.eq (local.get $entry) (i32.const 0))
      (then (return (call $host (i32.const 1) (i32.const 0xFFFFFFF0) (i32.const 64) (i32.const 1024) (i32.const 64)))))
    (if (i32.eq (local.get $entry) (i32.const 1))
      (then (return (call $host (i32.const 1) (i32.const 64) (i32.const 2) (i32.const 0xFFFFFFF0) (i32.const 64)))))
    (if (i32.eq (local.get $entry) (i32.const 2))
      (then (return (call $host (i32.const 1) (i32.const 0xFFFFFFFF) (i32.const 2) (i32.const 1024) (i32.const 64)))))
    (if (i32.eq (local.get $entry) (i32.const 3))
      (then (return (call $host (i32.const 1) (i32.const 64) (i32.const 2) (i32.const 1024) (i32.const 0xFFFFFFFF)))))
    (if (i32.eq (local.get $entry) (i32.const 4))
      (then (return (call $host (i32.const 1) (i32.const 64) (i32.const 2) (i32.const 1024) (i32.const 64)))))
    (i64.or (i64.shl (i64.const 4096) (i64.const 32)) (i64.const 0xFFFFFF00))))
