"""Collect (dc, q, coefs) -> pixel blocks from ffmpeg for fitting the exact dequantisation and transform."""
import sys, os, json, random
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from idct1 import block
from concurrent.futures import ThreadPoolExecutor
rnd = random.Random(int(sys.argv[2])); n = int(sys.argv[1]); out = sys.argv[3]; qfix = int(sys.argv[4]) if len(sys.argv) > 4 else None
if qfix is not None and not 0 <= qfix <= 15: raise SystemExit('the quantiser is a 4-bit field: 0 to 15')
cases = []
for i in range(n):
    kind = i % 4
    dc = rnd.randint(600, 1400); q = rnd.randint(0, 15) if qfix is None else qfix
    if kind == 0: co = {}
    elif kind == 1: co = {(rnd.randint(0, 7), rnd.randint(0, 7)): rnd.choice([-1, 1]) * rnd.randint(1, 40)}
    else: co = {(rnd.randint(0, 7), rnd.randint(0, 7)): rnd.choice([-1, 1]) * rnd.randint(1, 12) for _ in range(rnd.randint(2, 8))}
    co.pop((0, 0), None)
    cases.append((dc, q, co))
def run(c):
    dc, q, co = c
    g = block(co, dc=dc, q=q)
    return {'dc': dc, 'q': q, 'coefs': [[r, cc, x] for (r, cc), x in co.items()], 'px': g}
with ThreadPoolExecutor(8) as ex: rows = [r for r in ex.map(run, cases) if r['px']]
if not rows: raise SystemExit('no probe decoded: %s is not written' % out)
json.dump(rows, open(out, 'w')); print('saved', len(rows), 'rejected', n - len(rows))
