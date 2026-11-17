# craft-shx

Compiles SHP shape source into SHX shapes, big-font and Unicode containers.
Extracted from CADCraft by Francesco BV (cadcraft52c1e6de); no font assets are bundled.
App font discovery, text layout and glyph interpretation are not part of this crate.
The original 8 MiB source and u16 record bounds remain; malformed double signs,
excess escape ranges and oversized Unicode font records are refused. Duplicate
shape detection is linear rather than a repeated whole-list scan.
