"""Model decoder for a plane with block types 3, 6, 8, 9 (and scaled versions later); compares with ffmpeg."""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe, model, xform, cmodel, rmodel
cmodel.OPT['insig_front'] = False
WINTER = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'weights.json')))['inter']
class Tail:
    """Bit list view over the reader from its current position, for the list coders."""
    def __init__(s, br): s.br = br; s.base = br.pos
    def __len__(s): return 1 << 30
    def __getitem__(s, i):
        p = s.base + i
        return (s.br.d[p >> 3] >> (p & 7)) & 1 if (p >> 3) < len(s.br.d) else 0
from dec import sym, DEC, RUNS
SC = {int(k): v for k, v in json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'scans.json'))).items()}
SIGN_NEG = True
class Bundle:
    def __init__(s, name, width, kind): s.name = name; s.width = width; s.kind = kind; s.q = []; s.done = False; s.state = 0; s.last = 0
    def refill(s, br, T):
        if s.q or s.done: return
        n = br.get(s.width)
        if n == 0: s.done = True; return
        k = s.kind
        if k == 'rle':
            if br.get(1): s.q = [br.get(4)] * n; s.last = s.q[-1]; return
            s.last = 0      # a run symbol at the start of a chunk repeats 0
            while len(s.q) < n:
                v = sym(br, *T[s.name])
                if v < 12: s.q.append(v); s.last = v
                else: s.q += [s.last] * RUNS[v - 12]
            assert len(s.q) == n, (s.name, len(s.q), n)
        elif k == 'col':
            def one():
                hi = sym(br, *T['col%d' % s.state]); lo = sym(br, *T['collow']); s.state = hi; return hi << 4 | lo
            if br.get(1): s.q = [one()] * n
            else: s.q = [one() for _ in range(n)]
        elif k == 'pat': s.q = [sym(br, *T['pattern']) | sym(br, *T['pattern']) << 4 for _ in range(n)]
        elif k == 'nib':
            if br.get(1): s.q = [br.get(4)] * n
            else: s.q = [sym(br, *T[s.name]) for _ in range(n)]
        elif k in ('idc', 'pdc'):
            def delta_groups(val):
                out = [val]
                while len(out) < n:
                    cnt = min(8, n - len(out)); b = br.get(4)
                    for _ in range(cnt):
                        if b:
                            d = br.get(b)
                            if d and br.get(1): d = -d
                            val += d
                        out.append(val)
                return out
            if k == 'idc': start = br.get(11)
            else:
                start = br.get(10)
                if start and br.get(1): start = -start
            s.q = delta_groups(start)
        elif k == 'sgn':
            def sg(v):
                if v and br.get(1): return -v if SIGN_NEG else v
                return v if SIGN_NEG or not v else -v
            if br.get(1): s.q = [sg(br.get(4))] * n
            else: s.q = [sg(sym(br, *T[s.name])) for _ in range(n)]
        else: raise ValueError('bundle %s not modelled (len %d)' % (s.name, n))
    def get(s): return s.q.pop(0)
