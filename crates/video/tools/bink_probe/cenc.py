"""Encoder for the coefficient coding (inverse of cmodel.decode_coefs), by replaying the decoder's decisions."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
def encode_coefs(coefs):
    """coefs: {(row, col): signed int} without DC. Returns (v, bits)."""
    coefs = {k: x for k, x in coefs.items() if x}
    if not coefs: return 0, []
    top = {k: abs(x).bit_length() - 1 for k, x in coefs.items()}
    v = max(top.values()) + 1
    if v > 15: raise ValueError('a coefficient of more than 15 bits: the plane count is a 4-bit field')
    out = []
    def members(mode, r, c):
        if mode == 'S': return [(r, c)]
        if mode in 'AC': return [(r + dr, c + dc) for dr in (0, 1) for dc in (0, 1)]
        return [(r + dr, c + dc) for dr in range(4) for dc in range(4)]
    child = {(0, 2): (0, 4), (2, 0): (4, 0), (2, 2): (4, 4)}
    lst = [('A', (0, 2)), ('A', (2, 0)), ('A', (2, 2)), ('S', (0, 1)), ('S', (1, 0)), ('S', (1, 1))]
    for plane in range(v - 1, -1, -1):
        def now(cells): return any(top.get(m) == plane for m in cells)
        def value(m):
            x = abs(coefs[m])
            for k in range(plane): out.append((x >> k) & 1)
            out.append(1 if coefs[m] < 0 else 0)
        i = 0
        while i < len(lst):
            mode, (r, c) = lst[i]
            cells = members(mode, r, c) + (members('B', *child[(r, c)]) if mode == 'A' else [])
            if not now(cells): out.append(0); i += 1; continue
            out.append(1)
            if mode in 'AC':
                ins = []
                for m in members(mode, r, c):
                    if top.get(m) == plane: out.append(0); value(m)
                    else: out.append(1); ins.append(('S', m))
                if mode == 'A': lst[i] = ('B', child[(r, c)])
                else: del lst[i]
                ins.reverse(); lst[0:0] = ins; i += len(ins)
            elif mode == 'B':
                lst[i] = ('C', (r, c)); lst += [('C', (r, c + 2)), ('C', (r + 2, c)), ('C', (r + 2, c + 2))]
            else: value((r, c)); del lst[i]
    return v, out
if __name__ == '__main__':
    import random, cmodel
    os.environ['IF'] = '0'; os.environ['REV'] = '1'; cmodel.OPT['insig_front'] = False  # the order the encoder writes
    rnd = random.Random(5); bad = 0
    for t in range(2000):
        n = rnd.randint(0, 20)
        co = {(rnd.randint(0, 7), rnd.randint(0, 7)): rnd.choice([-1, 1]) * rnd.randint(1, 1 << rnd.randint(0, 9)) for _ in range(n)}
        co.pop((0, 0), None)
        v, bits = encode_coefs(co)
        dec, L = cmodel.decode_coefs(bits, v)
        if dec != {k: x for k, x in co.items() if x} or L != len(bits): bad += 1
    print('roundtrip failures:', bad, 'of 2000')
