#!/usr/bin/env python3
"""Generate straight-line 1D inverse transforms from the butterfly descriptions of the VP9
specification (v0.6 section 8.7.1): inverse DCT (8.7.1.2 / 8.7.1.3, n = 2..5) and inverse ADST
(8.7.1.4 - 8.7.1.8, n = 3, 4). ADST4 and WHT are small enough to be written by hand in
src/vp9/decode/transform.rs.

The generator symbolically executes the ordered steps of the specification on an array T (and the
higher precision array S of the ADST) and emits one `let` per written element, with the cos64 /
sin64 constants folded in.

Usage: python3 vp9_gen_transforms.py > ../src/vp9/decode/transform_gen.rs
"""

COS64 = [16384, 16364, 16305, 16207, 16069, 15893, 15679, 15426, 15137, 14811, 14449, 14053, 13623,
         13160, 12665, 12140, 11585, 11003, 10394, 9760, 9102, 8423, 7723, 7005, 6270, 5520, 4756,
         3981, 3196, 2404, 1606, 804, 0]


def cos64(angle):
    a = angle & 127
    if a <= 32:
        return COS64[a]
    if a <= 64:
        return -COS64[64 - a]
    if a <= 96:
        return -COS64[a - 64]
    return COS64[128 - a]


def sin64(angle):
    return cos64(angle - 32)


def brev(bits, x):
    t = 0
    for i in range(bits):
        t |= ((x >> i) & 1) << (bits - 1 - i)
    return t


class Gen:
    def __init__(self, n):
        self.n0 = 1 << n
        self.T = ["x[%d]" % i for i in range(self.n0)]
        self.S = [None] * self.n0
        self.lines = []
        self.k = 0

    def new(self, expr):
        name = "v%d" % self.k
        self.k += 1
        self.lines.append("    let %s = %s;" % (name, expr))
        return name

    @staticmethod
    def lin(terms):
        # Wrapping arithmetic: conformant streams never overflow (8.7.1.1), damaged ones must not
        # panic in debug builds.
        s = None
        for var, c in terms:
            if c == 0:
                continue
            t = "%s.wrapping_mul(%d)" % (var, c)
            s = t if s is None else "%s.wrapping_add(%s)" % (s, t)
        return s if s is not None else "0"

    def B(self, a, b, angle, flip):
        ta, tb = self.T[a], self.T[b]
        c, s = cos64(angle), sin64(angle)
        x = self.new("r14(%s)" % self.lin([(ta, c), (tb, -s)]))
        y = self.new("r14(%s)" % self.lin([(ta, s), (tb, c)]))
        self.T[a], self.T[b] = (y, x) if flip else (x, y)

    def H(self, a, b, flip):
        if flip:
            a, b = b, a
        x, y = self.T[a], self.T[b]
        self.T[a] = self.new("%s.wrapping_add(%s)" % (x, y))
        self.T[b] = self.new("%s.wrapping_sub(%s)" % (x, y))

    def SB(self, a, b, angle, flip):
        ta, tb = self.T[a], self.T[b]
        c, s = cos64(angle), sin64(angle)
        x = self.new(self.lin([(ta, c), (tb, -s)]))
        y = self.new(self.lin([(ta, s), (tb, c)]))
        self.S[a], self.S[b] = (y, x) if flip else (x, y)

    def SH(self, a, b):
        sa, sb = self.S[a], self.S[b]
        self.T[a] = self.new("r14(%s.wrapping_add(%s))" % (sa, sb))
        self.T[b] = self.new("r14(%s.wrapping_sub(%s))" % (sa, sb))


def idct_steps(g, n):
    n0, n1, n2, n3 = 1 << n, 1 << (n - 1), 1 << (n - 2), (1 << (n - 3)) if n >= 3 else 0
    if n == 2:
        g.B(0, 1, 16, 1)
    else:
        idct_steps(g, n - 1)
    for i in range(n2):
        g.B(n1 + i, n0 - 1 - i, 32 - brev(5, n1 + i), 0)
    if n >= 3:
        for i in range(n3):
            for j in range(2):
                g.H(n1 + 4 * i + 2 * j, n1 + 1 + 4 * i + 2 * j, j)
    if n == 5:
        for i in range(2):
            for j in range(2):
                g.B(n0 - n + 3 - n2 * j - 4 * i, n1 + n - 4 + n2 * j + 4 * i, 28 - 16 * i + 56 * j, 1)
        for i in range(2):
            for j in range(4):
                g.H(n1 + n3 * j + i, n1 + n2 - 5 + n3 * j - i, j & 1)
    if n >= 4:
        for i in range(2 if n == 5 else 1):
            for j in range(2):
                g.B(n0 - n + 2 - i - n2 * j, n1 + n - 3 + i + n2 * j, 24 + 48 * j, 1)
        for i in range(2 * n - 6):
            for j in range(2):
                g.H(n1 + n2 * j + i, n1 + n2 - 1 + n2 * j - i, j & 1)
    if n >= 3:
        for i in range(n3):
            g.B(n0 - n3 - 1 - i, n1 + n3 + i, 16, 1)
    for i in range(n1):
        g.H(i, n0 - 1 - i, 0)


