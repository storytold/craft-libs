# craft-dwg

A byte-level bridge between DWG and DXF. It does not expose `acadrust` types: callers pass bytes,
choose limits, and receive converted bytes plus notes about data the conversion could not preserve.
Application document mapping remains with the caller.

The default limits reject DWG inputs over 512 MiB and stop DXF output over 1 GiB. The bridge
also catches panics from the dependency and validates its conversion diagnostics. Because a small
hostile file can declare very large internal sections, size checks alone cannot prevent an
allocation failure inside the decoder. On native applications, call `use_own_worker()` during
startup and call `serve_if_asked()` before normal argument handling. Subsequent `decode` and
`encode_*` calls then run in a child process with a 30-minute timeout; a crash or stack overflow
is contained to that child. The worker has no operating-system memory limit yet. Without a
configured worker, conversions run in the current process.

The bridge is not built for `wasm32` or FreeBSD/aarch64. Those targets retain the public API and
return a clear unsupported-build error. The `acadrust` 0.6.6 dependency is used unmodified under
MPL-2.0; its license and third-party notices are in `licenses/dwg/` at the workspace root.
