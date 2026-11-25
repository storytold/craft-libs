import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from synth2 import *
def bitsof(v, n): return [(v >> i) & 1 for i in range(n)]
def probe_inline(inline, dc=800, blocks=((5, 5), (6, 5))):
    bits = frame(lambda b: luma_plane(b, {p: 5 for p in blocks}, idc_bits=bitsof(len(blocks), 10) + bitsof(dc, 11) + ([0] * 4 if len(blocks) > 1 else []), inline=inline), extra_zero=400)
    cur, prev, msg = decode(bits)
    if not cur: return None, msg
    return [blk(cur[0], *p) for p in blocks], ''
def desc(g):
    vals = sorted(set(v for r in g for v in r))
    if len(vals) == 1: return 'flat %d' % vals[0]
    return 'row0 %s col0 %s' % (g[0], [r[0] for r in g])
if __name__ == '__main__':
    for k in range(0, 36):
        res, msg = probe_inline([1 if j == k else 0 for j in range(k + 1)])
        print('bit %2d: %s' % (k, msg if res is None else ' | '.join(desc(g) for g in res)))
