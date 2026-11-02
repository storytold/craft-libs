"""Black-box derivation of the AAC Huffman codebooks by probing the ffmpeg decoder (oracle only)."""
import sys, json, math
from concurrent.futures import ThreadPoolExecutor
from probe import frame, silent, run, fit

def unit(sf):
    return 2.0 ** (0.25 * (sf - 100)) / (32768 * 1024)

PARAMS = {  # cb: (dim, signed, lav)
    1: (4, True, 1), 2: (4, True, 1), 3: (4, False, 2), 4: (4, False, 2),
    5: (2, True, 4), 6: (2, True, 4), 7: (2, False, 7), 8: (2, False, 7),
    9: (2, False, 12), 10: (2, False, 12), 11: (2, False, 16),
}

def analyse(y, err, sfs):
    if err or len(y) < 2048:
        return None
    x, res, en = fit(y[:2048])
    if en == 0:
        return [0] * 12 if res == 0 else None
    if res > 1e-6 * en:
        return None
    out = []
    for i, v in enumerate(x):
        u = unit(sfs[i // 4])
        m = abs(v) / u
        q = m ** 0.75
        r = round(q)
        if abs(q - r) > 0.02 * max(1, r):
            return None
        out.append(int(r) * (1 if v >= 0 else -1))
    return out

G = 140

def query_spec(cb, S, k, e):
    dim, signed, lav = PARAMS[cb]
    ncopies = 12 // dim
    bits = ''
    for c in range(ncopies):
        bits += S + ('0' if c % 2 == 0 else '1') * k + '00000' * e
    y, err = run([frame(cb, '000', bits, G=G), silent(), silent()])
    vals = analyse(y, err, [G, G, G])
    if vals is None:
        return None
    vecs = [vals[c * dim:(c + 1) * dim] for c in range(ncopies)]
    v = vecs[0]
    for c in range(ncopies):
        exp = v if (signed or c % 2 == 0) else [-a for a in v]
        if vecs[c] != exp:
            return None
    if signed:
        if any(abs(a) > lav for a in v):
            return None
        mod = 2 * lav + 1
        idx = 0
        for a in v:
            idx = idx * mod + (a + lav)
        return idx
    if any(a < 0 for a in v):
        return None
    if sum(1 for a in v if a != 0) != k:
        return None
    if cb == 11:
        if sum(1 for a in v if a == 16) != e:
            return None
    elif e:
        return None
    if any(a > lav for a in v):
        return None
    mod = lav + 1
    idx = 0
    for a in v:
        idx = idx * mod + a
    return idx

def query_sf(S):
    Gs = 128
    # spectral: cb1 (-1,0,0,0) = '10001' in each band
    y, err = run([frame(1, S + '0' + S, '10001' * 3, G=Gs), silent(), silent()])
    for d in range(-60, 61):
        sfs = [Gs + d, Gs + d, Gs + 2 * d]
        if not all(0 <= s <= 255 for s in sfs):
            continue
        vals = analyse(y, err, sfs)
        if vals == [-1, 0, 0, 0] * 3:
            return d + 60
    return None

def hyps(cb):
    dim, signed, lav = PARAMS[cb]
    if signed:
        return [(0, 0)]
    hs = [(k, 0) for k in range(dim + 1)]
    if cb == 11:
        hs += [(1, 1), (2, 1), (2, 2)]
    return hs

def derive(cb, maxlen=20):
    leaves = {}
    frontier = ['0', '1']
    with ThreadPoolExecutor(14) as ex:
        while frontier:
            def q(S):
                if cb == 0:
                    return S, query_sf(S)
                for (k, e) in hyps(cb):
                    r = query_spec(cb, S, k, e)
                    if r is not None:
                        return S, r
                return S, None
            nxt = []
            for S, r in ex.map(q, frontier):
                if r is not None:
                    leaves[S] = r
                elif len(S) < maxlen:
                    nxt += [S + '0', S + '1']
            print(f'cb{cb}: depth {len(frontier[0])} frontier {len(frontier)} leaves {len(leaves)}', file=sys.stderr)
            frontier = nxt
    return leaves

if __name__ == '__main__':
    cb = int(sys.argv[1])
    leaves = derive(cb)
    kraft = sum(2.0 ** -len(s) for s in leaves)
    idxs = sorted(leaves.values())
    print(f'cb{cb}: {len(leaves)} leaves, kraft {kraft}, unique idx {len(set(idxs))}, range {idxs[0]}..{idxs[-1]}', file=sys.stderr)
    json.dump({s: i for s, i in leaves.items()}, open(f'cb{cb}.json', 'w'))
