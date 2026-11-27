"""Random inter DCT blocks (motion, all quantisers) against the model."""
import sys, os, json, random
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from synth2 import *
from cenc import encode_coefs
import xform
from concurrent.futures import ThreadPoolExecutor
def bitsof(v, n): return [(v >> i) & 1 for i in range(n)]
WI = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'weights.json')))['inter']
rnd = random.Random(int(sys.argv[2])); n = int(sys.argv[1])
cases = []
for i in range(n):
    co = {(rnd.randint(0, 7), rnd.randint(0, 7)): rnd.choice([-1, 1]) * rnd.randint(1, 30) for _ in range(rnd.randint(0, 7))}
    co.pop((0, 0), None)
    cases.append((rnd.randint(-900, 900), rnd.randint(0, 15), co, rnd.randint(-15, 15), rnd.randint(-15, 15), (rnd.randint(3, 19), rnd.randint(3, 13))))
def run(case):
    dc, q, co, dx, dy, pos = case
    v, cb = encode_coefs(co)
    tail = bitsof(abs(dc), 10) + ([1 if dc < 0 else 0] if dc else [])
    bits = frame(lambda b: luma_plane(b, {pos: 7}, xoff=[(abs(dx), 1 if dx < 0 else 0)], yoff=[(abs(dy), 1 if dy < 0 else 0)], pdc_bits=bitsof(1, 10) + tail, inline=bitsof(v, 4) + cb + bitsof(q, 4)), extra_zero=300)
    cur, prev, msg = decode(bits)
    if not cur: return case, None
    g = blk(cur[0], *pos)
    R = xform.idct(xform.dequant(dc, co, xform.table(WI, q)))
    bad = 0
    for y in range(8):
        for x in range(8):
            pred = prev[0][(pos[1] * 8 + y + dy) * w + pos[0] * 8 + x + dx]
            bad += ((pred + R[y][x]) & 255) != g[y][x]
    return case, bad
with ThreadPoolExecutor(8) as ex: res = list(ex.map(run, cases))
err = [c for c, b in res if b is None]; bad = [(c, b) for c, b in res if b]
print('%d blocks: %d ffmpeg errors, %d blocks with mismatches' % (n, len(err), len(bad)))
for c, b in bad[:5]: print('  mismatch', b, 'dc', c[0], 'q', c[1], 'coefs', len(c[2]), 'mv', c[3], c[4])
for c in err[:3]: print('  error case', c)
