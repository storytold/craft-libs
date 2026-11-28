"""Measure every dequantisation multiplier exactly. For each (kind, q, position): start from the integers near the formula value, probe ffmpeg with coefficient values that split the remaining candidates (adding a helper coefficient whose multiplier is already known when a single coefficient cannot), until one integer remains. Resumable: results accumulate in tables_measured.json."""
import sys, os, json, random, math, threading
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from inter3 import block7
from idct1 import block
import xform
from concurrent.futures import ThreadPoolExecutor
here = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(here, 'tables_measured.json')
W = json.load(open(os.path.join(here, 'weights.json')))
LOCK = threading.Lock()
def load():
    return json.load(open(OUT)) if os.path.exists(OUT) else {}
def save(key, table):
    with LOCK:
        d = load(); d[key] = table; json.dump(d, open(OUT + '.tmp', 'w')); os.replace(OUT + '.tmp', OUT)
def predict(T, m, r, c, dcB, helper):
    B = [[0] * 8 for _ in range(8)]; B[0][0] = dcB; B[r][c] = xform.w32(m * T) >> 11
    for (hr, hc), (hm, hT) in helper.items(): B[hr][hc] = xform.w32(hm * hT) >> 11
    R = xform.idct(B)
    return tuple(R[y][x] & 255 for y in range(8) for x in range(8))
def ask(kind, q, coefs, dc):
    if kind == 'inter':
        g, p = block7(coefs, dc=dc, q=q)
        return None if g is None else tuple((g[y][x] - p[y][x]) & 255 for y in range(8) for x in range(8))
    g = block(coefs, dc=dc, q=q)
    return None if g is None else tuple(g[y][x] for y in range(8) for x in range(8))
def solve(kind, q, r, c, cands, dcT, known, max_probes=24, seed=0):
    num, den = xform.SCALE[q]
    dcmax = 1023 if kind == 'inter' else 2047
    # the model wraps at 32 bits like the decoder, so any coefficient a stream can code (15 planes) is a valid probe
    mmax = 32767
    rnd = random.Random(seed + r * 8 + c + 64 * q)
    helpers = [p for p in known if p != (r, c)]
    alive = list(cands)
    for step in range(max_probes):
        if len(alive) <= 1: break
        best = None
        for trial in range(500):
            m = rnd.choice([-1, 1]) * rnd.randint(1, mmax)
            helper = {}
            if helpers and (step >= 3 or trial >= 250):
                hp = rnd.choice(helpers); hT = known[hp]
                helper = {hp: (rnd.choice([-1, 1]) * rnd.randint(1, max(2, min(300, int(0.2e8 / hT)))), hT)}
            dc = max(1, min(dcmax, round(rnd.randint(300, 900) * (1 if kind == 'inter' else 1.6) * den / num)))
            dcB = xform.w32(dc * dcT) >> 11
            groups = {}
            for T in alive: groups.setdefault(predict(T, m, r, c, dcB, helper), []).append(T)
            score = max(len(g) for g in groups.values())
            if best is None or score < best[0]: best = (score, m, helper, dc, groups)
            if score == 1: break
        if best[0] == len(alive): break
        score, m, helper, dc, groups = best
        coefs = {(r, c): m}; coefs.update({hp: hm for hp, (hm, hT) in helper.items()})
        seen = ask(kind, q, coefs, dc)
        if seen is None: return []
        alive = groups.get(seen, [])
    return alive
def solve_dc(kind, q, cands, known, seed=0):
    """The DC multiplier: probes vary the DC value, with a helper coefficient for phase."""
    dcmax = 1023 if kind == 'inter' else 2047
    rnd = random.Random(seed + q); alive = list(cands); helpers = list(known)
    for step in range(24):
        if len(alive) <= 1: break
        best = None
        for trial in range(800):
            dc = rnd.randint(1, dcmax)
            hp = rnd.choice(helpers); hT = known[hp]
            hm = rnd.choice([-1, 1]) * rnd.randint(1, max(2, min(300, int(0.2e8 / hT))))
            groups = {}
            for T in alive:
                B = [[0] * 8 for _ in range(8)]; B[0][0] = xform.w32(dc * T) >> 11; B[hp[0]][hp[1]] = xform.w32(hm * hT) >> 11
                R = xform.idct(B)
                groups.setdefault(tuple(R[y][x] & 255 for y in range(8) for x in range(8)), []).append(T)
            score = max(len(g) for g in groups.values())
            if best is None or score < best[0]: best = (score, dc, hp, hm, groups)
            if score == 1: break
        if best[0] == len(alive): break
        score, dc, hp, hm, groups = best
        seen = ask(kind, q, {hp: hm}, dc)
        if seen is None: return []
        alive = groups.get(seen, [])
    return alive
def measure(kind, q):
    key = '%s_q%d' % (kind, q)
    table = {tuple(int(x) for x in k.split(',')): v for k, v in load().get(key, {}).items()}
    num, den = xform.SCALE[q]
    dc_guess = round(65536 * num / den)
    cells = [(r, c) for r in range(8) for c in range(8) if (r, c) != (0, 0) and (r, c) not in table]
    def formula(r, c): return W[kind][r][c] / 16 * xform.S[r] * xform.S[c] * 65536 * num / den
    pending = {}; equivalent = []
    for rounds in range(4):
        if not cells: break
        def run(rc):
            r, c = rc; lo = math.floor(formula(r, c)) - 3
            return rc, solve(kind, q, r, c, list(range(lo, lo + 8)), table.get((0, 0), dc_guess), {k: v for k, v in table.items() if k != (0, 0)}, seed=rounds * 1000)
        with ThreadPoolExecutor(8) as ex: rows = list(ex.map(run, cells))
        for rc, alive in rows:
            if len(alive) == 1: table[rc] = alive[0]
            elif len(alive) > 1: pending[rc] = alive
        cells = [rc for rc in cells if rc not in table]
        save(key, {'%d,%d' % k: v for k, v in table.items()})
    for rc in cells:
        if rc in pending:
            # the remaining candidates decode every probe-able coefficient value identically; take the one nearest the formula
            table[rc] = min(pending[rc], key=lambda T: abs(T - formula(*rc))); equivalent.append((rc, pending[rc]))
    cells = [rc for rc in cells if rc not in table]
    save(key, {'%d,%d' % k: v for k, v in table.items()})
    if (0, 0) not in table and len(table) >= 8:
        alive = solve_dc(kind, q, list(range(dc_guess - 2, dc_guess + 3)), {k: v for k, v in table.items() if k != (0, 0)})
        if len(alive) == 1: table[(0, 0)] = alive[0]
        elif dc_guess in alive: table[(0, 0)] = dc_guess; equivalent.append(((0, 0), alive))
        save(key, {'%d,%d' % k: v for k, v in table.items()})
    return len(table), cells, table.get((0, 0)), equivalent
if __name__ == '__main__':
    kind = sys.argv[1]
    for q in ([int(a) for a in sys.argv[2:]] or range(16)):
        n, left, dc, eq = measure(kind, q)
        print('%s q%d: %d of 64 entries pinned, unresolved %s, DC multiplier %s, chosen among equivalents %s' % (kind, q, n, left, dc, eq), flush=True)
