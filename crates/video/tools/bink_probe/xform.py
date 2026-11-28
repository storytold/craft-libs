"""Exact dequantisation and inverse DCT as derived (see NOTES.md)."""
import math
W_INTRA = [[16,16,16,19,16,19,22,22],[22,22,26,24,26,22,22,27],[27,27,26,26,26,29,29,29],[27,27,27,26,34,34,34,29],[29,29,27,27,37,34,34,32],[32,29,29,38,37,35,35,34],[35,40,40,40,38,38,48,48],[46,46,58,56,56,69,69,83]]
SCALE = [(1, 1), (4, 3), (5, 3), (2, 1), (8, 3), (7, 2), (4, 1), (5, 1), (6, 1), (8, 1), (12, 1), (17, 1), (22, 1), (28, 1), (34, 1), (44, 1)]
S = [1.0] + [math.cos(k * math.pi / 16) * math.sqrt(2) for k in range(1, 8)]
def table(W, q):
    num, den = SCALE[q]
    T = [[round(W[r][c] / 16 * S[r] * S[c] * 65536 * num / den) for c in range(8)] for r in range(8)]
    T[0][0] = round(65536 * num / den)
    return T
def w32(x):
    """Wraps to 32 bits, signed: every product and sum in the decoder does."""
    return ((x + (1 << 31)) & 0xffffffff) - (1 << 31)
def idct(B):
    """B: dequantised 8x8 (rows of ints). Returns signed residual/pixel values before the wrap to 8 bits."""
    A1, A2, A3, A4 = 2896, 3784, 2217, 5352
    def mul(x, a): return w32(x * a) >> 11
    def p(i):
        return [w32(v) for v in q(i)]
    def q(i):
        t10 = i[0] + i[4]; t11 = i[0] - i[4]; t13 = i[2] + i[6]; t12 = mul(i[2] - i[6], A1) - t13
        t0 = t10 + t13; t3 = t10 - t13; t1 = t11 + t12; t2 = t11 - t12
        z13 = i[5] + i[3]; z10 = i[5] - i[3]; z11 = i[1] + i[7]; z12 = i[1] - i[7]
        t7 = z11 + z13; u11 = mul(z11 - z13, A1); z5 = mul(z10 + z12, A2)
        u10 = mul(z12, A3) - z5; u12 = mul(z10, -A4) + z5; t6 = u12 - t7; t5 = u11 - t6; t4 = u10 + t5
        return [t0 + t7, t1 + t6, t2 + t5, t3 - t4, t3 + t4, t2 - t5, t1 - t6, t0 - t7]
    cols = [p([B[r][c] for r in range(8)]) for c in range(8)]
    return [[w32(v + 0x7F) >> 8 for v in p([cols[c][r] for c in range(8)])] for r in range(8)]
def dequant(dc, coefs, T):
    B = [[0] * 8 for _ in range(8)]; B[0][0] = w32(dc * T[0][0]) >> 11
    for (r, c), x in coefs.items(): B[r][c] = w32(x * T[r][c]) >> 11
    return B
