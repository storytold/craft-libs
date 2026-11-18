#!/usr/bin/env python3
"""Fetch the pinned CC0 Canon CRW sample files for craft-crw's tests, outside the checkout.

The manifest is crates/crw/tests/samples.json (name, raw.pixls.us URL, SHA-256, licence). Files
already present with the right hash are kept. Then run the tests with
`CRAFT_CRW_SAMPLES=<destination> cargo test -p craft-crw --release --test samples`.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

MAX_BYTES = 16 * 1024 * 1024


class _HttpsOnlyRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urllib.parse.urlsplit(newurl).scheme != "https":
            raise ValueError("refusing to follow a non-HTTPS redirect")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


urllib.request.install_opener(urllib.request.build_opener(_HttpsOnlyRedirectHandler))


def fetch(url):
    if urllib.parse.urlsplit(url).scheme != "https":
        raise ValueError("refusing to fetch a non-HTTPS URL")
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url, timeout=120) as response:
                return response.read(MAX_BYTES + 1)
        except urllib.error.HTTPError as error:
            if error.code not in (408, 429) and not 500 <= error.code <= 599:
                raise
            if attempt == 2:
                raise
            error.close()
        except (urllib.error.URLError, TimeoutError, ConnectionError):
            if attempt == 2:
                raise
        time.sleep(attempt + 1)
    raise RuntimeError("sample download retries exhausted")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    destination = args.destination.resolve()
    if destination == root or root in destination.parents:
        parser.error("sample files must be stored outside the checkout (they are never committed)")
    samples = json.loads((root / "crates/crw/tests/samples.json").read_text(encoding="utf-8"))
    destination.mkdir(parents=True, exist_ok=True)
    for sample in samples:
        name, url, digest, licence = sample["name"], sample["source"], sample["sha256"], sample["license"]
        if Path(name).name != name or name in ("", ".", "..") or not url.startswith("https://"):
            raise ValueError("invalid sample name or source")
        if licence != "CC0-1.0":
            raise ValueError(f"{name}: only CC0 samples are fetched (manifest says {licence})")
        path = (destination / name).resolve()
        if path.parent != destination:
            raise ValueError(f"sample path escapes destination: {name}")
        if path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == digest:
            print(f"Verified {name}")
            continue
        data = fetch(url)
        if len(data) > MAX_BYTES or hashlib.sha256(data).hexdigest() != digest:
            raise ValueError(f"sample size or SHA-256 mismatch: {name}")
        fd, tmp = tempfile.mkstemp(dir=destination, prefix=f".{name}.", suffix=".tmp")
        try:
            with os.fdopen(fd, "wb") as handle:
                handle.write(data)
            os.replace(tmp, destination / name)
        except BaseException:
            try:
                os.unlink(tmp)
            except OSError:
                pass
            raise
        print(f"Fetched and verified {name}")


if __name__ == "__main__":
    main()
