#!/usr/bin/env python3
"""Fetch the pinned, licensed PDF test fixtures outside the checkout."""

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
            with urllib.request.urlopen(url, timeout=60) as response:
                return response.read(16 * 1024 * 1024 + 1)
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
    raise RuntimeError("fixture download retries exhausted")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    destination = args.destination.resolve()
    if destination == root or root in destination.parents:
        parser.error("fixtures must be stored outside the checkout")
    fixtures = json.loads((root / "crates/pdf/tests/fonts.json").read_text())
    destination.mkdir(parents=True, exist_ok=True)
    for fixture in fixtures:
        name, url, digest = fixture["name"], fixture["source"], fixture["sha256"]
        if Path(name).name != name or name in ("", ".", "..") or not url.startswith("https://"):
            raise ValueError("invalid fixture name or source")
        path = (destination / name).resolve()
        if path.parent != destination:
            raise ValueError(f"fixture path escapes destination: {name}")
        if path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == digest:
            print(f"Verified {name}")
            continue
        data = fetch(url)
        if len(data) > 16 * 1024 * 1024 or hashlib.sha256(data).hexdigest() != digest:
            raise ValueError(f"fixture size or SHA-256 mismatch: {name}")
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
