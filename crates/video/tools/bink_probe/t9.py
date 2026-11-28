import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from t8 import *
def plane(b, tree, code_bits, ncol=1, hi=7, tail_zero=80):
    """All-fill luma plane; colour chunk of `ncol` colours, the first coded as hi + code_bits with collow tree `tree` (identity order)."""
    b.copy(pk, 0, 32)
    b.put(0, 4); b.put(0, 4)                 # btype, subtype trees raw
    for _ in range(16): b.put(0, 4)          # colour high trees raw
    if tree == 0: b.put(0, 4)
    else: b.put(tree, 4); b.put(1, 1); b.put(0, 3); b.put(0, 4)   # list [0] -> identity order
    for _ in range(4): b.put(0, 4)           # pattern, xoff, yoff, run trees raw
    b.put(414, 10); b.put(1, 1); b.put(6, 4) # btype: memset fill
    b.put(0, 10)                             # subtype: none
    b.put(ncol, 11); b.put(0, 1); b.put(hi, 4)
    for x in code_bits: b.put(x, 1)
    for _ in range(tail_zero): b.put(0, 1)
if __name__ == '__main__':
    for tree, code in ((1, [0]), (1, [1, 0, 0, 0, 0, 0, 0, 0]), (0, [1, 0, 1, 0]), (5, [0, 0, 0, 0, 0, 0, 0, 0])):
        for tz in (60, 80, 100):
            r, out = run('tree %d code %s tail %d' % (tree, ''.join(map(str, code)), tz), lambda b: plane(b, tree, code, tail_zero=tz), quiet=True)
            Y = probe.planes(out, w, h)[0] if len(out) == len(ref) else None
            print(tree, code, tz, r[:40], 'block0 px', Y[0] if Y else None, 'block1 px', Y[8] if Y else None, 'row1', Y[8 * w] if Y else None)
