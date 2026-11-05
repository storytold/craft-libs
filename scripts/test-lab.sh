#!/usr/bin/env bash
# Runtime crate fmt/tests. Needs cargo on a Linux host.
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
echo "craft-libs runtime tests OK"
