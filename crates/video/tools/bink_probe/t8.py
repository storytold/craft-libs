import sys, os, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe
from t1 import BW
f = probe.parse(probe.sample_path()); w, h = f['w'], f['h']; pk = probe.video_packet(f, 0)
ref, ref_err = probe.decode(probe.build(f['sig'], w, h, [pk]))
if len(ref) != w * h + 2 * ((w + 1) // 2) * ((h + 1) // 2): raise SystemExit('the reference frame did not decode: ' + ref_err.strip()[:300])
END = 461
def run(name, mk, quiet=False):
    b = BW(); mk(b); b.align(32)
    out, err = probe.decode(probe.build(f['sig'], w, h, [b.bytes() + pk[60:]]))
    msgs = re.findall(r'\[binkvideo @ \w+\] ([^\n]*)', err)
    if out == ref: r = 'identical'
    elif len(out) == len(ref):
        Y = probe.planes(out, w, h)[0]; R = probe.planes(ref, w, h)[0]
        d = [i for i in range(len(Y)) if Y[i] != R[i]]
        r = 'differs: %d luma px, first block %s, values %s' % (len(d), (d[0] % w // 8, d[0] // w // 8) if d else 'chroma', sorted(set(Y[i] for i in d))[:8])
    else: r = 'error: ' + (msgs[0] if msgs else 'invalid data')
    if not quiet: print('%-44s %s' % (name, r))
    return r, out
if __name__ == '__main__':
    def mk(b): b.copy(pk, 0, 351); b.put(1, 1); b.put(6, 4); b.copy(pk, 373, END)
    run('subtype memset 6 (tail to true end)', mk)
    def mk(b): b.copy(pk, 0, 168); b.put(414, 10); b.put(1, 1); b.put(6, 4); b.put(0, 10); b.put(414, 11); b.copy(pk, 384, 451)
    run('all 414 blocks plain fill, memset colour', mk)
    def mk(b): b.copy(pk, 0, 168); b.put(414, 10); b.put(1, 1); b.put(6, 4); b.put(0, 10); b.put(414, 11); b.put(1, 1); b.put(7, 4); b.put(0, 1); b.copy(pk, 390, 451)
    run('same with colour high nibble 7', mk)
