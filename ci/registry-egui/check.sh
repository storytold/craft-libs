#!/bin/sh
# Migration gates for craft-libs (docs/egui-037-migration-plan.md §5.2 and CL-4):
#   1. craft-ui (default features, by path, no patch) compiles, passes clippy and tests
#      against stock crates.io egui 0.37.0, with exactly one egui source in the graph.
#   2. `cargo package -p craft-ui` produces a .crate whose manifest carries no
#      machine-local path, no git branch pin and no hidden winit/egui git source.
#   3. craft-ui --all-features compiles for wasm32-unknown-unknown (compile-only gate;
#      the browser IME/input matrix stays on the manual release list, plan E10).
#
# Run from anywhere: ./ci/registry-egui/check.sh   (honours an externally set CARGO_TARGET_DIR)
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"

echo "== gate 1: craft-ui default features against stock crates.io egui 0.37.0 =="
cd "$here"
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo tree -i egui > /tmp/registry-egui-tree.txt
cat /tmp/registry-egui-tree.txt
# With a single source cargo prints no source annotation; a git or path source would show
# as `egui v0.37.0 (git+…)` / `egui v0.37.0 (/path)`.
if ! grep -qx 'egui v0.37.0' /tmp/registry-egui-tree.txt; then
    echo "FAIL: egui is not the single stock crates.io 0.37.0 source:" >&2
    cat /tmp/registry-egui-tree.txt >&2
    exit 1
fi

echo "== gate 2: cargo package -p craft-ui smoke =="
# The internal crates are unpublished, and `cargo package`'s upload-prepare resolve
# simulates a clean consumer (it ignores [patch]), so stage a minimal workspace, vendor
# the whole graph (adding the two local crates to the directory source by hand) and let
# crates-io source replacement satisfy that resolve. Packaging then normalizes the
# manifest and strips paths; the checks below fail the gate on any residue.
stage=$(mktemp -d)
trap 'rm -rf "$stage" /tmp/registry-egui-tree.txt' EXIT
cp -r "$root/crates/ui" "$root/crates/i18n" "$root/crates/fonts-build" "$stage/"
cat > "$stage/Cargo.toml" <<'EOF'
[workspace]
resolver = "3"
members = ["ui", "i18n", "fonts-build", "fonts-build/tests/consumer"]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"
rust-version = "1.95"
repository = "https://github.com/storytold/craft-libs"

[workspace.dependencies]
craft-fonts-build = { path = "fonts-build", version = "=0.1.0" }
craft-i18n = { path = "i18n", version = "=0.1.0" }
egui = { version = "0.37", default-features = false }

[workspace.lints.rust]
unsafe_code = "forbid"

[patch.crates-io]
craft-fonts-build = { path = "fonts-build" }
craft-i18n = { path = "i18n" }
EOF
cd "$stage"
cargo vendor vendor >/dev/null 2>&1
# cargo vendor skips path-patched crates; add them to the directory source manually.
for c in i18n fonts-build; do
    cp -r "$c" "vendor/craft-$c"
    printf '{"files":{}}' > "vendor/craft-$c/.cargo-checksum.json"
done
mkdir -p .cargo
printf '[source.crates-io]\nreplace-with = "vendored-sources"\n\n[source.vendored-sources]\ndirectory = "vendor"\n' > .cargo/config.toml
CARGO_TARGET_DIR="$stage/target" cargo package -p craft-ui --no-verify --allow-dirty >/dev/null
crate=$(find "$stage/target/package" -name 'craft-ui-*.crate' | head -1)
[ -n "$crate" ] || { echo "FAIL: no .crate produced" >&2; exit 1; }
manifest=$(tar -xzOf "$crate" --wildcards '*/Cargo.toml')
echo "$manifest" | grep -E '^name = |^version = ' | head -2
# Dependency entries must be registry versions: no git source, no branch pin, no absolute
# or escaping path (target paths like `path = "src/lib.rs"` are package-internal and fine).
if echo "$manifest" | grep -nE 'git \+|branch *=|path = "(/|\.\.|~)|/home/|/Users/'; then
    echo "FAIL: packaged manifest carries a git source, branch pin or machine-local path" >&2
    exit 1
fi
if echo "$manifest" | grep -q 'winit'; then
    echo "FAIL: packaged manifest hides a winit dependency" >&2
    exit 1
fi
if ! echo "$manifest" | grep -q '\[dependencies.craft-i18n\]'; then
    echo "FAIL: packaged manifest lost the craft-i18n dependency" >&2
    exit 1
fi
echo "packaged manifest is clean: registry versions only, no paths, no git, no winit"

echo "== gate 3: wasm32 check of craft-ui --all-features =="
cd "$root"
cargo check -p craft-ui --all-features --locked --target wasm32-unknown-unknown

echo "all migration gates passed"
