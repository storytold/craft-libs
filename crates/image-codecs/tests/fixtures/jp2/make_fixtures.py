"""Write the JPEG 2000 interoperability fixtures with OpenJPEG's opj_compress (see README.md)."""

import os
import struct
import subprocess
import sys
import tempfile
import zlib

W = H = 16
OPJ = os.environ.get("OPJ_COMPRESS", "opj_compress")
OUT = os.path.dirname(os.path.abspath(__file__))


def value(c, i, depth):
    """Sample of channel c at pixel i (row-major): a ramp, finer bits varying, each channel a quarter range apart."""
    step = 1 << (depth - 8)
    return (i * step + (i * 37) % step + c * (1 << (depth - 2))) % (1 << depth)


def planes(channels, depth, size=(W, H)):
    w, h = size
    return [[value(c, i, depth) for i in range(w * h)] for c in range(channels)]


def write_pnm(path, channels, depth):
    maxval = (1 << depth) - 1
    fmt = ">H" if depth > 8 else ">B"
    p = planes(channels, depth)
    with open(path, "wb") as f:
        f.write(b"P5" if channels == 1 else b"P6")
        f.write(f"\n{W} {H}\n{maxval}\n".encode())
        f.write(b"".join(struct.pack(fmt, p[c][i]) for i in range(W * H) for c in range(channels)))


def write_png(path, channels, depth):
    p = planes(channels, depth)
    fmt = ">H" if depth > 8 else ">B"
    rows = b"".join(b"\0" + b"".join(struct.pack(fmt, p[c][y * W + x]) for x in range(W) for c in range(channels)) for y in range(H))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    colour_type = {2: 4, 4: 6}[channels]
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, depth, colour_type, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(rows)))
        f.write(chunk(b"IEND", b""))


