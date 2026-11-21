;; A v1 PhotoCraft package. The loader must send it to its shim, not instantiate it.
(module
  (memory (export "memory") 1 16)
  (func (export "pc_abi_version") (result i32) i32.const 1))
