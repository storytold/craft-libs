#!/usr/bin/env python3
"""Measure LightCraft's raw camera coverage (moved from LightCraft's docs/raw/ with the decoders; its paths
and "LightCraft" mean this crate now) against Adobe's supported-camera list.

    python3 crates/raw/docs/coverage/coverage.py            # print the summary, rewrite docs/raw/coverage.tsv
    python3 crates/raw/docs/coverage/coverage.py --check    # exit 1 if coverage.tsv is stale

Inputs (all in this directory):
  adobe-cameras.tsv   Adobe's published list (maker, camera, extensions, ...).
  evidence.tsv        per-model evidence: verified decodes, known preview-only or refused bodies.
  usage-weights.tsv   estimated share of photographers per maker bucket and popular bodies.

Every Adobe-listed model gets one class:
  verified     decoded from sensor data, and a real file from this body was checked (evidence.tsv)
  unverified   the decoder path for its format and coding should handle it; no sample checked
  preview      opens as the embedded JPEG (format or coding not decoded)
  unsupported  not imported, refused, or decodes wrongly

The format rules below mirror the decoders in crates/raw/src (see docs/raw-parity.md). When a decoder
changes, update the rules and evidence, rerun, and commit coverage.tsv with the docs.
Standard library only; no third-party packages.
"""

import csv
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# Olympus ORF bodies that write uncompressed / packed data (crates/raw/src/vendor/orf.rs). Everything else
# writes compressed ORF, which is preview-only (the E-M5 II / PEN-F high-res mode alone is decoded).
ORF_DECODED = re.compile(r"^(E-1|E-10|E-20|EVOLT E-300|EVOLT E-330|EVOLT E-400|EVOLT E-500|E-300|E-330|E-500|Camedia C-\S.*|Stylus XZ-2 iHS)$")
# Samsung SRW bodies with compressed raw data (crates/raw/src/vendor/srw.rs refuses 32770 / 32772 / 32773).
SRW_COMPRESSED = re.compile(r"(^|\W)(NX1|NX30|NX300|NX300M|NX500|NX2000|NX3000|NX3300|NX mini|EK-GN\d+|EK-KN\d+)(\W|$)")

# CIFF-container CRW bodies (crates/raw/src/vendor/crw.rs): every CRW EOS and the CIFF PowerShots
# (the G and S series and the Pro 1).
CRW_DECODED = re.compile(r"^EOS |^PowerShot (G[1-6]$|Pro ?1$|S[34567]0$|S45$)")
# Fujifilm bodies before the X series whose RAF has no raw IFD (crates/raw/src/vendor/raf.rs).
RAF_OLD = re.compile(r"^(FinePix (E|F|S\d|S\d{3,4}|IS)|IS Pro|S5Pro|S3Pro|S2Pro)", re.I)

# Probability that an unverified body of a format decodes correctly, used for the use-weighted share only.
CONFIDENCE = {"DNG": 0.95, "CR2": 0.95, "CR3": 0.6, "NEF": 0.9, "NRW": 0.9, "ARW": 0.9, "SR2": 0.8, "RAF": 0.95,
              "RW2": 0.95, "RWL": 0.95, "RAW": 0.95, "PEF": 0.9, "ORF": 0.8, "SRW": 0.8}


def read_tsv(name):
    with open(os.path.join(HERE, name), newline="", encoding="utf-8") as f:
        lines = [line for line in f if not line.startswith("#")]
    return list(csv.DictReader(lines, delimiter="\t"))


def exts(field):
    return [e.strip().upper() for e in re.split(r"[,/+]", re.sub(r"\(.*?\)", "", field)) if e.strip()]


def classify(maker, camera, ext_field):
    """(class, format, reason) from the format rules alone."""
    es = exts(ext_field)
    for e in es:
        if e == "DNG":
            return "unverified", "DNG", "DNG decoder (uncompressed, LJ92, lossy, Deflate, JPEG XL, LinearRaw)"
        if e == "CR2":
            return "unverified", "CR2", "CR2 lossless JPEG decoder"
        if e == "CR3":
            return "unverified", "CR3", "CRX decoder: lossless and C-RAW 0x100 / 0x200 single-tile; other variants fall back to the preview"
        if e in ("NEF", "NRW"):
            return "unverified", e, "NEF decoder (uncompressed, lossless, lossy, lossy after split); HE / HE* setting preview-only"
        if e == "ARW":
            return "unverified", "ARW", "ARW decoder (uncompressed, cRAW, packed 12-bit, lossless, downsized YCbCr)"
        if e == "SR2":
            return "unverified", "SR2", "SR2 decoder"
        if e == "SRF":
            return "unsupported", "SRF", "SRF is discovered; sensor decoding unsupported, usable preview not verified on a real file"
        if e == "RAF":
            if RAF_OLD.search(camera):
                return "preview", "RAF", "pre-X FinePix RAF without a raw IFD"
            return "unverified", "RAF", "RAF decoder (uncompressed, lossless / lossy compressed, Bayer and X-Trans)"
        if e in ("RW2", "RWL") or (e == "RAW" and maker in ("Panasonic", "Leica")):
            return "unverified", e, "Panasonic decoder, every raw format"
        if e == "PEF":
            return "unverified", "PEF", "PEF decoder (uncompressed, Huffman, packed 12-bit)"
        if e == "ORF":
            if ORF_DECODED.search(camera):
                return "unverified", "ORF", "uncompressed / packed ORF"
            return "preview", "ORF", "compressed ORF is unsupported; embedded preview when available"
        if e == "SRW":
            if SRW_COMPRESSED.search(camera):
                return "preview", "SRW", "compressed SRW is not decoded"
            return "unverified", "SRW", "uncompressed / packed SRW"
    e = es[0] if es else ""
    if e == "MEF":
        return "unverified", "MEF", "MEF decoder (uncompressed 12-bit)"
    if e == "ERF":
        return "unverified", "ERF", "ERF decoder (12-bit 16-byte blocks)"
    if e == "CRW":
        # CIFF-era bodies decode (craft-libs crates/crw/clean-room/02-crw-format.md): EOS D30/D60/10D/300D and
        # the CIFF PowerShots; the pre-CIFF compacts (PowerShot 600, A5, A50, Pro70) and the
        # unprobed Pro90 IS stay preview. The G5 is preview by evidence (entropy anomaly).
        if CRW_DECODED.search(camera):
            return "unverified", "CRW", "CRW decoder (CIFF; selector 0-2 tables)"
        return "preview", "CRW", "pre-CIFF CRW is not decoded"
    if e == "MRW":
        return "unverified", "MRW", "MRW decoder (12-bit packed, 16-bit words)"
    if e in ("3FR", "FFF", "IIQ", "MOS", "KDC", "X3F", "RAW", "ORI"):
        return "preview", e, f"{e} is opened from its embedded JPEG only"
    return "unsupported", e, f"{e or 'unknown'} is not imported"


