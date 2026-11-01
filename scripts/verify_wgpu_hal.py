"""Verify the vendored wgpu-hal against the published 30.0.1 crate."""

import difflib
import hashlib
import pathlib
import sys
import tarfile


ARCHIVE_SHA256 = "b6b7fb58561a792bc237628ba0792e332de418fefe145f13b5ed8201e6d52f58"
SOURCE = "src/dx12/mod.rs"
EXTRA_FILES = {"CRAFT-PATCH.md", "dx12-window-scaling.patch"}
ROOT = pathlib.Path(__file__).resolve().parent.parent / "vendor" / "wgpu-hal"


def normalized(data: bytes) -> bytes:
    """Ignore only line endings changed by a Windows Git checkout."""
    return data.replace(b"\r\n", b"\n")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: python scripts/verify_wgpu_hal.py PATH/TO/wgpu-hal-30.0.1.crate")
        return 2

    archive = pathlib.Path(sys.argv[1])
    if hashlib.sha256(archive.read_bytes()).hexdigest() != ARCHIVE_SHA256:
        print("The crate archive SHA-256 differs from the recorded 30.0.1 source")
        return 1

    with tarfile.open(archive, "r:gz") as package:
        upstream = {
            pathlib.PurePosixPath(member.name).relative_to("wgpu-hal-30.0.1").as_posix(): normalized(
                package.extractfile(member).read()
            )
            for member in package.getmembers()
            if member.isfile()
        }

    local = {
        path.relative_to(ROOT).as_posix(): normalized(path.read_bytes())
        for path in ROOT.rglob("*")
        if path.is_file() and path.relative_to(ROOT).as_posix() not in EXTRA_FILES
    }
    if upstream.keys() != local.keys():
        print("Archive and vendor file lists differ")
        print("Only in archive:", sorted(upstream.keys() - local.keys()))
        print("Only in vendor:", sorted(local.keys() - upstream.keys()))
        return 1

    changed = {name for name in upstream if upstream[name] != local[name]}
    if changed != {SOURCE}:
        print("Unexpected changed files:", sorted(changed))
        return 1

    expected_patch = "".join(
        difflib.unified_diff(
            upstream[SOURCE].decode().splitlines(keepends=True),
            local[SOURCE].decode().splitlines(keepends=True),
            fromfile=f"a/{SOURCE}",
            tofile=f"b/{SOURCE}",
        )
    )
    if expected_patch != (ROOT / "dx12-window-scaling.patch").read_text(encoding="utf-8"):
        print("The DX12 source differs from the recorded patch")
        return 1

    print(f"Verified {len(upstream)} upstream files and the single DX12 patch")
    return 0


if __name__ == "__main__":
    sys.exit(main())
