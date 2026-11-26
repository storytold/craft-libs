"""Model of the DCT coefficient coding (hypothesis under test) and a differential tester against ffmpeg."""
import sys, os, random
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
OPT = dict(insig_front=os.environ.get('IF', '0') == '1')
def decode_coefs(bits, v, opt=OPT):
    """Returns ({(row, col): signed value}, bits consumed)."""
    pos = [0]
    def get():
        b = bits[pos[0]] if pos[0] < len(bits) else 0; pos[0] += 1; return b
    mag = {}; sign = {}; order = []
    lst = [('A', (0, 2)), ('A', (2, 0)), ('A', (2, 2)), ('S', (0, 1)), ('S', (1, 0)), ('S', (1, 1))]
    child = {(0, 2): (0, 4), (2, 0): (4, 0), (2, 2): (4, 4)}
    for plane in range(v - 1, -1, -1):
        bit = 1 << plane
        def value():
            m = bit
            for k in range(plane): m |= get() << k
            return m
        i = 0
        while i < len(lst):
            mode, (r, c) = lst[i]
            if not get(): i += 1; continue
            if mode in 'AC':
                ins = []
                for dr, dc in ((0, 0), (0, 1), (1, 0), (1, 1)):
                    m = (r + dr, c + dc)
                    if get(): ins.append(('S', m))
                    else: mag[m] = value(); sign[m] = get(); order.append(m)
                if mode == 'A': lst[i] = ('B', child[(r, c)])
                else: del lst[i]
                if opt['insig_front']: lst[i:i] = ins; i += len(ins)
                else:
                    if os.environ.get('REV', '1') == '1': ins.reverse()
                    lst[0:0] = ins; i += len(ins)
            elif mode == 'B':
                lst[i] = ('C', (r, c)); lst += [('C', (r, c + 2)), ('C', (r + 2, c)), ('C', (r + 2, c + 2))]
            else:
                mag[(r, c)] = value(); sign[(r, c)] = get(); order.append((r, c)); del lst[i]
    return {rc: (-m if sign[rc] else m) for rc, m in mag.items()}, pos[0]
if __name__ == '__main__':
    import ac5
    from ac1 import bitsof
    from concurrent.futures import ThreadPoolExecutor
    v = int(sys.argv[1]); n = int(sys.argv[2]); p1 = float(sys.argv[3]); seed = int(sys.argv[4]) if len(sys.argv) > 4 else 1
    MINP = 1
    if v >= 4: ac5.DC = 768; ac5.Q = 1; ac5.QDC = 4 / 3; MINP = 4
    rnd = random.Random(seed)
    def one(k):
        bits = [1 if rnd.random() < p1 else 0 for _ in range(300)]
        return bits
    cases = [one(k) for k in range(n)]
    def run(bits):
        coefs, L = decode_coefs(bits, v)
        bits = bits + [0] * (L - len(bits))              # the model read zeros past the end
        F = ac5.est(bitsof(v, 4) + bits[:L] + bitsof(ac5.Q, 4) + [0] * 40)
        if F is None: return bits, coefs, L, 'ffmpeg error'
        if abs(F[0][0] - ac5.DC * ac5.QDC) > 6: return bits, coefs, L, 'length wrong (DC %d)' % round(F[0][0])
        bad = []
        for r in range(8):
            for c in range(8):
                if (r, c) == (0, 0): continue
                x = F[r][c]; pred = coefs.get((r, c), 0)
                if pred == 0 and abs(x) > 3: bad.append('%d%d:got %+d want 0' % (r, c, round(x)))
                if abs(pred) >= MINP and (abs(x) < 2.5 or (x > 0) != (pred > 0)): bad.append('%d%d:got %+d want sign %+d' % (r, c, round(x), pred))
        return bits, coefs, L, ('ok' if not bad else 'coefs ' + ' '.join(bad[:6])), F
    with ThreadPoolExecutor(8) as ex: res = list(ex.map(run, cases))
    ok = sum(r[3] == 'ok' for r in res)
    print('v=%d: %d/%d ok' % (v, ok, n))
    for r in res:
        if r[3] != 'ok': print('  L=%d nz=%d %s  bits %s' % (r[2], len(r[1]), r[3], ''.join(map(str, r[0][:r[2]]))[:70]))
    import json
    json.dump([(r[0][:r[2]], {'%d%d' % k: x for k, x in r[1].items()}, [[round(x, 2) for x in row] for row in r[4]]) for r in res if r[3] == 'ok'], open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'ctest_v%d.json' % v), 'w'))
