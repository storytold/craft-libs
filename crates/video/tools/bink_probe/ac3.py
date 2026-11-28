import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ac1 import probe_inline, bitsof
C = [[(math.sqrt(1 / 8) if u == 0 else math.sqrt(2 / 8)) * math.cos((2 * x + 1) * u * math.pi / 16) for x in range(8)] for u in range(8)]
def fdct(g):
    t = [[sum(C[u][x] * g[y][x] for x in range(8)) for u in range(8)] for y in range(8)]
    return [[sum(C[v][y] * t[y][u] for y in range(8)) for u in range(8)] for v in range(8)]
def coefs(inline, dc=800):
    """Estimated coefficients {(v,u): value} of block (5,5), DC removed; pixel wrap undone around the DC level."""
    res, msg = probe_inline(inline, dc=dc, blocks=((5, 5),))
    if res is None: return None, msg
    g = res[0]; base = dc // 8
    g = [[((p - base + 128) % 256) - 128 + base for p in r] for r in g]
    F = fdct(g); out = {}
    for v in range(8):
        for u in range(8):
            val = F[v][u] - (dc if (u, v) == (0, 0) else 0)
            if abs(val) > 0.6: out[(v, u)] = round(val, 1)
    return out, ''
if __name__ == '__main__':
    v = int(sys.argv[1]); n = int(sys.argv[2])
    print('v=%d zeros:' % v, coefs(bitsof(v, 4) + [0] * 80))
    for k in range(n):
        print('v=%d bit %2d: %s' % (v, k, coefs(bitsof(v, 4) + [1 if j == k else 0 for j in range(80)])))
