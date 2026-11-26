import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe
class BW:
    def __init__(s): s.b = []
    def put(s, v, n):
        for i in range(n): s.b.append((v >> i) & 1)
    def copy(s, d, a, b):
        for i in range(a, b): s.b.append((d[i >> 3] >> (i & 7)) & 1)
    def align(s, n=32):
        while len(s.b) % n: s.b.append(0)
    def bytes(s):
        s.align(8); return bytes(sum(s.b[i + j] << j for j in range(8)) for i in range(0, len(s.b), 8))
