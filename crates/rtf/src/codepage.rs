//! The code pages of 8-bit RTF text, from [`craft_codepage`]: Windows-1252 and the other
//! single-byte Windows code pages RTF files name with `\ansicpg` or a font's `\fcharset`, Mac OS
//! Roman (10000, RTF `\mac`), the DOS code pages, and the East Asian multibyte code pages (932,
//! 936, 949, 950), whose bytes decode as runs rather than one at a time.

pub use craft_codepage::*;
