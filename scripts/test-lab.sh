#!/usr/bin/env bash
# Run on a lab Linux builder (192.168.1.88 vinez or 192.168.1.57 ai-node-02), not the laptop.
# Usage: ./scripts/test-lab.sh [repo-root]
set -euo pipefail
ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$ROOT"

PACKAGES=(
  craft-cloud
  craft-gpu
  craft-gpu-null
  craft-gpu-providers
  craft-gpu-wgpu
  craft-tier-store
  craft-worker-host
  craft-runtime
)

cargo fmt --check
for pkg in "${PACKAGES[@]}"; do
  cargo test -p "$pkg"
done
cargo test -p craft-runtime --features web,shared-worker
node tests/worker-host-protocol.mjs
node tests/opfs-flush.mjs
node tests/fs-access-host.mjs
echo "craft-libs runtime tests OK"
