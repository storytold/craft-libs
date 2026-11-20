#!/usr/bin/env bash
# craft-plugin-host must lint and pass its tests with each feature set that apps use.
# `cargo test --workspace` alone covers one set only, because Cargo unifies features across members.
set -euo pipefail
cd "$(dirname "$0")/../.."

sets=("" "link" "legacy-v1" "link,legacy-v1")
# wasmtime has no wasm32 build, so it only runs on native hosts.
sets+=("wasmtime")

for features in "${sets[@]}"; do
  echo "== craft-plugin-host features: [${features}]"
  cargo clippy -p craft-plugin-host --all-targets --features "${features}" --locked -- -D warnings
  cargo test -p craft-plugin-host --features "${features}" --locked
done

echo "== feature-gated test binaries are absent without their feature"
for test in link legacy_install; do
  if cargo test -p craft-plugin-host --test "${test}" --locked 2>&1 | grep -q "^test result"; then
    echo "test ${test} ran without its feature" >&2
    exit 1
  fi
done
