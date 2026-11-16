# craft-http-url

HTTP/HTTPS URL parsing, dot-segment removal, relative redirects, HTTPS downgrade rejection,
and query-redacted display. No dependencies or network operations. Parsing rejects inputs
larger than 16 KiB and redirects are bounded before concatenation.

This is not an SSRF filter, DNS resolver or HTTP client. Local/private hosts are allowed.
Applications keep destination permission, authentication, TLS, request/response limits and
redirect-count policy. Public fields preserve the original API; manually constructed values
must not be treated as validated URLs.

## Attribution

Extracted from LightCraft `crates/fetch/src/http.rs` at
`2dcb258ba40738230a07484a95eb27dc84ed66fe` (Apache-2.0).
Original parser: Daniel Voshart, `7fa7506db46d69d609e85e39b338f33786ed0293`.
HTTPS downgrade protection: MarekSpodBiedry, `4a91a1c218bf3d0ac49cc73ebb60621a21ae55e2`.
URL normalization, scheme, fragment and authority fixes: NetterSilver, including
`edc369565eef92161b56b8f1e39ef53629c05489`,
`78f0e6e650444886069f4d4ae4e6593f221b98d4`,
`cee93496587d31c514aaeced3e6b8027cfdb41a0`,
`dee5f7910ad8c24cf28bab528f8d007550af2b23`,
`c3a5d9b7f1f837dc14c01942e3d0293ecbf36345` and
`13849307a9343504339d68b086f1a32cdcbbb457`.
Source-author commits are retained separately in this extraction.
