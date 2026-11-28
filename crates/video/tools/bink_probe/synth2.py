"""Synthetic inter frames on top of real frames 0..2 of the probe sample (180x139, 23x18 luma blocks)."""
import sys, os, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe
from t1 import BW
from synth import f, w, h
PRE = [probe.video_packet(f, i) for i in range(3)]
FS = w * h + 2 * ((w + 1) // 2) * ((h + 1) // 2)
def skip_plane(b, n):
    for _ in range(23): b.put(0, 4)
    b.put(n, 10); b.put(1, 1); b.put(0, 4)
    for wd in (10, 11, 10, 10, 10, 10, 10, 11): b.put(0, wd)
def luma_plane(b, btypes, sub=(), colours=(), patterns=(), xoff=(), yoff=(), runs=(), inline=(), idc_bits=None, pdc_bits=None):
    """btypes: dict {(bx,by): type}, other blocks skip. Signed values as (magnitude, signbit). inline: list of bits for row-major blocks (only first row with inline data supported)."""
    b.put(0, 32)
    for _ in range(23): b.put(0, 4)
    b.put(414, 10); b.put(0, 1)
    for i in range(414): b.put(btypes.get((i % 23, i // 23), 0), 4)
    b.put(len(sub), 10)
    if sub:
        b.put(0, 1)
        for s in sub: b.put(s, 4)
    b.put(len(colours), 11)
    if colours:
        b.put(0, 1)
        for c in colours: b.put(c >> 4, 4); b.put(c & 15, 4)
    b.put(len(patterns), 10)
    for p in patterns: b.put(p & 15, 4); b.put(p >> 4, 4)
    for vals in (xoff, yoff):
        b.put(len(vals), 10)
        if vals:
            b.put(0, 1)
            for mag, sg in vals:
                b.put(mag, 4)
                if mag: b.put(sg, 1)
    for bits in (idc_bits, pdc_bits):
        if bits is None: b.put(0, 10)
        else:
            for x in bits: b.put(x, 1)
    b.put(len(runs), 11)
    if runs:
        b.put(0, 1)
        for r in runs: b.put(r, 4)
    for x in inline: b.put(x, 1)
def frame(build_luma, extra_zero=64):
    b = BW(); build_luma(b)
    for _ in range(extra_zero): b.put(0, 1)
    return b.b
def decode(bits, luma_end_guess=None):
    """bits: luma plane bits (with slack zeros). Tries plane ends at each word boundary near the end; returns (Y,U,V) of the new frame."""
    msgs = ''
    words_total = (len(bits) + 31) // 32
    for words in range(words_total, max(words_total - 8, 1), -1):
        bb = (bits + [0] * 64)[:words * 32]
        c = BW(); c.b = list(bb); skip_plane(c, 108); c.align(32); skip_plane(c, 108)
        out, err = probe.decode(probe.build(f['sig'], w, h, PRE + [c.bytes()]))
        if len(out) >= 4 * FS: return probe.planes(out, w, h, 3), probe.planes(out, w, h, 2), ''
        msgs = ' / '.join(re.findall(r'\[binkvideo @ \w+\] ([^\n]*)', err)) or 'invalid data'
    return None, None, msgs
def blk(P, bx, by): return [[P[(by * 8 + y) * w + bx * 8 + x] for x in range(8)] for y in range(8)]
if __name__ == '__main__':
    for (mx, sx, my, sy) in ((0, 0, 0, 0), (3, 0, 0, 0), (3, 1, 0, 0), (0, 0, 2, 0), (0, 0, 2, 1), (15, 0, 15, 1), (15, 1, 15, 0)):
        bits = frame(lambda b: luma_plane(b, {(5, 5): 2}, xoff=[(mx, sx)], yoff=[(my, sy)]))
        cur, prev, msg = decode(bits)
        if not cur: print((mx, sx, my, sy), 'error', msg); continue
        got = blk(cur[0], 5, 5); found = []
        for dy in range(-16, 17):
            for dx in range(-16, 17):
                if all(got[y][x] == prev[0][(40 + y + dy) * w + 40 + x + dx] for y in range(8) for x in range(8)): found.append((dx, dy))
        same_rest = sum(cur[0][i] != prev[0][i] for i in range(w * h))
        print('x mag %d sign %d, y mag %d sign %d -> matches offsets %s; luma px changed %d' % (mx, sx, my, sy, found[:3], same_rest))
