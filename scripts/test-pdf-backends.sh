#!/usr/bin/env bash
# Run transferred backend tests with the complete local patch set. Test copies
# keep Cargo's standalone lock resolution from modifying the vendored source.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/craft-pdf-backends.XXXXXX")
trap 'rm -rf "$test_root"' EXIT
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
patches=()
for backend in hayro hayro-interpret hayro-syntax hayro-jbig2 hayro-cmap hayro-postscript; do
  cp -R "$root/vendor/$backend" "$test_root/$backend"
  patches+=(--config "patch.crates-io.$backend.path=\"$test_root/$backend\"")
done
for backend in hayro-syntax hayro-interpret hayro hayro-jbig2 hayro-cmap hayro-postscript; do
  cargo test --manifest-path "$test_root/$backend/Cargo.toml" --lib "${patches[@]}"
done
cargo test --manifest-path "$test_root/hayro-syntax/Cargo.toml" --doc "${patches[@]}"
cargo test --manifest-path "$test_root/hayro-syntax/Cargo.toml" --lib --no-default-features --features std "${patches[@]}"