HOLD = {}
def decode_plane(pk, pos, bw, bh, log=None, w1=10, w2=11, prev=None, widths=None):
    br = model.BR(pk, pos); tr = model.read_trees(br); T = {n: (t, o) for n, (at, t, o) in zip(model.NAMES, tr)}
    B = [Bundle('btype', w1, 'rle'), Bundle('subtype', w1, 'rle'), Bundle('col', w2, 'col'), Bundle('pattern', w1, 'pat'),
         Bundle('xoff', w1, 'sgn'), Bundle('yoff', w1, 'sgn'), Bundle('intradc', w1, 'idc'), Bundle('interdc', w1, 'pdc'), Bundle('run', w2, 'nib')]
    if widths:
        for b_, wd in zip(B, widths): b_.width = wd
    bt, sub, col, pat, xo, yo, idc, pdc, run = B
    img = [[0] * (bw * 8) for _ in range(bh * 8)]; HOLD['img'] = img; HOLD['subs'] = []; HOLD['blocks'] = {}
    def dct(dc, W):
        v = br.get(4)
        co, L = cmodel.decode_coefs(Tail(br), v); br.pos += L
        q = br.get(4)
        R = xform.idct(xform.dequant(dc, co, xform.table(W, q)))
        HOLD.setdefault('dct', []).append((br.pos, dc, q, sorted(co.items()), W is xform.W_INTRA))
        return [R[y][x] for y in range(8) for x in range(8)]
    def blk(t):
        px = [0] * 64
        if t == 5: return [v & 255 for v in dct(idc.get(), xform.W_INTRA)]
        if t == 6: px = [col.get()] * 64
        elif t == 9: px = [col.get() for _ in range(64)]
        elif t == 8:
            c = [col.get(), col.get()]
            for y in range(8):
                v = pat.get()
                for x in range(8): px[y * 8 + x] = c[(v >> x) & 1]
        elif t == 3:
            sc = SC[br.get(4)]; i = 0
            while i < 63:
                r = run.get() + 1
                if br.get(1):
                    v = col.get()
                    for _ in range(r): px[sc[i]] = v; i += 1
                else:
                    for _ in range(r): px[sc[i]] = col.get(); i += 1
            if i == 63: px[sc[63]] = col.get()
        else: raise ValueError('block type %d not modelled' % t)
        return px
    for by in range(bh):
        for b in B: b.refill(br, T)
        bx = 0
        while bx < bw:
            t = bt.get(); HOLD['blocks'][(bx, by)] = (t, br.pos)
            if t == 0:
                for y in range(8): img[by * 8 + y][bx * 8:bx * 8 + 8] = prev[by * 8 + y][bx * 8:bx * 8 + 8] if prev else [0] * 8
            elif t in (2, 4, 7):
                dx = xo.get(); dy = yo.get()
                if t == 7: R = dct(pdc.get(), WINTER)
                elif t == 4:
                    res, L = rmodel.decode_residue(Tail(br), 'refone'); br.pos += L
                    R = [res.get((y, x), 0) for y in range(8) for x in range(8)]
                else: R = [0] * 64
                for y in range(8):
                    for x in range(8): img[by * 8 + y][bx * 8 + x] = (prev[by * 8 + y + dy][bx * 8 + x + dx] + R[y * 8 + x]) & 255
            elif t == 1:
                if by & 1 == 0:
                    st_ = sub.get(); HOLD['subs'].append((by, bx, st_)); px = blk(st_)
                    for y in range(min(16, (bh - by) * 8)):
                        for x in range(min(16, (bw - bx) * 8)): img[by * 8 + y][bx * 8 + x] = px[(y // 2) * 8 + x // 2]
                bx += 1
            else:
                px = blk(t)
                for y in range(8): img[by * 8 + y][bx * 8:bx * 8 + 8] = px[y * 8:y * 8 + 8]
            bx += 1
        if log: log(by, br.pos)
    return img, br.pos
RUNMUL = 48
def widths_for(bw):
    L = lambda n: (n + 511).bit_length()
    return [L(bw), L(bw // 2), L(bw * 64), L(bw * 8), L(bw), L(bw), L(bw), L(bw), L(bw * RUNMUL)]
def decode_frame(pk, w, h, prev=None, order='YVU', widths=None):
    """Model decode of a BIKi packet; returns dict of planes (lists of rows, padded to whole blocks)."""
    cw, ch = (w + 1) // 2, (h + 1) // 2; pos = 32; out = {}
    for name in order:
        pw, ph = (w, h) if name == 'Y' else (cw, ch)
        bw, bh = (pw + 7) // 8, (ph + 7) // 8
        out[name], end = decode_plane(pk, pos, bw, bh, prev=prev[name] if prev else None, widths=widths or widths_for(bw))
        pos = (end + 31) // 32 * 32
    return out, pos
if __name__ == '__main__':
    f = probe.parse(sys.argv[1]); n = int(sys.argv[2]); w, h = f['w'], f['h']
    pks = [probe.video_packet(f, i) for i in range(n)]
    ref, _ = probe.decode(probe.build(f['sig'], w, h, pks)); prev = None
    cw, ch = (w + 1) // 2, (h + 1) // 2
    for i in range(n):
        try: fr, end = decode_frame(pks[i], w, h, prev)
        except Exception as e: print('frame', i, 'model stopped:', repr(e)[:100]); break
        R = dict(zip('YUV', probe.planes(ref, w, h, i))); res = []
        for name in 'YUV':
            pw, ph = (w, h) if name == 'Y' else (cw, ch)
            res.append('%s %d/%d' % (name, sum(fr[name][y][x] != R[name][y * pw + x] for y in range(ph) for x in range(pw)), pw * ph))
        print('frame', i, 'mismatches', ' '.join(res), 'end bit', end, 'of', len(pks[i]) * 8)
        prev = fr
