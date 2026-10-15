# Run craft-runtime workspace tests in Linux (Podman). Use when Windows lacks MSVC link.exe.
$ErrorActionPreference = "Stop"
$Podman = "C:\Program Files\RedHat\Podman\podman.exe"
if (-not (Test-Path $Podman)) { $Podman = "podman" }

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Write-Host "Testing craft-libs at $Root"

& $Podman run --rm `
    -v "${Root}:/work:Z" `
    -w /work `
    docker.io/library/rust:bookworm `
    bash -lc @'
set -euo pipefail
export PATH=/usr/local/cargo/bin:$PATH
export CARGO_TARGET_DIR=/tmp/craft-target
rustup component add rustfmt
cargo fmt -p craft-gpu -p craft-tier-store -p craft-worker-host -p craft-runtime \
  -p craft-gpu-null -p craft-gpu-providers -p craft-gpu-wgpu
cargo fmt --check
cargo test -p craft-gpu -p craft-tier-store -p craft-worker-host -p craft-runtime \
  -p craft-gpu-null -p craft-gpu-providers -p craft-gpu-wgpu
cargo test -p craft-runtime --features web,shared-worker
echo ALL RUNTIME TESTS OK
'@
