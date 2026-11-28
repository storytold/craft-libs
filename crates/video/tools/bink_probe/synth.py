"""Synthetic luma planes for a 180x139 BIKi frame (23x18 blocks), chroma taken from the black first frame of the probe sample."""
import sys, os, subprocess, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe
from t1 import BW
SAMPLE = probe.sample_path()
f = probe.parse(SAMPLE); w, h = f['w'], f['h']; pk0 = probe.video_packet(f, 0)
def rle_fill_rest(first):
    """btype symbols (raw tree): `first` list of types, then fill (6) for the rest of the 414 blocks."""
    n = 414 - len(first) - 1; out = list(first) + [6]
    for sym, ln in ((15, 32), (14, 12), (13, 8), (12, 4)):
        while n >= ln: out.append(sym); n -= ln
    out += [6] * n
    return out
def luma(btypes, colours=(), runs=(), patterns=(), inline=(), sub=(), tail=22):
    b = BW(); b.put(0, 32)
    for _ in range(23): b.put(0, 4)                      # all trees raw
    b.put(414, 10); b.put(0, 1)
    for s in rle_fill_rest(btypes): b.put(s, 4)
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
    b.put(0, 10); b.put(0, 10); b.put(0, 10); b.put(0, 10)  # xoff, yoff, intra DC, inter DC
    b.put(len(runs), 11)
    if runs:
        b.put(0, 1)
        for r in runs: b.put(r, 4)
    for x in inline: b.put(x, 1)
    return b.b, len(b.b) + tail
def decode(bits, end, slack=1):
    """Try the word counts around the expected plane end; returns (luma bytes or None, messages)."""
    msgs = ''
    for words in sorted(set((end + d + 31) // 32 for d in range(-slack * 16, slack * 16 + 1, 8)), key=lambda x: abs(x - (end + 31) // 32)):
        bb = (bits + [0] * (words * 32))[:words * 32]
        body = bytes(sum(bb[i + j] << j for j in range(8)) for i in range(0, len(bb), 8))
        out, err = probe.decode(probe.build(f['sig'], w, h, [body + pk0[60:]]))
        if len(out) > 0: return probe.planes(out, w, h)[0], ''
        msgs = ' / '.join(re.findall(r'\[binkvideo @ \w+\] ([^\n]*)', err)) or 'invalid data'
    return None, msgs
def block(Y, bx, by=0): return [[Y[(by * 8 + y) * w + bx * 8 + x] for x in range(8)] for y in range(8)]
def show(Y, bx, by=0): print('\n'.join(' '.join('%3d' % v for v in row) for row in block(Y, bx, by)))
if __name__ == '__main__':
    # run block: scan index s, 64 runs of length 1, each literal, colours 1..64
    for pol in (0, 1):
        inline = [0, 0, 0, 0]
        for i in range(64): inline.append(pol)
        bits, end = luma([3], colours=list(range(1, 65)) + [200] * 22, runs=[0] * 64, inline=inline)
        Y, msg = decode(bits, end)
        print('literal flag =', pol, msg)
        if Y: show(Y, 0); print('block 1 px', Y[8], 'block 22 px', Y[22 * 8])
