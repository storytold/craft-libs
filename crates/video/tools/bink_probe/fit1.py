"""Offline test of candidate integer dequantisation + inverse DCT structures against the ffmpeg dataset."""
import sys, os, json, math, itertools
W = [[16,16,16,19,16,19,22,22],[22,22,26,24,26,22,22,27],[27,27,26,26,26,29,29,29],[27,27,27,26,34,34,34,29],[29,29,27,27,37,34,34,32],[32,29,29,38,37,35,35,34],[35,40,40,40,38,38,48,48],[46,46,58,56,56,69,69,83]]
SC = [1.0] + [math.cos(k * math.pi / 16) * math.sqrt(2) for k in range(1, 8)]
def make(cbits, qbits, qshift, final_bias, final_shift, order, qround, mround):
    A1 = round(1.414213562 * (1 << cbits)); A2 = round(1.847759065 * (1 << cbits)); A3 = round(1.082392200 * (1 << cbits)); A4 = round(2.613125930 * (1 << cbits))
    def mul(x, a): return (x * a + mround * (1 << (cbits - 1))) >> cbits
    def idct1(i):
        t10 = i[0] + i[4]; t11 = i[0] - i[4]; t13 = i[2] + i[6]; t12 = mul(i[2] - i[6], A1) - t13
        t0 = t10 + t13; t3 = t10 - t13; t1 = t11 + t12; t2 = t11 - t12
        z13 = i[5] + i[3]; z10 = i[5] - i[3]; z11 = i[1] + i[7]; z12 = i[1] - i[7]
        t7 = z11 + z13; t11b = mul(z11 - z13, A1); z5 = mul(z10 + z12, A2)
        t10b = mul(z12, A3) - z5; t12b = mul(z10, -A4) + z5
        t6 = t12b - t7; t5 = t11b - t6; t4 = t10b + t5
        return [t0 + t7, t1 + t6, t2 + t5, t3 - t4, t3 + t4, t2 - t5, t1 - t6, t0 - t7]
    QF = [[round(W[r][c] / 16 * SC[r] * SC[c] * (1 << qbits)) for c in range(8)] for r in range(8)]
    QF[0][0] = 1 << qbits
    def decode(dc, coefs):
        B = [[0] * 8 for _ in range(8)]
        B[0][0] = (dc * QF[0][0] + qround * (1 << (qshift - 1))) >> qshift
        for r, c, x in coefs: B[r][c] = (x * QF[r][c] + qround * (1 << (qshift - 1))) >> qshift
        if order == 'cols':
            cols = [idct1([B[r][c] for r in range(8)]) for c in range(8)]
            T = [[cols[c][r] for c in range(8)] for r in range(8)]
            return [[((v + final_bias) >> final_shift) & 255 for v in idct1(T[r])] for r in range(8)]
        rows = [idct1(B[r]) for r in range(8)]
        cols = [idct1([rows[r][c] for r in range(8)]) for c in range(8)]
        return [[((cols[c][r] + final_bias) >> final_shift) & 255 for c in range(8)] for r in range(8)]
    return decode
if __name__ == '__main__':
    # the table in make() is quantiser 0's (its scale is 1), so only its rows can be compared
    rows = [r for r in json.load(open(sys.argv[1])) if r['q'] == 0]
    if not rows: raise SystemExit('no row of quantiser 0: collect them with dataset.py and a fixed quantiser of 0')
    res = []
    for cbits, qbits, order, qround, mround, fb in itertools.product((8, 11, 12, 13), (14, 16), ('cols', 'rows'), (0, 1), (0, 1), (0x7F, 0x80)):
        qshift = qbits - 5
        dec = make(cbits, qbits, qshift, fb, 8, order, qround, mround)
        bad = 0
        for r in rows:
            got = dec(r['dc'], r['coefs'])
            bad += sum(got[y][x] != r['px'][y][x] for y in range(8) for x in range(8))
        res.append((bad, cbits, qbits, order, qround, mround, hex(fb)))
    res.sort()
    for r in res[:8]: print(r)
