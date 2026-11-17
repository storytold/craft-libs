# craft-xfdf

Scoped XFDF field-value reader extracted from PDFCraft #5937 refresh cb564fd82. Does not read FDF/PDF, parse annotations or apply field values. PDF application, permissions, calculations and password filtering stay caller-owned.

Preserves local-name matching, duplicate field order, dotted names, empty leaf clearing and ignored rich-text-only fields, direct split text concatenation and source UTF-8 replacement policy. XML depth1024, field depth64,2million nodes and64MiB charged expanded names/text. Names and values are charged before copying; temporary parents and emitted name copies both count. XML input/tree memory remains governed by craft-xml defaults, not this output budget alone.

Current strict craft-xml also rejects duplicate raw/expanded attributes and more than1024 active inherited namespace declarations; older app XML accepted those. These deliberate tighter admissions must be disclosed by consumers. No complete XFDF or corpus parity claim.

Source history remains in PDFCraft. Shared move and preallocation budget reconciliation by Dylan M. Taylor, MIT OR Apache-2.0. No assets are imported.
