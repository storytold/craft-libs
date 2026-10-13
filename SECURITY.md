# Security Policy

## Supported releases

Security patches are applied to the active `v0.1.x` release line and `main`.

## Threat model and hostile inputs

`craft-libs` crates parse and process untrusted external data, including files, fonts, color profiles, and configuration TOML. All external and foreign inputs are treated as hostile:
- Memory allocations and iteration counts derived from external inputs must be bounded.
- Arithmetic on lengths, offsets, byte counts, and media ticks must use checked or saturating operations.
- The entire workspace strictly enforces `#![forbid(unsafe_code)]`.
- Panics, unwrap calls, and unhandled lock poisoning are treated as critical defects.

## Reporting a vulnerability

Do not disclose security vulnerabilities, crash vectors, or exploitable inputs in public issues. Submit reports via GitHub Private Vulnerability Reporting or contact the maintainers directly.

A useful report includes the affected crate, version, reproduction fixture or minimal test case, observed failure, and potential impact.
