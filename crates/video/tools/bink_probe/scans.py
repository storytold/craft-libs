import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from synth import *
scans = {}
for s in range(16):
    inline = [(s >> i) & 1 for i in range(4)] + [0] * 64
    bits, end = luma([3], colours=list(range(1, 65)) + [200] * 22, runs=[0] * 64, inline=inline)
    Y, msg = decode(bits, end)
    if not Y: print(s, 'error', msg); continue
    blk = block(Y, 0); order = [None] * 64
    for y in range(8):
        for x in range(8): order[blk[y][x] - 1] = y * 8 + x
    assert sorted(order) == list(range(64)), s
    scans[s] = order
    print('%2d %s' % (s, ' '.join('%d' % o for o in order)))
if len(scans) != 16: raise SystemExit('only %d of 16 scans decoded: scans.json is left as it is' % len(scans))
json.dump(scans, open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'scans.json'), 'w'))
# flag polarity and run semantics: 16 runs of 4 (value 3), flags alternate 1,0
for flags in ([1, 0] * 8, [0, 1] * 8):
    inline = [0, 0, 0, 0] + flags
    bits, end = luma([3], colours=list(range(1, 65)) + [200] * 22, runs=[3] * 16, inline=inline)
    Y, msg = decode(bits, end)
    print('flags', flags[:4], msg)
    if Y: show(Y, 0); print('next block', Y[8])
