#!/usr/bin/env bash
# Restore both runtime feature configurations, docs and ordinary example linking, without fuzz caps.
set -euo pipefail
unset CRAFT_HOSTILE_DIVISOR PROPTEST_CASES
if [[ "${RUNNER_OS:-}" = Windows ]]; then
  powershell.exe -NoProfile -Command '$ErrorActionPreference = "Stop"; Start-Service Spooler; Add-PrinterDriver -Name "Generic / Text Only"'
fi
# Windows cannot spawn one rustfmt command with every workspace target (#2200).
if [[ "${RUNNER_OS:-}" = Windows ]]; then
  python scripts/ci-format-workspace.py
else
  cargo fmt --check
fi
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
# cargo test includes docs and ordinary example linking, as required by AGENTS.md.
cargo test --workspace --locked
cargo test --workspace --all-features --locked
