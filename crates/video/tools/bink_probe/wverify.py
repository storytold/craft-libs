"""Verify every quantiser weight with large single coefficients: exactly one integer weight must reproduce ffmpeg."""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from inter3 import block7
from idct1 import block
import xform
from concurrent.futures import ThreadPoolExecutor
here = os.path.dirname(os.path.abspath(__file__))
WI = json.load(open(os.path.join(here, 'weights.json')))['inter']
MS = (101, -173, 257, 331, -409, 487)
def fit(a):
    kind, r, c = a
    W0 = (xform.W_INTRA if kind == 'intra' else WI)[r][c]
    if kind == 'intra': obs = [(block({(r, c): m}, dc=1024, q=0), None) for m in MS]
    else: obs = [block7({(r, c): m}, dc=800, q=0) for m in MS]
    res = {}
    for w in range(W0 - 3, W0 + 4):
        t = round(w / 16 * xform.S[r] * xform.S[c] * 65536); bad = 0
        for m, (g, p) in zip(MS, obs):
            B = [[0] * 8 for _ in range(8)]; B[0][0] = ((1024 if kind == 'intra' else 800) * 65536) >> 11; B[r][c] = (m * t) >> 11
            R = xform.idct(B)
            if kind == 'intra': bad += sum((R[y][x] & 255) != g[y][x] for y in range(8) for x in range(8))
            else: bad += sum(((p[y][x] + R[y][x]) & 255) != g[y][x] for y in range(8) for x in range(8))
        res[w] = bad
    zero = [w for w, b in res.items() if b == 0]
    return kind, r, c, W0, zero
with ThreadPoolExecutor(8) as ex: rows = list(ex.map(fit, [(k, r, c) for k in ('inter',) for r in range(8) for c in range(8) if (r, c) != (0, 0)]))
out = {'intra': xform.W_INTRA, 'inter': [[16] * 8 for _ in range(8)]}; issues = []
for kind, r, c, w0, zero in rows:
    if len(zero) == 1: out[kind][r][c] = zero[0]
    else: out[kind][r][c] = w0
    if zero != [w0]: issues.append((kind, r, c, 'had', w0, 'exact candidates', zero))
for kind in ('inter',):
    print(kind); [print(' '.join('%3d' % x for x in row)) for row in out[kind]]
print('changed or unresolved:', issues)
json.dump(out, open(os.path.join(here, 'weights.json'), 'w'))
