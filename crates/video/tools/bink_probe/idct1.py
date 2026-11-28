import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ac1 import probe_inline, bitsof
from cenc import encode_coefs
def block(coefs, dc=1024, q=0):
    """Pixels of an intra DCT block with the given AC coefficients, DC and quantiser (values wrap mod 256)."""
    v, bits = encode_coefs(coefs)
    res, msg = probe_inline(bitsof(v, 4) + bits + bitsof(q, 4) + [0] * 40, dc=dc, blocks=((5, 5),))
    return res[0] if res else None
def centred(g, base=128): return [[((p - base + 128) % 256) - 128 for p in r] for r in g]
if __name__ == '__main__':
    for u in range(1, 8):
        g = centred(block({(0, u): 400}))
        print('coef (0,%d)=400 row0: %s  rows equal: %s' % (u, g[0], all(r == g[0] for r in g)))
    for u in (1, 2):
        g = centred(block({(u, 0): 400}))
        print('coef (%d,0)=400 col0: %s  cols equal: %s' % (u, [r[0] for r in g], all(len(set(r)) == 1 for r in g)))
    for m in (1, 2, 3, 4, 8, 16, 100, 200, 400):
        g = centred(block({(0, 1): m})); print('coef (0,1)=%d row0: %s' % (m, g[0]))
