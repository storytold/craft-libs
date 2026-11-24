#!/usr/bin/env bash
# Separate every plate-test scene with Ghostscript (local only, never CI) and judge the plates.
# With --flatten, also flatten each scene with craft-prepress's flattener and judge that page.
# Ghostscript is `gs` on PATH, or CRAFT_PLATE_TEST_GS.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
fonts="${XDG_CACHE_HOME:-$HOME/.cache}/craft-pdf-test-fonts"
if [[ -z "${CRAFT_PDF_TEST_FONT_DIR:-}" && -f "$fonts/Inter-Regular.ttf" ]]; then
  export CRAFT_PDF_TEST_FONT_DIR="$fonts"
fi
cd "$root"
case "${1:-}" in
  "")
    cargo test -p craft-plate-test --test ghostscript -- --ignored --nocapture
    echo "report: ${CARGO_TARGET_DIR:-target}/plate-test/report.md"
    ;;
  --flatten)
    cargo test -p craft-prepress --features flatten --test flatten_plates -- --ignored --nocapture
    echo "report: ${CARGO_TARGET_DIR:-target}/plate-test/flattened/report.md"
    ;;
  *)
    echo "usage: $0 [--flatten]" >&2
    exit 2
    ;;
esac