def idct(n):
    g = Gen(n)
    n0 = 1 << n
    g.T = ["x[%d]" % brev(n, i) for i in range(n0)]  # 8.7.1.2 permutation
    idct_steps(g, n)
    return g


def adst_in_perm(g, n):
    n0, n1 = 1 << n, 1 << (n - 1)
    c = list(g.T)
    for i in range(n1):
        g.T[2 * i] = c[n0 - 1 - 2 * i]
        g.T[2 * i + 1] = c[2 * i]


def adst_out_perm(g, n):
    c = list(g.T)
    if n == 4:
        for a in range(2):
            for b in range(2):
                for cc in range(2):
                    for d in range(2):
                        g.T[8 * a + 4 * b + 2 * cc + d] = c[8 * (d ^ cc) + 4 * (cc ^ b) + 2 * (b ^ a) + a]
    else:
        for a in range(2):
            for b in range(2):
                for cc in range(2):
                    g.T[4 * a + 2 * b + cc] = c[4 * (cc ^ b) + 2 * (b ^ a) + a]


def neg(g, i):
    g.T[i] = g.new("%s.wrapping_neg()" % g.T[i])


def adst8():
    g = Gen(3)
    adst_in_perm(g, 3)
    for i in range(4):
        g.SB(2 * i, 1 + 2 * i, 30 - 8 * i, 1)
    for i in range(4):
        g.SH(i, 4 + i)
    for i in range(2):
        g.SB(4 + 3 * i, 5 + i, 24 - 16 * i, 1)
    for i in range(2):
        g.SH(4 + i, 6 + i)
    for i in range(2):
        g.H(i, 2 + i, 0)
    for i in range(2):
        g.B(2 + 4 * i, 3 + 4 * i, 16, 1)
    adst_out_perm(g, 3)
    for i in range(4):
        neg(g, 1 + 2 * i)
    return g


def adst16():
    g = Gen(4)
    adst_in_perm(g, 4)
    for i in range(8):
        g.SB(2 * i, 1 + 2 * i, 31 - 4 * i, 1)
    for i in range(8):
        g.SH(i, 8 + i)
    for i in range(4):
        g.SB(8 + 2 * i, 9 + 2 * i, 28 - 16 * i, 1)
    for i in range(4):
        g.SH(8 + i, 12 + i)
    for i in range(4):
        g.H(i, 4 + i, 0)
    for i in range(2):
        for j in range(2):
            g.SB(4 + 8 * i + 3 * j, 5 + 8 * i + j, 24 - 16 * j, 1)
    for i in range(2):
        for j in range(2):
            g.SH(4 + 8 * j + i, 6 + 8 * j + i)
    for i in range(2):
        for j in range(2):
            g.H(8 * j + i, 2 + 8 * j + i, 0)
    for i in range(2):
        for j in range(2):
            g.B(2 + 4 * j + 8 * i, 3 + 4 * j + 8 * i, 48 + 64 * (i ^ j), 0)
    adst_out_perm(g, 4)
    for i in range(2):
        for j in range(2):
            neg(g, 1 + 12 * j + 2 * i)
    return g


def emit(name, g):
    n0 = g.n0
    print("    #[inline(always)]")
    print("    pub fn %s(x: [$t; %d]) -> [$t; %d] {" % (name, n0, n0))
    for l in g.lines:
        print("    " + l)
    print("        [%s]" % ", ".join(g.T))
    print("    }")
    print()


print("// Generated by tools/gen_transforms.py from the butterfly descriptions of the VP9 specification")
print("// v0.6 section 8.7.1. Do not edit by hand.")
print()
print("macro_rules! transforms_1d {")
print("    ($t:ty) => {")
print("    #[inline(always)]")
print("    fn r14(x: $t) -> $t {")
print("        x.wrapping_add(1 << 13) >> 14")
print("    }")
print()
for n in range(2, 6):
    emit("idct%d" % (1 << n), idct(n))
emit("iadst8", adst8())
emit("iadst16", adst16())
print("    };")
print("}")
print()
print("pub(crate) mod narrow {")
print("    transforms_1d!(i32);")
print("}")
print()
print("pub(crate) mod wide {")
print("    transforms_1d!(i64);")
print("}")
print()
print("/// The same butterflies on 4 / 8 columns at once (column pass of the 2D transforms).")
print("pub(crate) mod cols4 {")
print("    transforms_1d!(crate::transform::Cols<4>);")
print("}")
print()
print("pub(crate) mod cols8 {")
print("    transforms_1d!(crate::transform::Cols<8>);")
print("}")