def write_raw(path, channels, depth, subsampling=None, signed=False):
    """Planar big-endian samples, as opj_compress reads .raw files."""
    sizes = [(W // dx, H // dy) for dx, dy in subsampling] if subsampling else [(W, H)] * channels
    fmt = ">h" if depth > 8 and signed else ">H" if depth > 8 else ">B"
    with open(path, "wb") as f:
        for c, size in enumerate(sizes):
            for v in planes(c + 1, depth, size)[c]:
                f.write(struct.pack(fmt, v - (1 << (depth - 1)) if signed else v))


def opj(src, dst, *args):
    """Encode src to dst; 16x16 allows at most 5 resolution levels, so use 3"""
    subprocess.run([OPJ, "-i", src, "-o", dst, "-n", "3", *args], check=True, stdout=subprocess.DEVNULL)


def boxes(data):
    at = 0
    while at < len(data):
        size, kind = struct.unpack(">I4s", data[at : at + 8])
        yield kind, data[at + 8 : at + size]
        at += size


def pack(items):
    return b"".join(struct.pack(">I4s", len(body) + 8, kind) + body for kind, body in items)


def patch_header(path, edit):
    """Rewrite the jp2h box of the JP2 file at path with edit(list of (type, payload))."""
    with open(path, "rb") as f:
        data = f.read()
    top = [(k, pack(edit(list(boxes(b))))) if k == b"jp2h" else (k, b) for k, b in boxes(data)]
    with open(path, "wb") as f:
        f.write(pack(top))


def replace(kind, body):
    return lambda items: [(k, body if k == kind else b) for k, b in items]


def cdef(entries):
    return struct.pack(">H", len(entries)) + b"".join(struct.pack(">HHH", *e) for e in entries)


def main():
    with tempfile.TemporaryDirectory() as tmp:
        src = lambda name: os.path.join(tmp, name)
        out = lambda name: os.path.join(OUT, name)

        for channels, name in [(1, "gray"), (3, "rgb")]:
            for depth in (8, 16):
                ext = "pgm" if channels == 1 else "ppm"
                write_pnm(src(f"{name}-{depth}.{ext}"), channels, depth)
                opj(src(f"{name}-{depth}.{ext}"), out(f"{name}-{depth}.jp2"))
        for channels, name in [(2, "graya"), (4, "rgba")]:
            for depth in (8, 16):
                write_png(src(f"{name}-{depth}.png"), channels, depth)
                opj(src(f"{name}-{depth}.png"), out(f"{name}-{depth}.jp2"))
        for channels, name in [(1, "gray"), (3, "rgb")]:
            write_raw(src(f"{name}-12.raw"), channels, 12)
            opj(src(f"{name}-12.raw"), out(f"{name}-12.jp2"), "-F", f"{W},{H},{channels},12,u")
        opj(src("gray-12.raw"), out("gray-12.j2k"), "-F", f"{W},{H},1,12,u")
        opj(src("rgba-16.png"), out("rgba-16.j2k"))
        # Four components and no channel definitions: the fourth is read as alpha
        write_raw(src("rgba-16.raw"), 4, 16)
        opj(src("rgba-16.raw"), out("rgba-16-no-cdef.jp2"), "-F", f"{W},{H},4,16,u")

        opj(src("rgb-16.ppm"), out("rgb-16-no-mct.jp2"), "-mct", "0")
        opj(src("rgb-16.ppm"), out("rgb-16-ict.jp2"), "-I")

        # Colour associations swapped: channel 0 is blue and channel 2 red
        opj(src("rgba-16.png"), out("rgba-16-swapped-cdef.jp2"))
        patch_header(out("rgba-16-swapped-cdef.jp2"), replace(b"cdef", cdef([(0, 0, 3), (1, 0, 2), (2, 0, 1), (3, 1, 0)])))
        # Opacity first, then the colours
        opj(src("rgba-16.png"), out("rgba-16-alpha-first.jp2"))
        patch_header(out("rgba-16-alpha-first.jp2"), replace(b"cdef", cdef([(0, 1, 0), (1, 0, 1), (2, 0, 2), (3, 0, 3)])))
        # Enumerated sYCC (18) over components coded without a colour transform
        opj(src("rgb-16.ppm"), out("rgb-16-sycc.jp2"), "-mct", "0")
        patch_header(out("rgb-16-sycc.jp2"), replace(b"colr", struct.pack(">BBBI", 1, 0, 0, 18)))
        # ICC profile colour (method 2): a 128-byte RGB display profile header with no tags
        icc = bytearray(132)
        icc[0:4] = struct.pack(">I", 132)
        icc[8:12] = bytes([2, 0x10, 0, 0])
        icc[12:16], icc[16:20], icc[20:24], icc[36:40] = b"mntr", b"RGB ", b"XYZ ", b"acsp"
        opj(src("rgb-16.ppm"), out("rgb-16-icc.jp2"))
        patch_header(out("rgb-16-icc.jp2"), replace(b"colr", bytes([2, 0, 0]) + bytes(icc)))

        write_raw(src("gray-16-signed.raw"), 1, 16, signed=True)
        opj(src("gray-16-signed.raw"), out("gray-16-signed.jp2"), "-F", f"{W},{H},1,16,s")
        write_raw(src("rgb-16-420.raw"), 3, 16, subsampling=[(1, 1), (2, 2), (2, 2)])
        opj(src("rgb-16-420.raw"), out("rgb-16-420.jp2"), "-F", f"{W},{H},3,16,u@1x1:2x2:2x2", "-mct", "0")
        # opj_compress marks subsampled raw input as sYCC; mark it sRGB so only the subsampling differs
        patch_header(out("rgb-16-420.jp2"), replace(b"colr", struct.pack(">BBBI", 1, 0, 0, 16)))

        # 10-bit indices into a 1024-entry palette of 16-bit RGB, index i giving value(c, i, 16)
        write_raw(src("index-10.raw"), 1, 10)
        opj(src("index-10.raw"), out("palette-10.jp2"), "-F", f"{W},{H},1,10,u")
        entries = 1024
        pclr = struct.pack(">HB", entries, 3) + bytes([15, 15, 15])
        pclr += b"".join(struct.pack(">HHH", *(value(c, i % 256, 16) for c in range(3))) for i in range(entries))
        cmap = b"".join(struct.pack(">HBB", 0, 1, c) for c in range(3))

        def palette(items):
            items = [(k, struct.pack(">BBBI", 1, 0, 0, 16) if k == b"colr" else b) for k, b in items]
            return items + [(b"pclr", pclr), (b"cmap", cmap)]

        patch_header(out("palette-10.jp2"), palette)


if __name__ == "__main__":
    sys.exit(main())
