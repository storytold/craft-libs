"""Walk each predefined Huffman tree: a prefix q is a complete code iff a marker colour placed right after it decodes in block 1."""
import sys, os, subprocess, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe
from t1 import BW
from t9 import plane, pk, f, w, h
from concurrent.futures import ThreadPoolExecutor
def test(tree, q, marker):
    b = BW(); plane(b, tree, q, ncol=2, tail_zero=0)
    b.put(marker, 4)
    base = len(b.b)
    for words in range((base + 1 + 72 + 31) // 32, (base + 12 + 72 + 31) // 32 + 1):
        bits = (b.b + [0] * (words * 32))[:words * 32]
        body = bytes(sum(bits[i + j] << j for j in range(8)) for i in range(0, len(bits), 8))
        path = os.path.join(probe.TMP, 'w%d_%s_%d_%d.bik' % (tree, ''.join(map(str, q)), marker, words))
        open(path, 'wb').write(probe.build(f['sig'], w, h, [body + pk[60:]]))
        r = subprocess.run(['ffmpeg', '-v', 'error', '-i', path, '-f', 'rawvideo', '-pix_fmt', 'yuv420p', '-'], capture_output=True)
        os.remove(path)
        if len(r.stdout) > 0: return r.stdout[0], r.stdout[8]
    return None, None
def leaf(tree, q):
    """Symbol if q is a complete code (both markers come out right after it), else None."""
    a0, a1 = test(tree, q, 0xA); b0, b1 = test(tree, q, 0x5)
    if a0 is not None and a0 == b0 and a0 >> 4 == 7 and a1 >> 4 == 0xA and b1 >> 4 == 0x5: return a0 & 15
    return None
def walk(tree):
    codes = {}; level = [[0], [1]]
    for depth in range(1, 13):
        if not level: break
        with ThreadPoolExecutor(8) as ex: res = list(ex.map(lambda q: leaf(tree, q), level))
        nxt = []
        for q, s in zip(level, res):
            if s is None: nxt += [q + [0], q + [1]]
            else: codes[''.join(map(str, q))] = s
        level = nxt
    return codes, level
out = {}; incomplete = []
for t in range(16):
    codes, rest = walk(t)
    by = sorted(codes.items(), key=lambda kv: kv[1])
    kraft = sum(2.0 ** -len(c) for c in codes)
    print('tree %2d kraft=%.4f unresolved=%d syms=%d  %s' % (t, kraft, len(rest), len(set(codes.values())), ' '.join('%x:%s' % (s, c) for c, s in by)))
    out[str(t)] = {str(s): c for c, s in by}
    if rest or sorted(codes.values()) != list(range(16)): incomplete.append(t)
if incomplete: raise SystemExit('trees %s are incomplete: trees.json is left as it is' % incomplete)
json.dump(out, open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'trees.json'), 'w'), indent=0)
