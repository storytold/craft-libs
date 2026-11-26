"""Model of the residue coding (hypothesis under test) and differential tester against ffmpeg."""
import sys, os, random
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
def decode_residue(bits, variant='ref'):
    """Returns ({(row, col): signed value}, bits consumed)."""
    pos = [0]
    def get():
        b = bits[pos[0]] if pos[0] < len(bits) else 0; pos[0] += 1; return b
    budget = sum(get() << i for i in range(7)) + 1
    v = sum(get() << i for i in range(3))
    mag = {}; sign = {}; order = []
    lst = [('A', (0, 2)), ('A', (2, 0)), ('A', (2, 2)), ('C', (0, 0))]
    child = {(0, 2): (0, 4), (2, 0): (4, 0), (2, 2): (4, 4)}
    class Done(Exception): pass
    def newval(m, plane):
        val = 1 << plane
        if variant == 'imm':
            for k in range(plane): val |= get() << k
        mag[m] = val; sign[m] = get(); order.append(m)
        nonlocal_budget[0] -= 1
        if nonlocal_budget[0] == 0: raise Done()
    nonlocal_budget = [budget]
    try:
        for plane in range(v, -1, -1):
            if variant.startswith('ref'):
                for m in list(order):
                    b = get()
                    if b: mag[m] += 1 << plane
                    if variant == 'refone' and b or variant == 'refread':
                        nonlocal_budget[0] -= 1
                        if nonlocal_budget[0] == 0: raise Done()
            i = 0
            while i < len(lst):
                mode, (r, c) = lst[i]
                if not get(): i += 1; continue
                if mode in 'AC':
                    ins = []
                    cells = [(r + dr, c + dc) for dr, dc in ((0, 0), (0, 1), (1, 0), (1, 1))]
                    if mode == 'A': lst[i] = ('B', child[(r, c)])
                    else: del lst[i]
                    for m in cells:
                        if get(): ins.append(('S', m))
                        else: newval(m, plane)
                    ins.reverse(); lst[0:0] = ins; i += len(ins)
                elif mode == 'B':
                    lst[i] = ('C', (r, c)); lst += [('C', (r, c + 2)), ('C', (r + 2, c)), ('C', (r + 2, c + 2))]
                else:
                    del lst[i]; newval((r, c), plane)
    except Done: pass
    return {m: (-x if sign[m] else x) for m, x in mag.items()}, pos[0]
if __name__ == '__main__':
    from synth2 import *
    from concurrent.futures import ThreadPoolExecutor
    n = int(sys.argv[1]); p1 = float(sys.argv[2]); seed = int(sys.argv[3]); variant = sys.argv[4]
    maxv = int(sys.argv[5]) if len(sys.argv) > 5 else 7; maxn = int(sys.argv[6]) if len(sys.argv) > 6 else 127
    rnd = random.Random(seed)
    def bitsof(x, k): return [(x >> i) & 1 for i in range(k)]
    cases = [bitsof(rnd.randint(0, maxn), 7) + bitsof(rnd.randint(0, maxv), 3) + [1 if rnd.random() < p1 else 0 for _ in range(400)] for _ in range(n)]
    MARK = bitsof(0, 7) + bitsof(0, 3) + [1]         # second block: one value +1 at (0,2)
    def run(bits):
        res, L = decode_residue(bits, variant)
        bits = bits + [0] * (L - len(bits))              # the model read zeros past the end
        fr = frame(lambda b: luma_plane(b, {(5, 5): 4, (6, 5): 4}, xoff=[(0, 0)] * 2, yoff=[(0, 0)] * 2, inline=bits[:L] + MARK), extra_zero=400)
        cur, prev, msg = decode(fr)
        if not cur: return bits, L, 'ffmpeg error ' + msg
        g = blk(cur[0], 5, 5); p = blk(prev[0], 5, 5); g2 = blk(cur[0], 6, 5); p2 = blk(prev[0], 6, 5)
        bad = ['%d%d:got %+d want %+d' % (y, x, ((g[y][x] - p[y][x] + 128) & 255) - 128, res.get((y, x), 0)) for y in range(8) for x in range(8) if (p[y][x] + res.get((y, x), 0)) & 255 != g[y][x]]
        mark = [(y, x, g2[y][x] - p2[y][x]) for y in range(8) for x in range(8) if g2[y][x] != p2[y][x]]
        if bad: return bits, L, 'values ' + ' '.join(bad[:5])
        if mark != [(0, 2, 1)]: return bits, L, 'length (marker %s)' % mark[:3]
        return bits, L, 'ok'
    with ThreadPoolExecutor(8) as ex: res = list(ex.map(run, cases))
    print('%s: %d/%d ok' % (variant, sum(r[2] == 'ok' for r in res), n))
    for r in [r for r in res if r[2] != 'ok'][:5]:
        b = r[0]; print('  n=%d v=%d L=%d %s  data %s' % (sum(b[i] << i for i in range(7)), sum(b[7 + i] << i for i in range(3)), r[1], r[2][:110], ''.join(map(str, b[10:r[1]]))[:60]))
