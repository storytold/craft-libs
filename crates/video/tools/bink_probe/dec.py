"""Reference model decoder (python), grown step by step and checked against ffmpeg."""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe, model
TREES = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'trees.json')))
DEC = {int(t): {c: int(s) for s, c in v.items()} for t, v in TREES.items()}
RUNS = [4, 8, 12, 32]
def sym(br, tree, order):
    q = ''
    while True:
        q += str(br.get(1))
        if q in DEC[tree]: return order[DEC[tree][q]]
        if len(q) > 8: raise ValueError('bad code')
def read_rle(br, tree, order, n):
    out = []
    while len(out) < n:
        s = sym(br, tree, order)
        if s < 12: out.append(s)
        else: out += [out[-1]] * RUNS[s - 12]
    return out
if __name__ == '__main__':
    f = probe.parse(sys.argv[1]); pk = probe.video_packet(f, int(sys.argv[2]))
    br = model.BR(pk, 32); tr = model.read_trees(br)
    n = br.get(10); flag = br.get(1); print('btype len', n, 'flag', flag, 'data at', br.pos)
    bt = read_rle(br, tr[0][1], tr[0][2], n)
    print('consumed to', br.pos, 'count', len(bt))
    for r in range(0, len(bt), 23): print(''.join('%x' % x for x in bt[r:r + 23]))
    import collections; print(collections.Counter(bt))
    print('next 11 bits as 10-bit len:', model.BR(pk, br.pos).get(10), ' bits:', model.bitstr(pk, br.pos, br.pos + 40))
