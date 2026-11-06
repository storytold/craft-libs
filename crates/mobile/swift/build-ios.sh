#!/bin/sh
# Build a Craft app for iPadOS with CraftKit: the app's Rust static library, CraftKit and the
# app's own Swift, linked into an .app bundle.
#
#   build-ios.sh <cargo package> <app dir with Info.plist and *.swift> <bundle name> [sim|device]
#
# Run from the app's Cargo workspace. CARGO_TARGET_DIR is honoured.
set -eu
package=$1; app_dir=$(cd "$2" && pwd); name=$3; kind=${4:-sim}
kit=$(cd "$(dirname "$0")/CraftKit" && pwd)
case $kind in
  sim) rust=aarch64-apple-ios-sim; sdk=iphonesimulator; triple=arm64-apple-ios26.0-simulator ;;
  device) rust=aarch64-apple-ios; sdk=iphoneos; triple=arm64-apple-ios26.0 ;;
  *) echo "usage: $0 <package> <app dir> <name> [sim|device]" >&2; exit 2 ;;
esac
# C code in dependencies (ring, ...) must target the same minimum iOS as the Swift.
export IPHONEOS_DEPLOYMENT_TARGET=26.0
target_dir=${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')}
cargo build --release -p "$package" --target "$rust"
lib=$target_dir/$rust/release/lib$(echo "$package" | tr - _).a
bundle=$target_dir/$rust/release/$name.app
rm -rf "$bundle" && mkdir -p "$bundle"
# The static archive by path: -l would pick the cdylib built beside it.
xcrun --sdk "$sdk" swiftc -O -target "$triple" -parse-as-library -module-name "$name" \
  -import-objc-header "$kit/include/CraftMobile.h" "$kit"/Sources/*.swift "$app_dir"/*.swift \
  "$lib" -lc++ \
  -framework SwiftUI -framework UIKit -framework Metal -framework QuartzCore -framework CoreGraphics -framework CoreText \
  -framework Foundation -framework Security -framework SystemConfiguration -framework UniformTypeIdentifiers -framework Photos -framework PhotosUI \
  -o "$bundle/$name"
cp "$app_dir/Info.plist" "$bundle/"
codesign --force --sign - "$bundle"
echo "$bundle"
