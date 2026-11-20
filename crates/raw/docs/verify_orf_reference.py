#!/usr/bin/env python3
"""Recheck pinned ORF sensor hashes with an explicitly selected black-box executable.

Uses Python 3.11+ and its standard library. No reference source, camera matrix or
rendered image is read. Raw inputs remain unchanged; temporary PGM outputs are
removed, including on failure. This tool never updates the pinned hashes.
"""

import argparse
import array
import hashlib
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile


CASES = pathlib.Path(__file__).resolve().parents[1] / "tests/orf_corpus.rs"
CASE = re.compile(
    r'\s*\(\s*"([A-Za-z0-9_.-]+\.orf)",\s*"([0-9a-f]{64})",\s*'
    r'\((\d+),\s*(\d+)\),\s*"([0-9a-f]{64})",?\s*\)\s*,'
)


def parse_cases(path):
    """Read every fixture tuple; never silently skip a malformed reference."""
    manifest = re.search(r"const CASES:.*?=\s*&\[(.*?)\n\];", path.read_text(encoding="utf-8"), re.DOTALL)
    if not manifest:
        raise ValueError(f"{path.name}: cannot find the fixture array")
    text = manifest.group(1)
    cases = [(name, digest, int(width), int(height), sensor)
             for name, digest, width, height, sensor in CASE.findall(text)]
    if not cases or CASE.sub("", text).strip():
        raise ValueError(f"{path.name}: cannot read every fixture tuple")
    if len({case[0] for case in cases}) != len(cases) or any(w == 0 or h == 0 for _, _, w, h, _ in cases):
        raise ValueError(f"{path.name}: duplicate fixture or empty sensor dimensions")
    return cases


def sensor_hash(path, width, height):
    """Hash an unscaled PGM sensor as row-major little-endian u16, in bounded blocks."""
    if width <= 0 or height <= 0:
        raise ValueError(f"{path.name}: empty sensor dimensions")
    with path.open("rb") as stream:
        if stream.readline(4) != b"P5\n":
            raise ValueError(f"{path.name}: expected binary PGM")
        fields = []
        header_bytes = 0
        while len(fields) < 3:
            line = stream.readline(4096 - header_bytes)
            header_bytes += len(line)
            if not line or header_bytes >= 4096:
                raise ValueError(f"{path.name}: invalid PGM header")
            fields.extend(line.split(b"#", 1)[0].split())
        if fields != [str(width).encode(), str(height).encode(), b"65535"]:
            raise ValueError(f"{path.name}: unexpected dimensions or scaling: {fields}")
        digest = hashlib.sha256()
        remaining = width * height * 2
        while remaining:
            block = stream.read(min(remaining, 1024 * 1024))
            if not block or len(block) % 2:
                raise ValueError(f"{path.name}: truncated sensor")
            # PGM samples are big-endian; the fixture hashes use little-endian.
            samples = array.array("H")
            samples.frombytes(block)
            samples.byteswap()
            digest.update(samples.tobytes())
            remaining -= len(block)
        if stream.read(1):
            raise ValueError(f"{path.name}: unexpected trailing sensor bytes")
        return digest.hexdigest()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=pathlib.Path, required=True, help="directory containing raw/ fixtures")
    parser.add_argument("--oracle", required=True, help="installed unprocessed_raw executable (name or path)")
    args = parser.parse_args(argv)
    cases = parse_cases(CASES)
    oracle = shutil.which(args.oracle)
    if not oracle:
        parser.error("--oracle must name an installed unprocessed_raw executable")
    # Extraction changes cwd; resolve relative executable paths and PATH entries first.
    oracle = pathlib.Path(oracle).resolve(strict=True)
    subprocess.run([oracle], check=True)  # Record the executable's version/usage with the results.
    with tempfile.TemporaryDirectory(prefix="craft-orf-reference-") as tmp:
        scratch = pathlib.Path(tmp)
        for name, input_hash, width, height, expected in cases:
            source = (args.corpus / "raw" / name).resolve(strict=True)
            with source.open("rb") as stream:
                actual = hashlib.file_digest(stream, "sha256").hexdigest()
            if actual != input_hash:
                raise ValueError(f"{name}: input SHA-256 differs")
            local = scratch / name
            try:
                local.symlink_to(source)
            except OSError:
                shutil.copyfile(source, local)
            subprocess.run([oracle, "-q", str(local)], cwd=scratch, check=True)
            pgm = pathlib.Path(str(local) + ".pgm")
            actual = sensor_hash(pgm, width, height)
            if actual != expected:
                raise ValueError(f"{name}: sensor SHA-256 {actual} differs from {expected}")
            print(f"{name}: {width}x{height}, input and full sensor SHA-256 match", flush=True)
            pgm.unlink()
            local.unlink()
    print(f"All {len(cases)} reference sensors match. No fixture hashes changed.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
