# Third-Party Notices

## ACadSharp

The DWG reader and writer include code ported and adapted from ACadSharp. The repository
history's initial DWG support commit identifies the source classes explicitly, including
`DwgStreamReaderBase`, `DwgLZ77AC21Decompressor`, `DwgFileHeaderWriterAC18`, and
`DwgObjectWriter`. Their current Rust implementations are in `src/io/dwg`.

Copyright (c) 2021 Albert Domenech

MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## DWG Format References

DWG field layouts and version behavior are based on the Open Design Alliance (ODA) DWG
specification and reverse engineering, including comparison with drawings produced by CAD
applications. LibreDWG is used as a secondary cross-check for selected field names and
layouts. The Reed-Solomon tests cite numeric reference values from LibreDWG's
`src/reedsolomon.c` to validate the Rust implementation; those values are test data, not
translated routines. The repository history and source review found no LibreDWG routines
ported into the implementation. The source comments identify the specific cross-checks.
