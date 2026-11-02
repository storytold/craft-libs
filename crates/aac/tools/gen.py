import json
out = ['//! AAC Huffman codebooks (ISO/IEC 14496-3 §4.A.1, spectrum codebooks 1–11 and the scalefactor codebook).',
       '//!',
       '//! Each table is indexed by the codebook index defined in the standard and stores `(codeword, length)`.',
       '//! Generated data: derived by black-box probing of an external decoder used purely as a test oracle',
       '//! (every table is a complete prefix code — Kraft sum exactly 1 — covering every index once; see',
       '//! `tests::codebooks_are_complete_prefix_codes`).',
       '',
       '#![allow(clippy::unreadable_literal)]',
       '']
for cb in [0] + list(range(1, 12)):
    d = json.load(open(f'cb{cb}.json'))
    n = len(d)
    arr = [None] * n
    for s, i in d.items():
        arr[i] = (int(s, 2), len(s))
    name = 'SCALEFACTOR' if cb == 0 else f'SPECTRUM_{cb}'
    out.append(f'pub(crate) static {name}: [(u32, u8); {n}] = [')
    line = '   '
    for (c, l) in arr:
        item = f' (0x{c:x}, {l}),'
        if len(line) + len(item) > 100:
            out.append(line)
            line = '   '
        line += item
    out.append(line)
    out.append('];')
    out.append('')
open('huffman_tables.rs', 'w').write('\n'.join(out))
