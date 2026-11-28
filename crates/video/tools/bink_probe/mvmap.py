import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from synth2 import *
from concurrent.futures import ThreadPoolExecutor
def ok(a):
    bx, by, dx, dy = a
    bits = frame(lambda b: luma_plane(b, {(bx, by): 2}, xoff=[(abs(dx), 1 if dx < 0 else 0)], yoff=[(abs(dy), 1 if dy < 0 else 0)]), extra_zero=300)
    cur, prev, msg = decode(bits)
    return cur is not None
if __name__ == '__main__':
    for dx, dy in ((-9, 15), (8, 14), (0, 15), (0, -15), (15, 0), (-15, 0)):
        cells = [(bx, by, dx, dy) for by in range(18) for bx in range(23)]
        with ThreadPoolExecutor(8) as ex: res = list(ex.map(ok, cells))
        print('mv (%d,%d): rows of blocks, # = accepted' % (dx, dy))
        for by in range(18): print('  ' + ''.join('#' if res[by * 23 + bx] else '.' for bx in range(23)))