def main():
    adobe = read_tsv("adobe-cameras.tsv")
    evidence = {(r["maker"], r["camera"]): r for r in read_tsv("evidence.tsv")}
    rows = []
    for a in adobe:
        cls, fmt, reason = classify(a["maker"], a["camera"], a["extensions"])
        ev = evidence.get((a["maker"], a["camera"]))
        if ev:
            cls, reason = ev["status"], ev["source"]
        rows.append({"maker": a["maker"], "camera": a["camera"], "format": fmt, "year_added": a["year_added"], "class": cls, "reason": reason})

    out = os.path.join(HERE, "coverage.tsv")
    header = ["maker", "camera", "format", "year_added", "class", "reason"]
    text = "# Generated by docs/raw/coverage.py; do not edit.\n" + "\t".join(header) + "\n"
    text += "".join("\t".join(r[k] for k in header) + "\n" for r in rows)
    if "--check" in sys.argv:
        with open(out, encoding="utf-8") as f:
            if f.read() != text:
                print("docs/raw/coverage.tsv is stale: run python3 crates/raw/docs/coverage/coverage.py")
                return 1
    else:
        with open(out, "w", encoding="utf-8") as f:
            f.write(text)

    classes = ["verified", "unverified", "preview", "unsupported"]
    by_maker = {}
    for r in rows:
        by_maker.setdefault(r["maker"], dict.fromkeys(classes, 0))[r["class"]] += 1
    print("| Maker | Adobe models | verified | unverified | preview | unsupported |")
    print("|---|---:|---:|---:|---:|---:|")
    for m, c in sorted(by_maker.items(), key=lambda kv: -sum(kv[1].values())):
        print(f"| {m} | {sum(c.values())} | " + " | ".join(str(c[k]) for k in classes) + " |")
    tot = {k: sum(c[k] for c in by_maker.values()) for k in classes}
    n = sum(tot.values())
    print(f"| **All** | **{n}** | " + " | ".join(f"**{tot[k]}** ({100 * tot[k] / n:.0f}%)" for k in classes) + " |")
    recent = [r for r in rows if r["year_added"] and int(r["year_added"]) >= 2017]
    rc = {k: sum(1 for r in recent if r["class"] == k) for k in classes}
    print(f"\nModels added since 2017 (ACR 10+): {len(recent)}: " + ", ".join(f"{k} {rc[k]}" for k in classes))

    # Use-weighted share of photographers whose camera fully works (decodes from sensor data).
    weights = read_tsv("usage-weights.tsv")
    by_cam = {(r["maker"], r["camera"]): r for r in rows}
    total = works = verified_share = 0.0
    print("\n| Bucket | Weight | Camera fully works | of which verified bodies |")
    print("|---|---:|---:|---:|")
    for b in [w for w in weights if w["kind"] == "maker"]:
        makers = b["makers_or_camera"].split(",")
        popular = [w["makers_or_camera"] for w in weights if w["kind"] == "body" and w["bucket"] == b["bucket"]]
        members = [r for r in rows if r["maker"] in makers]
        pop_rows = [r for m in makers for (mk, cam), r in by_cam.items() if mk == m and cam in popular]
        missing = set(popular) - {r["camera"] for r in pop_rows}
        if missing:
            print(f"usage-weights.tsv: unknown cameras {sorted(missing)}", file=sys.stderr)
            return 1
        rest = [r for r in members if r["camera"] not in popular]

        def score(r):
            if r["class"] == "verified":
                return 1.0
            if r["class"] == "unverified":
                return CONFIDENCE.get(r["format"], 0.7)
            return 0.0

        def mean(rs, f):
            return sum(f(r) for r in rs) / len(rs) if rs else 0.0

        if pop_rows:
            s = 0.8 * mean(pop_rows, score) + 0.2 * mean(rest, score)
            v = 0.8 * mean(pop_rows, lambda r: r["class"] == "verified") + 0.2 * mean(rest, lambda r: r["class"] == "verified")
        else:
            s, v = mean(members, score), mean(members, lambda r: r["class"] == "verified")
        w = float(b["weight"])
        total += w
        works += w * s
        verified_share += w * v
        print(f"| {b['bucket']} | {w:g}% | {100 * s:.0f}% | {100 * v:.0f}% |")
    print(f"| **Weighted** | {total:g}% | **{100 * works / total:.0f}%** | **{100 * verified_share / total:.0f}%** |")
    return 0


if __name__ == "__main__":
    sys.exit(main())
