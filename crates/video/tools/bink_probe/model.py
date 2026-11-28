"""Working model of the Bink video bitstream, built up from probing."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe
class BR:
    def __init__(s, d, pos=0): s.d = d; s.pos = pos
    def get(s, n):
        v = 0
        for i in range(n):
            v |= ((s.d[s.pos >> 3] >> (s.pos & 7)) & 1) << i; s.pos += 1
        return v
def merge(br, a, b):
    out = []; a = list(a); b = list(b)
    while a and b:
        out.append(b.pop(0) if br.get(1) else a.pop(0))
    return out + a + b
def read_tree(br):
    """Returns (tree number, symbol order)."""
    t = br.get(4)
    if t == 0: return 0, list(range(16))
    if br.get(1):
        n = br.get(3) + 1; syms = [br.get(4) for _ in range(n)]
        return t, syms + [s for s in range(16) if s not in syms]
    depth = br.get(2); runs = [[s] for s in range(16)]
    for i in range(depth + 1):
        runs = [merge(br, runs[j], runs[j + 1]) for j in range(0, len(runs), 2)]
    order = [s for r in runs for s in r]
    return t, order
NAMES = ['btype', 'subtype'] + ['col%d' % i for i in range(16)] + ['collow', 'pattern', 'xoff', 'yoff', 'run']
def read_trees(br, n=23):
    return [(br.pos,) + read_tree(br) for _ in range(n)]
def bitstr(d, a, b): return ''.join(str((d[i >> 3] >> (i & 7)) & 1) for i in range(a, b))
if __name__ == '__main__':
    f = probe.parse(sys.argv[1]); pk = probe.video_packet(f, int(sys.argv[2])); start = int(sys.argv[3])
    br = BR(pk, start)
    for name, (at, t, order) in zip(NAMES, read_trees(br)):
        print('%4d %-8s tree %2d %s' % (at, name, t, ' '.join('%x' % s for s in order)))
    print('trees end', br.pos)
    s = bitstr(pk, br.pos, br.pos + 240)
    for i in range(0, len(s), 80): print('%4d %s' % (br.pos + i, ' '.join(s[i + j:i + j + 8] for j in range(0, 80, 8))))
