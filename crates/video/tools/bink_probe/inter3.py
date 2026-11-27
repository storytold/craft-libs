import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from synth2 import *
from cenc import encode_coefs
from ac3 import fdct
from concurrent.futures import ThreadPoolExecutor
def bitsof(v, n): return [(v >> i) & 1 for i in range(n)]
def block7(coefs, dc=800, q=0, pos=(5, 5), xo=(0, 0), yo=(0, 0)):
    """(new block, previous block) for an inter DCT block."""
    v, cb = encode_coefs(coefs)
    tail = bitsof(abs(dc), 10) + ([1 if dc < 0 else 0] if dc else [])
    bits = frame(lambda b: luma_plane(b, {pos: 7}, xoff=[xo], yoff=[yo], pdc_bits=bitsof(1, 10) + tail, inline=bitsof(v, 4) + cb + bitsof(q, 4)), extra_zero=300)
    cur, prev, msg = decode(bits)
    if not cur: return None, None
    return blk(cur[0], *pos), blk(prev[0], *pos)
if __name__ == '__main__':
    q = int(sys.argv[1]); m = int(sys.argv[2])
    g0, p = block7({}, q=0)
    def weight(a):
        r, c = a
        g, p = block7({(r, c): m}, q=q)
        d = [[g[y][x] - g0[y][x] for x in range(8)] for y in range(8)]
        return r, c, fdct(d)[r][c] / m * 16
    with ThreadPoolExecutor(8) as ex: rows = list(ex.map(weight, [(r, c) for r in range(8) for c in range(8) if (r, c) != (0, 0)]))
    M = [[0.0] * 8 for _ in range(8)]
    for r, c, x in rows: M[r][c] = x
    for r in range(8): print(' '.join('%7.2f' % M[r][c] for c in range(8)))
