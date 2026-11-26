#!/usr/bin/env python3
"""Writes crates/video/src/bink/tables.rs from the measurements in this directory:
trees.json (trees2.py), scans.json (scans.py) and tables_measured.json (texact5.py)."""
import json, os, sys

here = os.path.dirname(os.path.abspath(__file__))
trees = json.load(open(os.path.join(here, 'trees.json')))
scans = json.load(open(os.path.join(here, 'scans.json')))
tables = json.load(open(os.path.join(here, 'tables_measured.json')))

out = ['''//! Constant tables of the Bink video decoder.
//!
//! Every table here was measured from the observable behaviour of a Bink decoder (see
//! `README.md` in this directory); none is copied from another implementation.
//! `crates/video/tools/bink_probe/gen_tables.py` writes this file from those measurements.
''']
out.append('/// The 16 predefined Huffman trees: for each tree, the code of each rank as `(bits, length)`,\n/// where bit 0 of `bits` is the first bit read.\npub(super) const TREES: [[(u8, u8); 16]; 16] = [')
for t in range(16):
    row = []
    for rank in range(16):
        code = trees[str(t)][str(rank)]
        row.append('(0x%02x, %d)' % (sum(int(b) << i for i, b in enumerate(code)), len(code)))
    out.append('    [' + ', '.join(row) + '],')
out.append('];\n')
out.append('/// The 16 scan patterns of a run-fill block: the sample index (row * 8 + column) of each step.\npub(super) const RUN_SCANS: [[u8; 64]; 16] = [')
for s in range(16):
    assert sorted(scans[str(s)]) == list(range(64))
    out.append('    [' + ', '.join(str(v) for v in scans[str(s)]) + '],')
out.append('];\n')
out.append('/// Run lengths of the run symbols 12 to 15 in the block-type bundles.\npub(super) const RLE_RUNS: [usize; 4] = [4, 8, 12, 32];\n')
for kind in ('intra', 'inter'):
    out.append('/// Dequantisation multipliers of %s DCT blocks, by quantiser index and coefficient position\n/// (row * 8 + column; position 0 is the DC), with 16 fractional bits before the `>> 11` of\n/// dequantisation.\npub(super) const %s_QUANT: [[u32; 64]; 16] = [' % (kind, kind.upper()))
    for q in range(16):
        t = tables['%s_q%d' % (kind, q)]
        assert len(t) == 64, (kind, q, len(t))
        out.append('    [' + ', '.join(str(t['%d,%d' % (r, c)]) for r in range(8) for c in range(8)) + '],')
    out.append('];\n')

target = os.path.join(here, '..', '..', 'src', 'bink', 'tables.rs')
if len(sys.argv) > 1 and sys.argv[1] == '--check':
    sys.exit(0 if open(target).read() == '\n'.join(out) else 1)
open(target, 'w').write('\n'.join(out))
print('wrote', os.path.normpath(target))
