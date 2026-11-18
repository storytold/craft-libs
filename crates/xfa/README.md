# craft-xfa

Scoped namespace-aware XML DOM extracted from PDFCraft xfa/xml.rs at 271e479c. This is not a complete XFA implementation: template interpretation, packets, data binding, scripts, layout, fonts and PDF generation remain app-owned.

Preserves source byte ranges, coalesced text, namespace shadowing and document-local node identities. Uses craft-xml strict events with DTD rejection by default, depth 1024 and caller node count. Other event-reader defaults remain in force. Namespace snapshots are copied per element as before, with a 64 MiB cumulative pre-allocation budget for snapshot entry headers, prefix/URI bytes and resolved element/attribute namespace URI copies. Excess returns an error. This intentionally tightens resource admission; it is not a total DOM memory bound or a full XML namespaces conformance claim.

Original source by Dylan M. Taylor (eead4541, 7d44e654), MIT OR Apache-2.0.

Compatibility delta versus the original app XML pin e2dc681a3: current strict craft-xml rejects duplicate raw or expanded attribute names. This deliberately preserves shared security hardening; valid-document DOM behavior is unchanged, but malformed acceptance and errors are not promised byte-identical. Regression tests cover both duplicate forms.

The updated strict reader also bounds aggregate active namespace declarations to 1024 across ancestor elements, not just attributes per tag. The old app path could accept more inherited declarations. The exact boundary and one-over case are tested; this intentional tighter resource admission is retained.
