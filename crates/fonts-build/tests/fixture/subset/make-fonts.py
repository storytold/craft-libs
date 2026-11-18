#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["fonttools==4.67.0"]
# ///
"""Write the subsetting fixtures next to this script: Tiny-glyf.ttf (TrueType outlines) and
Tiny-cff.otf (CFF outlines). Both map the same characters, have a `vert` feature that swaps
U+300C for a vertical form, and draw every glyph as a square.
"""

from pathlib import Path

from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools.pens.ttGlyphPen import TTGlyphPen

HERE = Path(__file__).resolve().parent
CMAP = {
    0x20: "space",
    0x41: "A",
    0x3001: "uni3001",
    0x300C: "uni300C",
    0x3042: "uni3042",
    0x4E00: "uni4E00",
    0x4E01: "uni4E01",
    0x4E03: "uni4E03",
    0xFF01: "uniFF01",
}
GLYPHS = [".notdef", *CMAP.values(), "uni300C.vert"]
FEATURES = "languagesystem DFLT dflt;\nfeature vert { sub uni300C by uni300C.vert; } vert;\n"
# 2026-01-01, so the files are the same on every run.
TIMESTAMP = 3_850_000_000


def square(pen, size):
    pen.moveTo((100, 0))
    pen.lineTo((100, size))
    pen.lineTo((100 + size, size))
    pen.lineTo((100 + size, 0))
    pen.closePath()


def build(cff: bool) -> FontBuilder:
    fb = FontBuilder(1000, isTTF=not cff)
    fb.setupGlyphOrder(GLYPHS)
    fb.setupCharacterMap(CMAP)
    if cff:
        charstrings = {}
        for i, name in enumerate(GLYPHS):
            pen = T2CharStringPen(1000, None)
            square(pen, 100 + 10 * i)
            charstrings[name] = pen.getCharString()
        fb.setupCFF("Tiny-Regular", {"FullName": "Tiny Regular"}, charstrings, {})
    else:
        glyphs = {}
        for i, name in enumerate(GLYPHS):
            pen = TTGlyphPen(None)
            square(pen, 100 + 10 * i)
            glyphs[name] = pen.glyph()
        fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({name: (1000, 100) for name in GLYPHS})
    fb.setupHorizontalHeader(ascent=880, descent=-120)
    fb.setupNameTable({"familyName": "Tiny", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=880, sTypoDescender=-120, usWinAscent=880, usWinDescent=120)
    fb.setupPost()
    addOpenTypeFeaturesFromString(fb.font, FEATURES)
    fb.font["head"].created = TIMESTAMP
    fb.font["head"].modified = TIMESTAMP
    fb.font.recalcTimestamp = False
    return fb


def main() -> None:
    build(cff=False).save(HERE / "Tiny-glyf.ttf")
    build(cff=True).save(HERE / "Tiny-cff.otf")


if __name__ == "__main__":
    main()
