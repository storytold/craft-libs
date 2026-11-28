"""Coefficient-data explorer: find where the quantiser nibble sits after a data prefix, and show the amplified coefficients."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ac1 import probe_inline, bitsof
from ac3 import fdct
from concurrent.futures import ThreadPoolExecutor
DC = 64; Q = 6; QDC = 4
def est(inline):
    res, msg = probe_inline(inline, dc=DC, blocks=((5, 5),))
    if res is None: return None
    g = res[0]; base = (DC * QDC) // 8
    g = [[((p - base + 128) % 256) - 128 + base for p in r] for r in g]
    return fdct(g)
def analyze(v, P, maxo=48):
    """Returns (zero bits consumed after P before the quantiser, {(row,col): amplified value})."""
    def one(o):
        bits = P + [0] * o + bitsof(Q, 4) + [0] * 40
        F = est(bitsof(v, 4) + bits)
        return o, F
    with ThreadPoolExecutor(8) as ex: rows = list(ex.map(one, range(maxo)))
    for o, F in rows:
        if F is not None and abs(F[0][0] - DC * QDC) < 6:
            return o, {(r, c): round(F[r][c]) for r in range(8) for c in range(8) if (r, c) != (0, 0) and abs(F[r][c]) > 2.5}
    return None, {}
def fmt(c): return ' '.join('%d%d:%+d' % (k[0], k[1], x) for k, x in sorted(c.items())) or '-'
if __name__ == '__main__':
    v = int(sys.argv[1])
    for p in sys.argv[2:]:
        P = [int(c) for c in p] if p != '-' else []
        o, c = analyze(v, P)
        print('v=%d prefix %-18s total data bits: %s  coefs: %s' % (v, p, None if o is None else o + len(P), fmt(c)))
