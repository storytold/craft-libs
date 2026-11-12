# craft-codepage

Legacy 8-bit code pages, one table each, for every reader of old text in the Craft libraries:

- the single-byte Windows code pages (874, 1250–1258) and Mac OS Roman (10000), byte by byte
  (`windows_byte`, `cp1252`);
- the DOS (IBM PC, OEM) code pages 437, 720, 737, 775, 850, 852, 855, 857, 860, 861, 863, 864,
  865, 866 and 869 (`dos_upper`, the `dos` tables; `oem_byte` with the glyphs DOS shows for
  control codes);
- document text in any of those (`code_page_byte`), Windows' symbol code page, the RTF/Word font
  charsets (`charset_code_page`), and the names of the East Asian multibyte code pages for
  `encoding_rs` (`multibyte_encoding_name`).

Everything is keyed by Windows code page number. Readers that name code pages another way map
their names onto these: craft-dxf (`$DWGCODEPAGE` names such as `DOS850`), craft-mime (charset
labels such as `IBM850`), craft-rtf (`\ansicpg`, `\fcharset`).

```rust
use craft_codepage::{code_page_byte, dos_upper, windows_byte};

assert_eq!(windows_byte(1251, 0xC4), 'Д');
assert_eq!(code_page_byte(850, 0x9B), 'ø');
assert_eq!(dos_upper(437).map(|t| t[0x1B]), Some('¢'));
```

The Windows and Mac tables match the WHATWG Encoding Standard's indexes, except that a byte a
Windows code page other than 1252 leaves unassigned is U+FFFD here where the standard gives the
C1 control of the same number. A test checks that against `encoding_rs`.

No dependencies.
