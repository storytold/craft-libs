#!/usr/bin/env bash
# Run tests/web.rs in headless Chromium through wasm-bindgen-test. Not part of CI.
#
# Needs a wasm-bindgen-test-runner whose version equals the wasm-bindgen in Cargo.lock, and a
# chromedriver that matches the installed Chromium or Chrome:
#   cargo install wasm-bindgen-cli --version <lock version> --locked
# Extra arguments go to the test binary, for example a test name filter.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner
export CHROMEDRIVER="${CHROMEDRIVER:-$(command -v chromedriver)}"
export WASM_BINDGEN_TEST_TIMEOUT="${WASM_BINDGEN_TEST_TIMEOUT:-300}"

locked="$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/[^0-9.]/,"",$3); print $3; exit}' "$root/Cargo.lock")"
have="$(wasm-bindgen-test-runner --version | awk '{print $2}')"
if [ "$locked" != "$have" ]; then
    echo "wasm-bindgen-test-runner is $have but Cargo.lock has wasm-bindgen $locked" >&2
    exit 1
fi

cd "$root"
exec cargo test -p craft-plugin-host --target wasm32-unknown-unknown --features browser --test web -- "$@"
